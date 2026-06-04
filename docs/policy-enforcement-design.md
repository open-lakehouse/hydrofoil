# Policy enforcement on DataFusion — technical design

> A record of the design decisions behind hydrofoil's Cedar policy enforcement:
> where we are today (a coarse table-level access gate), where we're going
> (fine-grained row filters and column masks), and how we get there without
> forking DataFusion. Companion to `docs/open-lineage-design.md` — same engine,
> same house style, the *fourth* capability layered onto the same session.

## Why

We run a Flight SQL query service (`hydrofoil`) on Apache DataFusion over object
storage and Delta/Unity Catalog tables. A governed lakehouse has to answer two
*different* questions about every query:

1. **Access** — *may this principal touch this table at all?* A binary
   allow/deny over the tables (and the actions on them) a query references.
2. **Data governance** — *of the tables they may touch, which rows are visible
   and which columns must be masked?* Row-level security and column masking,
   evaluated per-principal, applied to the data itself.

Hydrofoil now answers both. **Layer 1** (`Policy::is_allowed`) walks the optimized
plan, finds the tables and actions, and returns `Decision::Allow` /
`Decision::Deny` via a Cedar authorizer. **Layer 2** (`Policy::table_policy` +
`govern_plan`, behind the `governance` feature) injects per-table row filters and
column masks derived from Cedar partial-evaluation residuals. This document
records the design behind both; the sections below note what has shipped versus
what remains (principally the authentication interceptor and edge-case
hardening).

A colleague's prior-art project, **Policast**
(`github.com/open-lakehouse/policast`), is a working demonstration of the second
question: governance authored once in **Cedar**, compiled to portable **CEL**,
and enforced inside both Spark (Catalyst) and DataFusion as row filters and
column masks. This document evaluates those patterns, decides which fit
hydrofoil and which don't, and specifies a two-layer architecture that **hardens
the coarse gate** and **adds fine-grained enforcement** — reusing the extension
patterns already validated by our OpenLineage and Unity Catalog integrations.

Policy enforcement is the fourth capability we layer onto one DataFusion session
through its extension points, alongside **Cedar authorization** (this, extended),
**Unity Catalog resolution**, and **OpenLineage**. The recurring move is the
same every time: *intercept at planning, read the `LogicalPlan`/`SessionState`,
compose by wrapping rather than replacing.*

## Design principles

These mirror the OpenLineage design deliberately — one engine, one set of rules.

1. **Plug in, don't fork.** Use DataFusion's published extension seams; keep the
   Cedar machinery feature-gated and in-crate.
2. **One policy language, one source of truth.** Author in Cedar; source from the
   same `OciPolicyProvider` policy set for both layers. Don't introduce a second
   expression language unless an external requirement forces it.
3. **Fail closed.** A policy that can't be fetched, parsed, or evaluated denies
   (or fully masks). Governance must never *open up* on error.
4. **Pluggable at the edges, opinionated in the middle.** The principal source
   and the residual→expression translation are traits; the plan walk and the
   layering are fixed and correct.
5. **Learn from prior art, defer the rest.** Adopt the row-filter / column-mask
   *capability* and the Cedar-annotation model Policast validated; skip the
   machinery (a portable CEL IR) that earns its keep only in a multi-engine world
   hydrofoil doesn't live in.

## Where we are today, precisely

> **Status update (2026-06).** What this section originally enumerated as the
> Layer-1 hardening backlog has **shipped**. The reusable policy machinery now
> lives in the `datafusion-cedar` crate (the policy analog of
> `datafusion-open-lineage`); `crates/hydrofoil/src/policy.rs` is a thin
> re-export. Both Layer 1 (the gate) and Layer 2 (governance) are wired into
> `LakehouseSession`. The bullets below record where each item landed; the one
> genuinely-open item is the authentication interceptor (the trust boundary).

- **The gate exists and runs in the right place.** *(done)*
  `LakehouseSession::create_physical_plan` (`crates/hydrofoil/src/session.rs`)
  injects governance, optimizes the logical plan, then calls
  `policy.is_allowed(&optimized_plan, &principal)`, denying the whole query
  before physical planning. Optimizing first is deliberate: projections and
  filters are pushed down, so the gate sees the columns actually read. The
  ingest / delta-connect path (`LakehouseCtx::execute_logical_plan`) enforces the
  same govern → optimize → gate contract.
