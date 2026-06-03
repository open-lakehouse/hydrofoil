# OpenLineage on DataFusion — technical design

> A concise record of the design decisions and patterns behind the
> `datafusion-open-lineage` crate. Source material for a blog post on how we
> extend DataFusion to build a full-featured, governed object-store query
> service.

## Why

We run a Flight SQL query service (`hydrofoil`) on Apache DataFusion over object
storage and Delta/Unity Catalog tables. A query engine in a lakehouse isn't
finished when it returns rows — it has to tell the rest of the platform *what it
touched*: which datasets were read, which were written, and how columns flow
between them. That's [OpenLineage](https://openlineage.io): an open standard for
lineage events that catalog and observability tools (Marquez, DataHub, …)
already consume.

The goal: emit spec-conformant lineage from DataFusion **without forking the
engine**, as a crate that drops into any `SessionContext`, with the wire
transport and the orchestration context both pluggable. This mirrors how we
already extend DataFusion for Cedar authorization and Unity Catalog resolution —
lineage is the third capability layered onto the same session.

## Design principles

1. **Plug in, don't fork.** Use DataFusion's published extension points; ship a
   standalone crate, not a patched engine.
2. **Conform to the ecosystem.** Match the OpenLineage object model and naming
   spec exactly — interoperability is the whole point.
3. **Lineage must never break the query.** Emission is best-effort, async, and
   swallows its own errors.
4. **Pluggable at the edges, opinionated in the middle.** The transport (where
   events go) and the context (orchestration metadata) are traits; the model and
   the plan extraction are fixed and correct.
5. **Learn from prior art, defer the rest.** Adopt the patterns mature
   integrations (Spark) have validated; skip the machinery that doesn't earn its
   complexity in a first version.

## Critical decision 1 — Where to hook: wrap the `QueryPlanner`

DataFusion offers several extension seams: `AnalyzerRule`, `OptimizerRule`,
`PhysicalOptimizerRule`, custom `ExecutionPlan` nodes, and the `QueryPlanner`
trait. We wrap the **`QueryPlanner`**.

Rationale: `QueryPlanner::create_physical_plan(&LogicalPlan, &SessionState)`
receives the *fully optimized* logical plan — the richest lineage signal (scans
have their projections/filters pushed down, so we see exactly which columns are
read) — **and** the `SessionState`, which is how per-query context reaches us. It
also straddles the success/failure boundary of planning, giving a natural place
to emit START before and COMPLETE/FAIL after.

The wrapper preserves any existing planner:

```rust
let inner = state.query_planner().clone();
SessionStateBuilder::from(state)
    .with_query_planner(Arc::new(OpenLineageQueryPlanner { inner, client, context, config }))
    .build()
```

This is the same composition pattern `datafusion-tracing` uses, and it composes
cleanly with our other customizations — the Cedar policy check and the
`datafusion-tracing` physical-optimizer rule all coexist on one session because
each wraps rather than replaces.

**Public surface** is one call, matching `datafusion-tracing`'s ergonomics:

```rust
let state = instrument_session_state(state, client, context_provider, config);
```

### Known limitation, by design

Wrapping the planner means COMPLETE/FAIL currently fire at **plan-creation**
time, not end-of-execution. A query that plans cleanly but errors mid-stream
emits a spurious COMPLETE. We shipped this knowingly for v1 (inputs/outputs/
column lineage are already correct) and scoped the fix as a follow-up: wrap the
*physical plan* in an instrumented `ExecutionPlan` node and emit COMPLETE/FAIL
from a `Drop`-based stream guard once all partitions drain — the same technique
`datafusion-tracing` uses to harvest metrics. That also unlocks runtime
row/byte statistics as run facets. See *Cross-RPC correlation* below for why the
hook has to survive across requests.

## Critical decision 2 — Deriving lineage from the logical plan

Lineage extraction is a `TreeNodeVisitor` walk over the optimized
`LogicalPlan` — the exact pattern our Cedar integration uses to find tables to
authorize, which keeps the two readable side by side.

- **Inputs**: every `TableScan` → an input dataset, with a `SchemaDatasetFacet`
  built from the scan's `projected_schema`.
