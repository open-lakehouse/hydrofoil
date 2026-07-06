//! The UC-ABAC **policy-binding PIP**: sourcing a table's row-filter / column-mask
//! policy bindings from Unity Catalog at resolution time.
//!
//! Structurally parallel to [`UnityGovernedTagProvider`](super::tags::UnityGovernedTagProvider):
//! as each table resolves, its already-inheritance-folded policy set is fetched
//! from mangrove's `/policies` API (`include_inherited=true` — the catalog →
//! schema → table union is done server-side) and mapped from the UC
//! [`PolicyInfo`] wire type into the neutral
//! [`PolicyBinding`](datafusion_policy::binding::PolicyBinding) model the
//! [`AbacPolicyEngine`](datafusion_policy::AbacPolicyEngine) consumes off
//! [`TableFacts::policies`](datafusion_policy_cedar::TableFacts::policies).
//!
//! # Where the parsing lives
//!
//! mangrove stores and returns a policy's `when_condition` and each
//! `match_columns[].condition` **opaque** — it does not parse them. Turning those
//! predicate strings into [`TagCondition`]s is this host's job (see the
//! `binding` module docs in `datafusion-policy`). This module owns the small
//! grammar: conjunctions of `hasTag('k')` / `hasTagValue('k','v')` joined by
//! `AND`. Anything outside it is a **skip + poison**: the binding is dropped and
//! the table is marked poisoned so [`Posture::Enforcing`] fails closed for that
//! table rather than silently under-enforcing (a policy we cannot fully honor
//! must not resolve to "no constraint").
//!
//! # Fail-closed vs best-effort
//!
//! Unlike governed tags (best-effort: a sourcing error records "no tags"),
//! **policy sourcing is fail-closed**: a `/policies` API error is surfaced as an
//! `Err`, and `record_facts` turns that into a poisoned table so an enforcing
//! session refuses to run a query it cannot govern. The convention-free default
//! ([`NoPolicyBindingProvider`]) yields no bindings for offline / deterministic
//! tests, exactly as the convention tag provider does.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use datafusion::error::DataFusionError;
use datafusion_policy::{BindingKind, ColumnMatch, FunctionArg, PolicyBinding, TagCondition};
use unitycatalog_client::UnityCatalogClient;
use unitycatalog_common::models::policies::v1::{
    FunctionRef, MatchColumn, PolicyInfo, PolicyType, function_arg, policy_info,
};
use unitycatalog_common::models::tables::v1::Table;

/// The policy bindings applying to one table, plus whether sourcing them was
/// lossy. A poisoned table has at least one binding we could not fully honor
/// (an unparsable predicate); the enforcement layer must fail closed for it
/// under [`Posture::Enforcing`] rather than run under-constrained.
#[derive(Debug, Clone, Default)]
pub struct TablePolicies {
    /// The bindings we successfully mapped to the neutral model.
    pub bindings: Vec<PolicyBinding>,
    /// True when one or more of the table's policies was dropped because its
    /// predicate could not be parsed — the fail-closed signal.
    pub poisoned: bool,
}

/// Source of a table's UC-ABAC policy bindings — the seam for sourcing the
/// row-filter / column-mask policies the [`AbacPolicyEngine`] consumes. One
/// trait per PIP (per ADR-0007), kept separate from the governed-tag and
/// classification providers.
///
/// Sourcing is **fail-closed**: an error must propagate (so an enforcing session
/// refuses to run), unlike the best-effort tag providers.
#[async_trait::async_trait]
pub trait PolicyBindingProvider: Send + Sync + std::fmt::Debug {
    /// Fetch the already-inheritance-folded bindings applying to `table`. An
    /// error means "could not source policies" and is surfaced to the caller,
    /// which fails closed under enforcement.
    async fn bindings(&self, table: &Table) -> Result<TablePolicies, DataFusionError>;
}

/// The default [`PolicyBindingProvider`]: no bindings, ever. Used offline and in
/// deterministic tests (the analog of the convention tag providers), where
/// policies are seeded directly into the fact sink instead of fetched.
#[derive(Debug, Default)]
pub struct NoPolicyBindingProvider;

#[async_trait::async_trait]
impl PolicyBindingProvider for NoPolicyBindingProvider {
    async fn bindings(&self, _table: &Table) -> Result<TablePolicies, DataFusionError> {
        Ok(TablePolicies::default())
    }
}