- **The principal flows from request metadata.** *(done)* No more hardcoded
  `"User:default"`: `crates/hydrofoil/src/identity.rs::principal_from_metadata`
  parses the principal (and `role` / `region` attributes) from the
  `x-hydrofoil-principal` / `-role` / `-region` gRPC metadata, defaulting to
  `User::"anonymous"`. `with_principal` attaches it to the session at
  `Engine::new_session` → `create_session_for`. **Open:** the principal is still
  client-asserted — see the trust-boundary note below.
- **The Cedar request is populated.** *(done)* `datafusion-cedar/src/visitor.rs`
  builds each request's `Context` via `Context::from_pairs`
  (`table_context` carries catalog/schema/table identity), and the columns the
  visitor extracts from each `TableScan` flow through as `ReadTable(table,
  fields)`. The principal is supplied as a request-time entity
  (`principal.to_entity()`) so policies resolve `principal.<attr>`;
  `Entities::empty()` appears only as the fail-closed fallback when building that
  entity set fails.
- **Writes and DDL are authorized, not panicking.** *(done)*
  `PlanAction::WriteTable(TableReference)` and `CreateTable(TableReference)` are
  real variants lowered to `WRITE_TABLE_ACTION` / `CREATE_EXTERNAL_TABLE_ACTION`
  requests in `visitor.rs`; the `Dml` arm captures `dml.table_name`. No `todo!()`
  remains on this path; unsupported DDL maps to `DenyUnsupported` (fail closed).
- **A public constructor wires Cedar in.** *(done)*
  `CedarPolicy::from_oci(reference)` builds the authorizer from the
  `OciPolicyProvider`; `hydrofoil/src/main.rs` constructs it via `with_policy`
  when `HYDROFOIL_POLICY_REF` is set, else defaults to `StaticPolicy` (allow-all).
- **Still open — authentication (the trust boundary).** The metadata principal is
  the *transport* seam, not the *trust* boundary. A tonic interceptor that
  validates a Flight SQL bearer token / mTLS subject upstream of
  `principal_from_metadata` is not yet implemented; until it lands a client may
  assert any principal, so the gate is correct but unauthenticated.

## Evaluating Policast

Policast is the right capability built for a different constraint. Understanding
*why* it made its choices is what lets us keep the parts that fit.

### What Policast does

- **Authoring.** Policies are ordinary Cedar, annotated with `@id`,
  `@filter_type("row_filter" | "column_mask" | "deny_override")`,
  `@target_table` (`a.b.c`, `a.b.*`, or `*`), `@target_tag`, `@applies_to_tag`,
  and `@roles`. The `when`/`unless` conditions of each policy are compiled to a
  **CEL string**.
- **Manifest.** Compilation yields a `PolicyManifest { version, policies:
  Vec<CompiledPolicy> }`, where `CompiledPolicy` carries `{ id, effect, filter_type,
  target_table, column?, target_tag?, applies_to_tag?, cel_expression, applies_to?,
  description }`. The manifest is the engine-facing artifact; tag-scoped policies
  are expanded to concrete table/column bindings by a resolver *before* the engine
  sees them.
- **Enforcement (DataFusion).** A `GovernedTable` wraps each inner
  `TableProvider`. In `scan()` it builds row filters (CEL → DataFusion `Expr`,
  one stacked `FilterExec` each) and column masks (a single `ProjectionExec`
  replacing masked columns with a `"***"` literal). A `QueryIdentity { role,
  region?, name? }` binds `principal.*` to scalars at plan time so they
  constant-fold; `resource.<col>` becomes `col(<col>)`. `deny_override` keeps rows
  where `NOT(condition)`. Mask evaluation is **fail-closed** (mask on error).
- **Enforcement (Spark).** `policast-spark` applies the *same manifest* via
  injected Catalyst optimizer rules — stacked `Filter` nodes for row filters, a
  rebuilt `Project` substituting `"***"` literals for masks.

### Why CEL exists in Policast — and why that reason doesn't transfer

