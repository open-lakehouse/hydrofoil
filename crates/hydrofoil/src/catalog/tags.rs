//! The resource/catalog classification PIP: deriving a table's tags/taints and
//! its reader/writer ACLs from Unity Catalog metadata.
//!
//! Unity Catalog (in this fork) has **no tags API** — the `Table` model exposes
//! only `owner`, a `properties` string map, a `comment`, and `columns` (each
//! with a `comment`). So v1 derives classification *by convention* from those
//! fields, behind the [`TagProvider`] trait. A future backend can call an
//! external classification service instead — same trait, async already. See
//! `docs/adr/0007-fact-gathering-pips.md`.
//!
//! The neutral [`TableFacts`](datafusion_policy_cedar::TableFacts) these produce cross
//! into the policy layer; this module is the only place a UC `Table` is read for
//! facts, keeping `datafusion-cedar` free of any catalog dependency.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use datafusion::error::DataFusionError;
use unitycatalog_client::UnityCatalogClient;
use unitycatalog_common::models::tables::v1::Table;
use unitycatalog_common::models::tags::v1::ListEntityTagAssignmentsRequest;

// The catalog-fact-sink `SessionConfig` extension now lives in `datafusion-cedar`
// (the crate owns its per-request seam); re-export it so the rest of hydrofoil
// keeps importing it from `crate::catalog`.
pub use datafusion_policy_cedar::CatalogFactSinkExt;

/// gRPC/property convention keys.
mod keys {
    /// Table-level tags: `properties["tags"] = "pii,regulated"`.
    pub const TABLE_TAGS: &[&str] = &["tags", "classification"];
    /// Column-level tags: `properties["tag.<column>"] = "pii,regulated"`.
    pub const COLUMN_TAG_PREFIX: &str = "tag.";
    /// Reader/writer ACLs: `properties["readers"|"writers"] = "grp_a,grp_b"`.
    pub const READERS: &str = "readers";
    pub const WRITERS: &str = "writers";
    /// Governed (key→value) tags, the UC ABAC model consumed by the Cedar
    /// column-mask / row-filter path (distinct from the opaque classification
    /// tags above, which feed taints):
    /// - table-level:  `properties["govtag.<key>"] = "<value>"`
    /// - per-column:   `properties["govtag.<column>.<key>"] = "<value>"`
    pub const GOVERNED_TAG_PREFIX: &str = "govtag.";
}

/// The classification of one table: its table-level tags and per-column tags.
#[derive(Debug, Clone, Default)]
pub struct TableClassification {
    pub table_tags: BTreeSet<String>,
    pub column_tags: std::collections::HashMap<String, BTreeSet<String>>,
}

/// Source of a table's classification tags/taints — the seam for sourcing
/// `pii`/`regulated`/… classifications. v1 reads UC conventions; a future impl
/// can call an external classification service. Receives the already-fetched
/// `&Table` (catalog resolution has it in hand; re-fetching would be wasteful).
#[async_trait::async_trait]
pub trait TagProvider: Send + Sync + std::fmt::Debug {
    async fn classify(&self, table: &Table) -> Result<TableClassification, DataFusionError>;
}

/// v1 [`TagProvider`]: derives tags by convention from UC `Table` metadata.
///
/// - table tags from `properties["tags"]` / `properties["classification"]`,
/// - column tags from `properties["tag.<column>"]`,
/// - column tags from a `[tags: …]` marker in a column's `comment`.
///
/// Tags are comma-split, trimmed, and lower-cased.
#[derive(Debug, Default)]
pub struct ConventionTagProvider;

#[async_trait::async_trait]
impl TagProvider for ConventionTagProvider {
    async fn classify(&self, table: &Table) -> Result<TableClassification, DataFusionError> {
        let mut out = TableClassification::default();

        for key in keys::TABLE_TAGS {
            if let Some(v) = table.properties.get(*key) {
                out.table_tags.extend(split_tags(v));
            }
        }
        for (k, v) in &table.properties {
            if let Some(col) = k.strip_prefix(keys::COLUMN_TAG_PREFIX) {
                out.column_tags
                    .entry(col.to_string())
                    .or_default()
                    .extend(split_tags(v));
            }
        }
        for col in &table.columns {
            if let Some(comment) = &col.comment {
                let tags = parse_comment_tags(comment);
                if !tags.is_empty() {
                    out.column_tags
                        .entry(col.name.clone())
                        .or_default()
                        .extend(tags);
                }
            }
        }
        Ok(out)
    }
}