/// Production [`PolicyBindingProvider`]: reads bindings from Unity Catalog's
/// `/policies` API (`include_inherited=true`, so mangrove folds the
/// catalog → schema → table hierarchy server-side) and maps each [`PolicyInfo`]
/// into the neutral [`PolicyBinding`].
///
/// A small TTL cache keyed by securable full name amortizes the fetch across the
/// tables of one query (a catalog-/schema-level policy would otherwise be
/// re-fetched for every table it inherits to). Per-query freshness still holds
/// because the fact sink is re-recorded each resolution; the cache only spares
/// repeated network calls within the TTL window.
#[derive(Clone)]
pub struct UnityPolicyBindingProvider {
    client: UnityCatalogClient,
    cache: Arc<DashMap<String, (Instant, Arc<TablePolicies>)>>,
    ttl: Duration,
}

impl std::fmt::Debug for UnityPolicyBindingProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnityPolicyBindingProvider")
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

impl UnityPolicyBindingProvider {
    /// Default cache TTL — long enough to amortize the repeated catalog-/
    /// schema-level fetches within a query, short enough that a policy change is
    /// picked up promptly.
    pub const DEFAULT_TTL: Duration = Duration::from_secs(30);

    /// A provider caching for [`DEFAULT_TTL`](Self::DEFAULT_TTL).
    pub fn new(client: UnityCatalogClient) -> Self {
        Self::with_ttl(client, Self::DEFAULT_TTL)
    }

    /// A provider caching for `ttl`.
    pub fn with_ttl(client: UnityCatalogClient, ttl: Duration) -> Self {
        Self {
            client,
            cache: Arc::new(DashMap::new()),
            ttl,
        }
    }

    /// Fetch + map the bindings for `full_name`, consulting the TTL cache first.
    async fn fetch(&self, full_name: &str) -> Result<Arc<TablePolicies>, DataFusionError> {
        if let Some(entry) = self.cache.get(full_name) {
            let (at, cached) = entry.value();
            if at.elapsed() < self.ttl {
                return Ok(cached.clone());
            }
        }

        let response = self
            .client
            .list_policies("tables", full_name)
            .with_include_inherited(true)
            .await
            // Fail-closed: a policies-API error must not degrade to "no
            // policies" (which would run the query unconstrained).
            .map_err(|e| DataFusionError::External(Box::new(e)))?;

        let mut out = TablePolicies::default();
        for info in response.policies {
            match map_policy(&info) {
                Some(binding) => out.bindings.push(binding),
                None => {
                    tracing::warn!(
                        target: "hydrofoil::policies",
                        table = %full_name,
                        policy = %info.name,
                        "policy binding could not be parsed; skipping and poisoning the table \
                         (Enforcing will fail closed, Permissive logs and proceeds)"
                    );
                    out.poisoned = true;
                }
            }
        }

        let out = Arc::new(out);
        self.cache
            .insert(full_name.to_string(), (Instant::now(), out.clone()));
        Ok(out)
    }
}

#[async_trait::async_trait]
impl PolicyBindingProvider for UnityPolicyBindingProvider {
    async fn bindings(&self, table: &Table) -> Result<TablePolicies, DataFusionError> {
        Ok((*self.fetch(&table.full_name).await?).clone())
    }
}

/// Map a UC [`PolicyInfo`] into the neutral [`PolicyBinding`], returning `None`
/// when any part cannot be honored — an unknown policy type, a missing function
/// reference, or an unparsable `when_condition` / column condition. A `None`
/// poisons the table (fail-closed) rather than dropping a constraint silently.
fn map_policy(info: &PolicyInfo) -> Option<PolicyBinding> {
    let (kind, function) = match (info.policy_type(), &info.function) {
        (PolicyType::RowFilter, Some(policy_info::Function::RowFilter(f))) => {
            (BindingKind::RowFilter, f)
        }
        (PolicyType::ColumnMask, Some(policy_info::Function::ColumnMask(f))) => {
            (BindingKind::ColumnMask, f)
        }
        // Type/function mismatch or unspecified type: cannot enforce.
        _ => return None,
    };

    let when_condition = match &info.when_condition {
        Some(s) => parse_conditions(s)?,
        None => Vec::new(),
    };

    let match_columns = info
        .match_columns
        .iter()
        .map(map_match_column)
        .collect::<Option<Vec<_>>>()?;

    let (name, using_args) = map_function(function);

    Some(PolicyBinding {
        name: info.name.clone(),
        kind,
        to_principals: info.to_principals.clone(),
        except_principals: info.except_principals.clone(),
        when_condition,
        match_columns,
        function: name,
        using_args,
    })
}