- **Outputs**: `Dml` (`Insert`/`Update`/`Delete`/`Ctas`) and
  `CreateExternalTable` → output datasets.
- **Column lineage** is the marquee feature, and follows the same node set
  Spark's `ExpressionDependencyCollector` handles. For each `Projection`, we pair
  every output field with its defining `Expr`, collect the expression's
  `column_refs()`, and resolve each `Column.relation` back to its source dataset.
  An identity column (`Expr::Column`) is `DIRECT/IDENTITY`; any computed
  expression is `DIRECT/TRANSFORMATION`.

### The subtle bit: identity lineage lives on the scan

A trivial `SELECT a, b FROM t` *has no `Projection` node* after optimization —
the identity projection is absorbed into the scan's `projection=[a, b]`. If you
only handle `Projection`, you silently lose column lineage for the most common
query shape. So we also emit identity lineage directly from the `TableScan`
(each projected column maps 1:1 to itself), and let a `Projection` above
override it for transformed columns. The visitor runs top-down, so the
projection's richer mapping is recorded first and the scan uses `entry().or_*`
to fill only what's missing. This "read the optimized plan, and know what the
optimizer did to it" insight is the kind of thing that only surfaces once you
test against real plans.

### No silent truncation

Plan nodes we don't yet extract from log a `trace` line rather than vanishing —
coverage gaps stay visible, per OpenLineage's "don't silently drop" guidance.

## Critical decision 3 — Spec conformance is non-negotiable

Lineage is only useful if other tools accept it, so we model the OpenLineage
object spec precisely with `serde` types rather than approximating it:

- The six `eventType` values, a constant `runId` (UUIDv7) across START/COMPLETE/
  FAIL, top-level `producer` + `schemaURL`.
- Every facet embeds a `BaseFacet { _producer, _schemaURL }` (the underscore
  prefix is mandated to avoid collisions) pointing at versioned spec URLs.
- The facets a query engine is expected to populate: `processing_engine` (engine
  name/version), `schema`, `dataSource`, `columnLineage` (with the
  `DIRECT`/`INDIRECT` + subtype model), `sql`, `jobType`, plus `parent` and
  `errorMessage`.

Two `serde` details that bit us and are worth calling out:

- `#[serde(rename_all = "camelCase")]` turns `schema_url` into `schemaUrl`, but
  the spec wants `schemaURL` — so it needs an explicit `#[serde(rename)]`.
  Conformance means matching the spec's casing, not Rust's idiom.
- Facet bags carry `#[serde(flatten)] extra: Map<String, Value>` so
  context-supplied custom facets merge into the event without the crate
  enumerating every possible facet.

There is **no official OpenLineage Rust client** (only Java/Python/Go), so we own
this model — which is also why getting it exactly right matters.

### Naming-spec mapping

Dataset `(namespace, name)` follows the OpenLineage naming spec — this is what
lets graphs join across tools. Object-store URLs map as
`s3://bucket/key → namespace s3://bucket, name key`; `file:///p → namespace
file, name /p`. Because a bare `TableScan` carries only a `TableReference` (not a
storage location), v1 names from the qualified table reference and reserves the
`symlinks` facet for publishing the physical path + catalog identity once the
host integration provides it — the same physical-vs-catalog split Spark uses.

## Critical decision 4 — Pluggable transport, async, fail-safe

The sink is a trait, named to match OpenLineage's own SPI:

```rust
#[async_trait]
pub trait Transport: Send + Sync {
    async fn emit(&self, event: &RunEvent) -> Result<(), TransportError>;
}
```