CEL is Policast's **portable intermediate representation**. Cedar is the authoring
and decision language; its condition clauses are compiled to CEL *strings* that
travel with the manifest and are re-evaluated/translated identically inside two
very different engines — Spark on the JVM (Catalyst) and DataFusion in Rust. CEL
is the lingua franca that makes "author once, enforce everywhere" true across a
JVM/Rust boundary.

Hydrofoil is a **single Rust engine that already links `cedar-policy`
in-process.** The portability that justifies CEL buys us nothing here, while a CEL
parser plus a CEL→`Expr` compiler is pure added surface to keep correct. So the
*capability* transfers; the *IR* does not (see Critical decision 2).

### What fits, and what doesn't

**Fits, adopt:**
- The row-filter / column-mask **capability** — exactly the governance hydrofoil
  lacks.
- The **Cedar-annotation policy model** (`@filter_type`, `@target_table`,
  `@roles`, tag scoping) — it keeps one authoring language and is human-readable.
- **`deny_override` semantics** (keep rows where `NOT(deny condition)`) and the
  **fail-closed** posture.
- The `principal.* → scalar`, `resource.<col> → column` binding — the mechanism
  that makes a policy condition into a pushdown-friendly predicate.

**Doesn't fit unchanged, adapt:**
- The **CEL IR** — replaced by native Cedar evaluation for hydrofoil (Critical
  decision 2), with a translator seam so a CEL/manifest backend can be added later
  if the org wants to share Policast manifests with Spark.
- The **`GovernedTable` TableProvider hook** — wrong layer for hydrofoil's
  principal-threading, async-policy-sourcing, and pushdown constraints (Critical
  decision 3). We inject the same `Filter`/`Projection` *logical* nodes, but
  higher up.

## Critical decision 1 — Two layers, not one

Access and data governance are different questions; conflating them produces a
gate that's either too coarse (can't mask) or a masker that re-derives access
control. We keep them separate and ordered:

```
LakehouseSession::create_physical_plan
  │  govern_plan(logical_plan, policy, principal)      LAYER 2 (new): inject row Filter + mask Projection per TableScan
  ▼
  │  inner.optimize(governed_plan)                      (pushes the row filter into the Delta scan, prunes masked-away cols)
  ▼
  │  policy.is_allowed(optimized_plan, principal)       LAYER 1 (exists, hardened): Allow/Deny over the tables/actions
  ▼  Deny ⇒ reject the whole query
  │  inner.query_planner().create_physical_plan(...)    OpenLineage planner wrapper composes here, now sees the mask nodes
  ▼
execution
```

- **Layer 1 — coarse access gate.** "May principal P perform action A on table
  T?" Cedar `is_allowed`. Deny short-circuits the entire query. This is the
  existing mechanism, hardened (see the backlog).
- **Layer 2 — fine-grained data governance.** "Which rows of T does P see, which
  columns are masked?" Injects logical `Filter`/`Projection` nodes per
  `TableScan`. New.

The two are independent. A query that survives Layer 1 still has its rows/columns
shaped by Layer 2; a query denied by Layer 1 never reaches data. Note that Layer 2
runs *before* optimization and Layer 1 *after* — Layer 1 keys off `TableScan`
table references, which the Layer-2 rewrite does not change (it adds no scans), so
the gate's contract is preserved.

## Critical decision 2 — Evaluate Cedar natively (residuals), don't carry a CEL IR

**Decision: derive row filters and column predicates from Cedar **partial
evaluation residuals**, via the authorizer we already have. Do not introduce a CEL
string IR inside hydrofoil.**

The mechanism: issue the governance request (e.g. `read_table` on `Table::"…"`)
but leave the per-row `resource` symbolic, calling
`Authorizer::is_authorized_partial`. Cedar returns a `PartialResponse` whose
**residual** is exactly the condition that must hold for the policy to permit a
given row — the row-filter predicate, with the constant principal/context
attributes (role, region, name) already folded away. Translate that residual into
a DataFusion `Expr` (`resource.<field> → col(field)`; comparisons/booleans →
`Expr` operators). This is the same `principal→scalar`, `resource→column` mapping
Policast hand-writes in CEL, obtained for free from the engine that already owns
the policy semantics.