/// Map a [`FunctionRef`] to `(function_name, using_args)`.
fn map_function(f: &FunctionRef) -> (String, Vec<FunctionArg>) {
    let args = f
        .using
        .iter()
        .filter_map(|a| match &a.value {
            Some(function_arg::Value::Alias(alias)) => Some(FunctionArg::Alias(alias.clone())),
            Some(function_arg::Value::Constant(c)) => Some(FunctionArg::Constant(c.clone())),
            None => None,
        })
        .collect();
    (f.function_name.clone(), args)
}

/// Map a UC [`MatchColumn`] (opaque `condition` string + alias) to a neutral
/// [`ColumnMatch`], returning `None` if the condition is unparsable — the same
/// `hasTag`/`hasTagValue` grammar as `when_condition`. A `MATCH COLUMNS`
/// predicate must be a single condition (not a conjunction).
fn map_match_column(m: &MatchColumn) -> Option<ColumnMatch> {
    let mut conds = parse_conditions(&m.condition)?;
    // Exactly one tag condition per matched-columns clause.
    if conds.len() != 1 {
        return None;
    }
    Some(ColumnMatch {
        condition: conds.remove(0),
        alias: m.alias.clone(),
    })
}

/// Parse an `AND`-joined conjunction of `hasTag('k')` / `hasTagValue('k','v')`
/// into [`TagCondition`]s. Returns `None` on anything outside that grammar (an
/// unrecognized function, malformed args, a non-`AND` operator) so the caller
/// can fail closed. An empty / whitespace-only string is an empty conjunction
/// (always applies).
fn parse_conditions(s: &str) -> Option<Vec<TagCondition>> {
    let s = s.trim();
    if s.is_empty() {
        return Some(Vec::new());
    }
    // Split on `AND` case-insensitively at the top level. UC ABAC WHEN clauses
    // are flat conjunctions, so a simple split is sufficient — there is no
    // nesting or `OR` in the supported grammar.
    split_top_level_and(s)
        .into_iter()
        .map(|term| parse_tag_predicate(term.trim()))
        .collect()
}

/// Split `s` into its top-level `AND`-separated terms, matching `AND` only as a
/// whole word (case-insensitive) and not inside a quoted literal.
fn split_top_level_and(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut terms = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut in_quote = false;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            in_quote = !in_quote;
        } else if !in_quote
            // A whole-word `AND` (case-insensitive) bounded by non-alphanumeric
            // characters (or the string edges), outside any quoted literal.
            && i + 3 <= bytes.len()
            && s[i..i + 3].eq_ignore_ascii_case("and")
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
            && (i + 3 == bytes.len() || !bytes[i + 3].is_ascii_alphanumeric())
        {
            terms.push(&s[start..i]);
            i += 3;
            start = i;
            continue;
        }
        i += 1;
    }
    terms.push(&s[start..]);
    terms
}

/// Parse a single `hasTag('k')` or `hasTagValue('k','v')` predicate. Returns
/// `None` on any other function name or malformed arguments.
fn parse_tag_predicate(term: &str) -> Option<TagCondition> {
    let term = term.trim();
    let (func, rest) = term.split_once('(')?;
    let args = rest.strip_suffix(')')?;
    match func.trim() {
        "hasTag" => {
            let key = parse_quoted(args.trim())?;
            Some(TagCondition { key, value: None })
        }
        "hasTagValue" => {
            let (k, v) = args.split_once(',')?;
            let key = parse_quoted(k.trim())?;
            let value = parse_quoted(v.trim())?;
            Some(TagCondition {
                key,
                value: Some(value),
            })
        }
        _ => None,
    }
}