Defaults: `NoopTransport` (the safe default when lineage isn't configured) and
`ConsoleTransport` (dev/test). The production default, behind the `http` feature,
is `CloudClientTransport` — built on our `olai-http` `CloudClient`, which already
handles bearer-token, Databricks, and cloud-credential auth. Emitting a lineage
event is then just `client.post(endpoint).json(event).send()`, and we inherit
auth against deployed, secured services for free. Keeping it feature-gated means
the core crate has no HTTP dependency.

**The fail-safe guarantee lives in `OpenLineageClient`.** `emit` is non-blocking:
it `try_send`s onto a bounded channel and, if the queue is full or closed, drops
the event with a warning. A background task drains the channel and calls the
transport, swallowing and logging any error. The query path never blocks on
lineage and never fails because of it — the Rust/`tokio` equivalent of Spark's
`asyncTaskQueue` circuit breaker. We read the standard `OPENLINEAGE_URL` /
`OPENLINEAGE_API_KEY` / `OPENLINEAGE_ENDPOINT` env vars so the integration
behaves the way operators already expect.

## Critical decision 5 — Pluggable orchestration context

A query's lineage often belongs to a larger run — an Airflow DAG task, a
Databricks job. OpenLineage models this with a `ParentRunFacet`. But *how* that
context arrives differs per orchestrator, so we don't hardcode it; we inject it:

```rust
#[async_trait]
pub trait LineageContextProvider: Send + Sync {
    async fn context(&self, session_state: &SessionState) -> LineageContext;
}
```

`LineageContext` carries optional `run_id`, job identity, a `parent_run` facet,
and free-form `run_facets`/`job_facets` maps — open extension points so an
integration attaches whatever its orchestrator provides. The provider is `async`
and receives the `SessionState`, which is the channel by which per-request data
reaches planning.

### The context-forwarding problem (and the bridge)

There is **no standard OpenLineage header** for forwarding parent context — every
integration uses native config (Spark properties, env vars). For our Flight SQL
service we define our own gRPC metadata keys, mirroring Spark's discrete
`parent*` property names (`x-openlineage-parent-run-id`, …; slash-safe, unlike
the single `{ns}/{name}/{runId}` form).

The architectural seam: `LineageContextProvider::context()` only sees a
`SessionState`, but the headers arrive on the gRPC request. The bridge is a
**typed `SessionConfig` extension** — the request handler parses metadata into a
`LineageContext`, attaches it via `SessionConfig::with_extension`, and the
provider reads it back with `get_extension`. DataFusion internals stay unaware of
gRPC; orchestration context still flows to planning.

## Cross-RPC correlation (the session problem)

A single client operation is several Flight SQL RPCs: `get_flight_info_statement`
*plans* (where START fires), then a separate `do_get_statement` *executes* (where
the future COMPLETE/FAIL belongs) — often on different connections. Correct
lineage therefore needs one stable `runId` to survive across RPCs.

The pattern we're designing toward: a statement handle owns its `runId` and a
snapshot of its `LineageContext`, minted at plan time and reused at execution
time. This is the prerequisite for the execution-accurate COMPLETE/FAIL
follow-up — the physical-plan wrapper has to emit under the *same* run id START
used. (Today's server still uses a demo session stub; the real session/statement
store is designed in `docs/session-management.md` and intentionally deferred.)

## Patterns adopted vs. deferred

**Adopted** (validated by mature integrations, mostly Spark): planner-wrapping as
the hook; `TreeNodeVisitor` plan walk; the `DIRECT`/`INDIRECT` column-lineage
model; spec-exact facets with `_producer`/`_schemaURL`; async fail-safe emission;
standard env-var config; pluggable transport.

**Deferred** (complexity that doesn't earn its place in v1): the full
`OpenLineageEventHandlerFactory` ServiceLoader-style SPI for third-party facet
builders — Rust has no ServiceLoader, and a fixed visitor set plus trait seams
covers us until a second consumer appears; Kafka/GCS/composite transports;
JVM-specific circuit breakers (our bounded queue covers the safety goal);
end-of-execution COMPLETE/FAIL and runtime-statistics facets (the physical-plan
wrapper follow-up).

## The bigger picture

OpenLineage is one of three capabilities we layer onto the same DataFusion
session through its extension points — alongside **Cedar policy enforcement**
(authorize the optimized plan before execution) and **Unity Catalog resolution**
(resolve `catalog.schema.table` and vend object-store credentials at plan time).
The recurring pattern is the same: *intercept at planning, read the
`LogicalPlan`/`SessionState`, compose by wrapping rather than replacing.* That's
how you build a full-featured, governed object-store query service on DataFusion
without forking it — which is the story this design is one chapter of.