This is reachable through the abstraction hydrofoil already uses.
**`cedar-local-agent` 3.0.0 exposes `Authorizer::is_authorized_partial`** behind a
`partial-eval` feature (`src/public/simple.rs`), delegating to
`cedar_policy::Authorizer::is_authorized_partial`. So both layers source from the
**same `OciPolicyProvider`-backed authorizer** — one policy set, one language, one
truth.

Why native over CEL, for hydrofoil specifically:
- **No portability requirement.** CEL is Policast's JVM/Rust bridge; hydrofoil has
  no JVM side. A CEL parser + compiler would be maintained surface with no
  consumer here.
- **The engine owns the semantics.** Residuals come from Cedar's own evaluator —
  no second implementation of "what this policy means" to drift out of sync with
  the gate.
- **Annotations still drive structure.** `@filter_type`, `@column`, `@roles`,
  `@target_table` are read off the policy via Cedar's annotation API to decide
  *whether* a policy is a row filter vs a column mask and *what* it applies to;
  only the *condition* is obtained as a residual instead of a CEL string.

**Keep the door open (trait seam).** The residual→`Expr` step lives behind a
`ResidualTranslator` (and the per-table resolution behind a `GovernanceEngine`
trait), so a future `CelTranslator` that consumes Policast's `PolicyManifest`
directly is an *additive* impl, not a rewrite.

**Documented alternative — adopt Policast's CEL manifest.** If the org later
standardizes on the Policast `PolicyManifest` as the cross-engine interchange
format (so Spark and hydrofoil share one artifact), hydrofoil consumes the
manifest and translates `cel_expression → Expr` via the `CelTranslator` seam. We'd
switch only under that explicit requirement; until then, native residuals are
less code and one fewer language.

**Risk to manage:** `partial-eval` is a non-default, still-evolving Cedar feature.
Residual shapes for unusual operators may be opaque. Mitigation: restrict the
*supported residual grammar* (equality/comparison, boolean combinators,
membership, `like`), **fail closed** on anything we can't translate (deny the row
/ fully mask the column), and feature-gate the whole path so it can be disabled.
The fallback, if partial eval proves insufficient, is to compile the
annotation-declared condition expression to `Expr` directly — still no CEL, same
`ResidualTranslator` seam.

## Critical decision 3 — Hook fine-grained enforcement at a pre-optimize plan rewrite

We need a place to inject the row `Filter` and the mask `Projection`. Four
candidates, evaluated for *this* codebase:

**A. `GovernedTable` TableProvider wrapper (Policast's choice).** Wrap each
resolved provider; do the work in `scan()`. *Rejected.* Hydrofoil builds providers
deep inside catalog resolution (`LakehouseTableProviderBuilder`,
`UnityCatalogProviderList`, `DeltaTableFactory`), where the only context is a
`TaskContext` *shared across principals* — the per-request principal isn't
reachable without re-plumbing that layer. It fragments enforcement across every
provider-construction site, fights `DeltaScanNext`'s own predicate/projection
pushdown (your injected `FilterExec` sits *above* the scan, so it isn't file
skipping), and is **invisible to OpenLineage's logical-plan `extract()`** — so
masking would be under-reported and the lineage `masking` flag could never be set.

**B. Registered `AnalyzerRule` / `OptimizerRule`.** Right *mechanism* (inject
logical nodes before optimization, let the optimizer push the filter down and
prune columns), wrong *registration site*: rules are built once in
`create_session`, before any principal exists, and `optimize()` hands them only
`&ConfigOptions` — **no principal**. Worse, rules are **sync** while policy
sourcing (OCI pull, partial eval) is **async**. Hard impedance mismatch.

**C. `QueryPlanner` wrapper (OpenLineage's choice).** Inherits B's principal
problem (planner built at `create_session`), *plus* it receives the
*already-optimized* plan — inject filters/masks here and the optimizer has already
run, so the row filter never reaches the Delta scan and masked columns were
already projected. You'd have to re-`optimize()`. Not worth it over D.

**D. Inline two-phase rewrite in `create_physical_plan`, before `optimize()`
(recommended).** `create_physical_plan` already holds `self.principal` and
`self.policy` and is already `async`. Rewriting the analyzed plan *before*
`self.inner.optimize()` means the injected row `Filter` pushes down into the Delta
scan (real file/stats pruning via the provider's existing
`supports_filters_pushdown`) and mask `Projection`s get column-pruned naturally —
exactly B's optimizer win, at the one site that actually has the principal and is
async. The coarse gate keeps seeing an optimized (now governed) plan; OpenLineage,
downstream in the planner wrapper, sees the mask nodes.

```rust
async fn create_physical_plan(&self, logical_plan: &LogicalPlan)
    -> Result<Arc<dyn ExecutionPlan>>
{
    // LAYER 2: inject row filters + column masks before optimization.
    let governed = crate::policy::enforce::govern_plan(
        logical_plan, self.policy.as_ref(), &self.principal,
    ).await?;
    let optimized_plan = self.inner.optimize(&governed)?;
    // LAYER 1 (hardened): coarse allow/deny over the (now governed) plan.
    if self.policy.is_allowed(&optimized_plan, &self.principal).await? == Decision::Deny {
        return exec_err!("Principal '{}' is not authorized to execute this query", self.principal);
    }
    self.inner.query_planner().create_physical_plan(&optimized_plan, &self.inner).await
}
```

**The async/sync bridge.** DataFusion's `TreeNodeRewriter` is sync, but policy
resolution is async. Use a two-phase pass: first an async walk that collects the
distinct `TableReference`s in the plan (reuse `resolve_table_references`, already
used for Unity resolution in `create_logical_plan`) and awaits per-table
enforcement into a `HashMap<TableReference, TablePolicy>`; then run a **sync**
`TreeNodeRewriter` over that map. Resolution stays async; the rewrite stays pure.

Proposed surface (extends the existing `Policy` trait; new `enforce` module):

```rust
// crates/hydrofoil/src/policy/mod.rs
pub struct TablePolicy {
    /// Conjunctive row-filter predicates over the table's columns
    /// (principal.* already folded to literals at translation time).
    pub row_filters: Vec<Expr>,
    /// column name -> replacement Expr (e.g. CASE / Literal) for masked columns.
    pub column_masks: HashMap<String, Expr>,
}

#[async_trait]
pub trait Policy: Debug + Send + Sync {
    async fn is_allowed(&self, plan: &LogicalPlan, principal: &EntityUid) -> Result<Decision>;

    /// Per-table row filters + column masks. Default: none (StaticPolicy).
    async fn table_policy(&self, _table: &TableReference, _principal: &EntityUid)
        -> Result<TablePolicy> { Ok(TablePolicy::default()) }
}

// crates/hydrofoil/src/policy/enforce.rs
struct GovernRewriter { policies: HashMap<TableReference, TablePolicy> }
impl TreeNodeRewriter for GovernRewriter {
    type Node = LogicalPlan;
    fn f_up(&mut self, node: LogicalPlan) -> Result<Transformed<LogicalPlan>> {
        let LogicalPlan::TableScan(scan) = &node else { return Ok(Transformed::no(node)); };
        let Some(tp) = self.policies.get(&scan.table_name) else { return Ok(Transformed::no(node)); };
        // Projection(masks) over the scan output, then Filter(and(row_filters)) on top,
        // built with LogicalPlanBuilder; masked columns aliased to their original names.
        Ok(Transformed::yes(/* governed subtree */))
    }
}
pub async fn govern_plan(plan: &LogicalPlan, policy: &dyn Policy, principal: &EntityUid)
    -> Result<LogicalPlan> { /* phase 1: async collect refs + table_policy; phase 2: sync rewrite */ }
```

`StaticPolicy` gets the trivial default (empty `TablePolicy`), so non-Cedar setups
are unaffected. No changes to `create_session`, the catalog providers, or the
OpenLineage wiring — the rewrite is invisible to them and composes downstream.

## Critical decision 4 — Principal identity flows like lineage context

Layer 1's hardcoded `"User:default"` is the blocker for everything: a real
policy needs a real principal. We solve it the way we already solved orchestration
context for OpenLineage — a provider trait reading a typed `SessionConfig`
extension — so the two read identically side by side.

Clone `crates/hydrofoil/src/lineage.rs` (`LineageContextExt` /
`HydrofoilContextProvider` / `context_from_metadata` / `with_lineage_context`)
into an identity module:

```rust
// crates/hydrofoil/src/identity.rs  (mirrors lineage.rs)
#[derive(Debug, Clone)]
pub struct PrincipalExt(pub PrincipalIdentity);   // distinct newtype: get_extension keys by TypeId

#[derive(Debug, Clone)]
pub struct PrincipalIdentity {
    pub uid: EntityUid,
    pub attributes: BTreeMap<String, RestrictedExpression>,  // role, region, name, ...
}

pub mod headers { /* x-hydrofoil-principal, x-hydrofoil-role, ... */ }
pub fn principal_from_metadata(meta: &MetadataMap) -> Result<PrincipalIdentity, Status>;
pub fn with_principal(cfg: SessionConfig, id: PrincipalIdentity) -> SessionConfig;

#[async_trait]
pub trait PrincipalProvider: Debug + Send + Sync {
    async fn principal(&self, state: &SessionState) -> Result<PrincipalIdentity>;
}
#[derive(Debug, Default)]
pub struct SessionConfigPrincipalProvider;   // reads PrincipalExt back (like HydrofoilContextProvider)
```

`get_ctx` parses the principal from the (authenticated) request metadata, attaches
it via `with_principal` before/at session creation, and stops using the `"key"`
singleton — context is keyed by a protocol-derived session id (the deferred
session-management work in `docs/session-management.md`). The principal already
threads through `LakehouseCtx`/`LakehouseSession`; we populate it from the
provider rather than a constructor literal.

**Trust boundary.** The metadata key is the *transport* seam, not the *trust*
boundary. The principal must be established by real authentication — a tonic
interceptor validating a Flight SQL bearer token / mTLS subject — upstream of
`principal_from_metadata`. A client-asserted header is never trusted on its own.

## Layer 1 hardening — shipped

The backlog that originally appeared here (the gaps that made the gate unsound) is
**done**; recorded below for traceability. The reusable bits live in
`datafusion-cedar`, not under `hydrofoil/src/policy/`.

| Gap (was) | Where it landed | Status |
| --- | --- | --- |
| Hardcoded principal; ctx shared under `"key"` | `identity.rs` `principal_from_metadata` + `with_principal`; session keyed per-connection in `engine.rs` | ✅ done |
| Empty request `Context`; columns dropped | `datafusion-cedar/src/visitor.rs` `table_context` → `Context::from_pairs`; `ReadTable(table, fields)` carries columns | ✅ done |
| Authorizes against `Entities::empty()` | `cedar.rs` supplies `principal.to_entity()`; `Entities::empty()` is only the fail-closed fallback | ✅ done |
| No public Cedar constructor | `cedar.rs` `CedarPolicy::from_oci`; wired in `main.rs` via `with_policy` | ✅ done |
| `WriteTable` / `CreateTable` panic (`todo!()`) | `visitor.rs` `PlanAction::{WriteTable,CreateTable}(TableReference)` → `WRITE_TABLE_ACTION` / `CREATE_EXTERNAL_TABLE_ACTION` | ✅ done |
| Error semantics undefined | `cedar.rs` / `govern.rs` fail closed: authorizer/partial-eval error ⇒ deny / mask-all; unsupported DDL ⇒ `DenyUnsupported` | ✅ done |
| **Authentication interceptor (trust boundary)** | not yet built — tonic interceptor validating bearer token / mTLS upstream of `principal_from_metadata` | ⛔ open |

## Risks

- **`partial-eval` maturity.** Non-default, evolving Cedar feature; reachable via
  `cedar-local-agent` but still hardening. *Mitigation:* restricted residual
  grammar, fail-closed on untranslatable residuals, feature flag, and the
  compile-the-annotated-condition fallback behind the same translator seam.
- **Mask vs predicate pushdown — leakage.** A `WHERE masked_col = 'x'` must
  evaluate against the *masked* value, and a user predicate must not be pushed
  *through* a mask `Projection` into the scan. *Mitigation:* keep user predicates
  above the mask projection; mark mask-projection outputs non-pushable; add
  plan-shape tests for `SELECT masked WHERE masked = …`.
- **The optimizer eating identity projections.** Just as a trivial `SELECT a, b
  FROM t` has its identity projection absorbed into the scan (the exact note in
  `open-lineage-design.md`'s "identity lineage lives on the scan"), an identity
  *mask* projection could be eliminated by `OptimizeProjections` and leak the raw
  column. *Mitigation:* the mask must be a non-identity expression (literal / CASE
  / hash), never a bare column; test `SELECT a, b FROM t` where `a` is masked.
- **Current `todo!()` panic.** Not merely a gap — DML/DDL through the Cedar policy
  panics the worker today. Phase 1 is a correctness fix, not an enhancement.
- **Principal leakage via the `"key"` singleton.** Until `get_ctx` keys by session
  id, a per-request principal would be shared globally. Fix it *with* the
  principal wiring, not after.
- **Feature gating.** All of Layer 2 sits under the existing `cedar` feature plus
  a `governance` / `partial-eval` sub-feature, so a build without Cedar (or with
  partial eval disabled) compiles and runs with the trivial `StaticPolicy`
  defaults.

## Patterns adopted vs. deferred

**Adopted** (validated by Policast and our own OpenLineage/Unity work): the
row-filter / column-mask capability; the Cedar-annotation policy model;
`deny_override` and fail-closed semantics; `principal→scalar` / `resource→column`
binding; inject *logical* `Filter`/`Projection` and let the optimizer push down;
the provider-trait + `SessionConfig`-extension pattern for per-request context;
sourcing both layers from one `OciPolicyProvider`.

**Deferred** (doesn't earn its place yet): the portable **CEL IR** (no JVM side to
bridge — re-enter only if a shared Spark manifest becomes a requirement, via the
`CelTranslator` seam); tag-scope *expansion* in-engine (assume a resolver expands
`@target_tag`/`@applies_to_tag` to concrete bindings, as Policast does); the real
session/statement store (designed in `docs/session-management.md`, a prerequisite
for non-leaky per-principal contexts but tracked there).

## Phased implementation

- **Phase 0 — de-risk, no behavior change.** ✅ *done.* `partial-eval` is wired
  through the `governance` feature on `datafusion-cedar` (and re-exported by
  hydrofoil's `governance` feature).
- **Phase 1 — harden Layer 1 (correctness).** ✅ *done.* Principal flow
  (`identity.rs`), populated `Context`, request-time principal entity,
  `CedarPolicy::from_oci`, write/DDL arms, fail-closed semantics — see the
  "Layer 1 hardening — shipped" table. (Reusable code lives in `datafusion-cedar`,
  not the originally-planned `hydrofoil/src/policy/`.)
- **Phase 2 — Layer 2 scaffolding (capability, gated).** ✅ *done.* Lives in
  `datafusion-cedar` (`govern.rs` `govern_plan`/`GovernRewriter`,
  `Policy::table_policy`, `translate.rs` `CedarResidualTranslator`); wired into
  `create_physical_plan` (and the ingest path) via the `governance` feature.
- **Phase 3 — edge-case hardening.** 🚧 *in progress.* Plan-shape tests exist in
  `govern.rs`; still to do: pushdown/mask-interaction tests, identity-projection
  survival test, broader `deny_override` coverage, and the demo that exercises the
  whole path (`notebooks/policy_demo.py`). The optional `CelTranslator` stays
  unimplemented behind its seam.
- **Phase 4 — authentication.** ⛔ *not started.* The tonic auth interceptor that
  establishes a trusted principal upstream of `principal_from_metadata`.

## The bigger picture

Policy enforcement is the fourth capability layered onto one DataFusion session
through its extension points — alongside Cedar authorization (extended here),
Unity Catalog resolution, and OpenLineage. The recurring pattern holds: *intercept
at planning, read the `LogicalPlan`/`SessionState`, compose by wrapping rather than
replacing.* Policast showed the row-filter/column-mask capability is worth having
and that Cedar is the right place to author it; hydrofoil's contribution is to
enforce it natively, in one language, at the one planning seam that already has the
principal — building a governed object-store query service on DataFusion without
forking it.