/// Unwrap a single-quoted literal `'x'` → `x`. Returns `None` if not quoted or if
/// it contains an embedded quote (the grammar has no escaping).
fn parse_quoted(s: &str) -> Option<String> {
    let inner = s.strip_prefix('\'')?.strip_suffix('\'')?;
    if inner.contains('\'') {
        return None;
    }
    Some(inner.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use unitycatalog_common::models::policies::v1::FunctionArg as UcFunctionArg;

    fn cond(key: &str, value: Option<&str>) -> TagCondition {
        TagCondition {
            key: key.to_string(),
            value: value.map(str::to_string),
        }
    }

    #[test]
    fn parses_has_tag_and_has_tag_value() {
        assert_eq!(
            parse_conditions("hasTag('pii')").unwrap(),
            vec![cond("pii", None)]
        );
        assert_eq!(
            parse_conditions("hasTagValue('pii','ssn')").unwrap(),
            vec![cond("pii", Some("ssn"))]
        );
    }

    #[test]
    fn parses_and_conjunction_case_insensitively() {
        assert_eq!(
            parse_conditions("hasTag('pii') AND hasTagValue('env','prod')").unwrap(),
            vec![cond("pii", None), cond("env", Some("prod"))]
        );
        // Lowercase `and` and extra whitespace are fine.
        assert_eq!(
            parse_conditions("hasTag('a')   and   hasTag('b')").unwrap(),
            vec![cond("a", None), cond("b", None)]
        );
    }

    #[test]
    fn empty_condition_is_empty_conjunction() {
        assert_eq!(parse_conditions("").unwrap(), vec![]);
        assert_eq!(parse_conditions("   ").unwrap(), vec![]);
    }

    #[test]
    fn value_containing_the_word_and_is_not_split() {
        // `AND` inside a quoted literal must not be treated as a separator.
        assert_eq!(
            parse_conditions("hasTagValue('k','brand')").unwrap(),
            vec![cond("k", Some("brand"))]
        );
    }

    #[test]
    fn rejects_unsupported_grammar() {
        // Unknown function.
        assert!(parse_conditions("isMember('g')").is_none());
        // OR is not supported.
        assert!(parse_conditions("hasTag('a') OR hasTag('b')").is_none());
        // Malformed args.
        assert!(parse_conditions("hasTagValue('k')").is_none());
        assert!(parse_conditions("hasTag(pii)").is_none());
        assert!(parse_conditions("hasTag('unterminated)").is_none());
    }

    fn uc_alias(alias: &str) -> UcFunctionArg {
        UcFunctionArg {
            value: Some(function_arg::Value::Alias(alias.to_string())),
        }
    }

    fn column_mask_policy() -> PolicyInfo {
        PolicyInfo {
            name: "mask_ssn".to_string(),
            policy_type: PolicyType::ColumnMask as i32,
            to_principals: vec!["account users".to_string()],
            except_principals: vec!["admins".to_string()],
            when_condition: Some("hasTagValue('classification','regulated')".to_string()),
            match_columns: vec![MatchColumn {
                alias: "col".to_string(),
                condition: "hasTagValue('pii','ssn')".to_string(),
            }],
            function: Some(policy_info::Function::ColumnMask(FunctionRef {
                function_name: "hr.security.mask_ssn".to_string(),
                using: vec![uc_alias("col")],
            })),
            ..Default::default()
        }
    }

    #[test]
    fn maps_a_column_mask_policy() {
        let binding = map_policy(&column_mask_policy()).expect("maps");
        assert_eq!(binding.name, "mask_ssn");
        assert_eq!(binding.kind, BindingKind::ColumnMask);
        assert_eq!(binding.to_principals, vec!["account users".to_string()]);
        assert_eq!(binding.except_principals, vec!["admins".to_string()]);
        assert_eq!(
            binding.when_condition,
            vec![cond("classification", Some("regulated"))]
        );
        assert_eq!(binding.match_columns.len(), 1);
        assert_eq!(binding.match_columns[0].alias, "col");
        assert_eq!(binding.match_columns[0].condition, cond("pii", Some("ssn")));
        assert_eq!(binding.function, "hr.security.mask_ssn");
        assert_eq!(binding.using_args, vec![FunctionArg::Alias("col".into())]);
    }

    #[test]
    fn unparsable_when_condition_drops_the_binding() {
        let mut p = column_mask_policy();
        p.when_condition = Some("isMember('g')".to_string());
        assert!(map_policy(&p).is_none(), "unparsable WHEN => poison");
    }

    #[test]
    fn type_function_mismatch_drops_the_binding() {
        let mut p = column_mask_policy();
        // ColumnMask type but a RowFilter function body.
        p.function = Some(policy_info::Function::RowFilter(FunctionRef {
            function_name: "f".to_string(),
            using: vec![],
        }));
        assert!(map_policy(&p).is_none());
    }

    #[test]
    fn multi_condition_match_columns_is_rejected() {
        let mut p = column_mask_policy();
        p.match_columns[0].condition = "hasTag('a') AND hasTag('b')".to_string();
        // A MATCH COLUMNS clause must be a single tag condition.
        assert!(map_policy(&p).is_none());
    }
}