/// The governed (key→value) tags of one table: table-level and per-column.
///
/// Distinct from [`TableClassification`] (opaque `BTreeSet<String>` sets that
/// feed the session taint ledger): governed tags are key→value because the Cedar
/// policy layer matches them exactly (`resource.getTag("pii") == "ssn"`), the
/// Unity Catalog ABAC model. They fold into `TableFacts.governed_tags` /
/// `governed_column_tags` and drive the column-mask / row-filter path.
#[derive(Debug, Clone, Default)]
pub struct GovernedTags {
    pub table_tags: BTreeMap<String, String>,
    pub column_tags: HashMap<String, BTreeMap<String, String>>,
}

/// Source of a table's governed (key→value) tags — the seam for sourcing the UC
/// ABAC governed tags the Cedar mask/filter path consumes. v1 reads UC
/// conventions ([`ConventionGovernedTagProvider`]); the production impl
/// ([`UnityGovernedTagProvider`]) calls the UC entity-tag-assignments API. One
/// trait per PIP (per ADR-0007), kept separate from [`TagProvider`] (opaque
/// sets, taint path).
#[async_trait::async_trait]
pub trait GovernedTagProvider: Send + Sync + std::fmt::Debug {
    /// Best-effort: an error means "no governed tags" and must not block
    /// resolution. The mask decision stays fail-closed at the Cedar layer.
    async fn governed_tags(&self, table: &Table) -> Result<GovernedTags, DataFusionError>;
}

/// v1 [`GovernedTagProvider`]: derives governed tags by convention from UC
/// `Table` properties (synchronous, zero network):
///
/// - table-level `properties["govtag.<key>"] = "<value>"` → `table_tags[key]`,
/// - per-column `properties["govtag.<column>.<key>"] = "<value>"` →
///   `column_tags[column][key]`.
///
/// Keys and values are preserved verbatim (no lower-casing or splitting): the
/// Cedar layer matches governed tags exactly, unlike the opaque classification
/// tags in [`ConventionTagProvider`].
#[derive(Debug, Default)]
pub struct ConventionGovernedTagProvider;

#[async_trait::async_trait]
impl GovernedTagProvider for ConventionGovernedTagProvider {
    async fn governed_tags(&self, table: &Table) -> Result<GovernedTags, DataFusionError> {
        let mut out = GovernedTags::default();
        for (k, v) in &table.properties {
            let Some(rest) = k.strip_prefix(keys::GOVERNED_TAG_PREFIX) else {
                continue;
            };
            // `govtag.<column>.<key>` is a per-column tag; `govtag.<key>` (no dot)
            // is a table-level tag. Split on the first dot.
            match rest.split_once('.') {
                Some((col, key)) if !col.is_empty() && !key.is_empty() => {
                    out.column_tags
                        .entry(col.to_string())
                        .or_default()
                        .insert(key.to_string(), v.clone());
                }
                _ => {
                    out.table_tags.insert(rest.to_string(), v.clone());
                }
            }
        }
        Ok(out)
    }
}

/// Production [`GovernedTagProvider`]: reads governed tags from the Unity Catalog
/// **entity-tag-assignments** API (the UC ABAC governed-tag source), one call for
/// the table (`entity_type = "tables"`) and one per column
/// (`entity_type = "columns"`, `entity_name = "<table.full_name>.<column>"`).
///
/// Sourcing is best-effort: a per-entity API error is logged and folded as "no
/// tags for that entity" rather than propagated, so a catalog hiccup never blocks
/// table resolution. The mask decision itself stays fail-closed at the Cedar
/// layer. Assignments whose `tag_value` is `None` are skipped (a governed tag
/// with no value can't satisfy `getTag(k) == v`).
#[derive(Clone)]
pub struct UnityGovernedTagProvider {
    client: UnityCatalogClient,
}

impl std::fmt::Debug for UnityGovernedTagProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnityGovernedTagProvider")
            .finish_non_exhaustive()
    }
}

impl UnityGovernedTagProvider {
    pub fn new(client: UnityCatalogClient) -> Self {
        Self { client }
    }

    /// List the governed-tag assignments for one entity, folding
    /// `(tag_key, tag_value)` pairs into `out` (skipping valueless assignments).
    /// An API error is logged and treated as "no assignments".
    async fn fold_entity(
        &self,
        entity_type: &str,
        entity_name: &str,
        out: &mut BTreeMap<String, String>,
    ) {
        let request = ListEntityTagAssignmentsRequest {
            entity_type: entity_type.to_string(),
            entity_name: entity_name.to_string(),
            max_results: None,
            page_token: None,
        };
        match self
            .client
            .entity_tag_assignments_client()
            .list_entity_tag_assignments(&request)
            .await
        {
            Ok(response) => {
                for assignment in response.tag_assignments {
                    if let Some(value) = assignment.tag_value {
                        out.insert(assignment.tag_key, value);
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e, entity_type, entity_name,
                    "listing governed-tag assignments failed; treating as none"
                );
            }
        }
    }
}

#[async_trait::async_trait]
impl GovernedTagProvider for UnityGovernedTagProvider {
    async fn governed_tags(&self, table: &Table) -> Result<GovernedTags, DataFusionError> {
        let mut out = GovernedTags::default();

        self.fold_entity("tables", &table.full_name, &mut out.table_tags)
            .await;

        for column in &table.columns {
            let mut column_tags = BTreeMap::new();
            let entity_name = format!("{}.{}", table.full_name, column.name);
            self.fold_entity("columns", &entity_name, &mut column_tags)
                .await;
            if !column_tags.is_empty() {
                out.column_tags.insert(column.name.clone(), column_tags);
            }
        }

        Ok(out)
    }
}

/// Owner + reader/writer ACLs derived from UC `Table` metadata, kept beside the
/// other `Table`-reading conventions. Owner is `Table.owner`; readers/writers
/// come from `properties["readers"|"writers"]`.
pub fn table_acl_facts(table: &Table) -> (Option<String>, BTreeSet<String>, BTreeSet<String>) {
    let readers = table
        .properties
        .get(keys::READERS)
        .map(|v| split_tags(v).collect())
        .unwrap_or_default();
    let writers = table
        .properties
        .get(keys::WRITERS)
        .map(|v| split_tags(v).collect())
        .unwrap_or_default();
    (table.owner.clone(), readers, writers)
}

/// Split a comma-separated value into trimmed, lower-cased, non-empty tokens.
fn split_tags(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(',')
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
}

/// Parse a `[tags: pii, regulated]` marker out of a column comment; empty if
/// none.
fn parse_comment_tags(comment: &str) -> Vec<String> {
    let Some(start) = comment.find("[tags:") else {
        return Vec::new();
    };
    let inner_start = start + "[tags:".len();
    let Some(rel_end) = comment[inner_start..].find(']') else {
        return Vec::new();
    };
    split_tags(&comment[inner_start..inner_start + rel_end]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use unitycatalog_common::models::tables::v1::Column;

    fn col(name: &str, comment: Option<&str>) -> Column {
        Column {
            name: name.to_string(),
            comment: comment.map(|c| c.to_string()),
            ..Default::default()
        }
    }

    fn tags(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn classifies_table_and_column_tags_from_properties_and_comments() {
        let table = Table {
            properties: std::collections::HashMap::from([
                ("tags".to_string(), "Regulated, internal".to_string()),
                ("tag.ssn".to_string(), "pii,sensitive".to_string()),
            ]),
            columns: vec![
                col("ssn", Some("SSN [tags: pii]")),
                col("email", Some("contact [tags: pii, contact_info]")),
                col("id", None),
            ],
            ..Default::default()
        };

        let c = ConventionTagProvider.classify(&table).await.unwrap();
        // Table tags are lower-cased and split.
        assert_eq!(c.table_tags, tags(&["internal", "regulated"]));
        // ssn merges the property tags and the comment tag.
        assert_eq!(
            c.column_tags.get("ssn").unwrap(),
            &tags(&["pii", "sensitive"])
        );
        // email comes only from the comment marker.
        assert_eq!(
            c.column_tags.get("email").unwrap(),
            &tags(&["contact_info", "pii"])
        );
        // An untagged column is absent.
        assert!(!c.column_tags.contains_key("id"));
    }

    #[test]
    fn acl_facts_read_owner_and_readers_writers() {
        let table = Table {
            owner: Some("User::\"alice\"".to_string()),
            properties: std::collections::HashMap::from([
                (
                    "readers".to_string(),
                    "readers,privileged_readers".to_string(),
                ),
                ("writers".to_string(), "lakehouse_admins".to_string()),
            ]),
            ..Default::default()
        };
        let (owner, readers, writers) = table_acl_facts(&table);
        assert_eq!(owner.as_deref(), Some("User::\"alice\""));
        assert_eq!(readers, tags(&["privileged_readers", "readers"]));
        assert_eq!(writers, tags(&["lakehouse_admins"]));
    }

    #[tokio::test]
    async fn empty_metadata_yields_empty_classification() {
        let c = ConventionTagProvider
            .classify(&Table::default())
            .await
            .unwrap();
        assert!(c.table_tags.is_empty());
        assert!(c.column_tags.is_empty());
    }

    #[tokio::test]
    async fn convention_governed_tags_fold_table_and_column_keys() {
        let table = Table {
            properties: std::collections::HashMap::from([
                // Table-level governed tag (no dot after the prefix).
                (
                    "govtag.classification".to_string(),
                    "restricted".to_string(),
                ),
                // Per-column governed tags.
                ("govtag.ssn.pii".to_string(), "ssn".to_string()),
                (
                    "govtag.salary.classification".to_string(),
                    "secret".to_string(),
                ),
                // A non-governed property is ignored.
                ("tags".to_string(), "internal".to_string()),
            ]),
            ..Default::default()
        };

        let g = ConventionGovernedTagProvider
            .governed_tags(&table)
            .await
            .unwrap();

        // Table-level tag, key/value preserved verbatim (not lower-cased/split).
        assert_eq!(
            g.table_tags.get("classification").map(String::as_str),
            Some("restricted")
        );
        // Per-column governed tags land under their column, keyed by tag key.
        assert_eq!(g.column_tags.get("ssn").unwrap().get("pii").unwrap(), "ssn");
        assert_eq!(
            g.column_tags
                .get("salary")
                .unwrap()
                .get("classification")
                .unwrap(),
            "secret"
        );
        // The opaque `tags` property is not a governed tag.
        assert!(!g.table_tags.contains_key("tags"));
    }

    #[tokio::test]
    async fn convention_governed_tags_empty_when_no_govtag_properties() {
        let g = ConventionGovernedTagProvider
            .governed_tags(&Table::default())
            .await
            .unwrap();
        assert!(g.table_tags.is_empty());
        assert!(g.column_tags.is_empty());
    }

    /// The UC-backed provider sources tags best-effort: when the catalog is
    /// unreachable, every per-entity list fails and folds to "no tags" rather
    /// than propagating an error — resolution is never blocked (the mask decision
    /// stays fail-closed at the Cedar layer). Pointing the client at a dead
    /// address exercises exactly that path without a mock server.
    #[tokio::test]
    async fn unity_governed_tags_fail_open_when_catalog_unreachable() {
        // A closed localhost port refuses instantly, so the list requests fail
        // fast without depending on any external service or timeout.
        let client = UnityCatalogClient::new_unauthenticated(
            url::Url::parse("http://127.0.0.1:1/").unwrap(),
        );
        let provider = UnityGovernedTagProvider::new(client);
        let table = Table {
            catalog_name: "hr".to_string(),
            schema_name: "security".to_string(),
            name: "employees".to_string(),
            full_name: "hr.security.employees".to_string(),
            columns: vec![col("ssn", None)],
            ..Default::default()
        };

        let g = provider.governed_tags(&table).await.unwrap();
        assert!(g.table_tags.is_empty());
        assert!(g.column_tags.is_empty());
    }
}
