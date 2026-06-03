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

### Two hooks: START at plan time, COMPLETE/FAIL at execution time

The planner emits only **START** — at this point we have the optimized plan and
the context, which is everything START needs. But COMPLETE/FAIL must reflect the
actual *execution* outcome, not just that planning succeeded: a query that plans
cleanly can still error mid-stream. Emitting COMPLETE from the planner would
report success for a query that later fails.

So the planner wraps the root physical plan in an **`OpenLineageExec`** node and
hands it the pre-built COMPLETE event (carrying the same `runId` as START). That
node observes the result streams and emits the terminal event when execution
actually ends:

- COMPLETE when every output partition drains successfully;
- FAIL (with an `errorMessage` run facet) if any partition yields an error, or
  is dropped before exhaustion (a cancelled/abandoned query).

**Exactly-once across partitions.** A root plan has *N* output partitions, each
producing an independent stream. `OpenLineageExec` shares a small `RunState`
(an `AtomicUsize` of outstanding partitions + a `failed` flag) across them. Each
partition's stream is wrapped so that on terminal — exhaustion, error, or
`Drop` — it decrements the counter; the partition that brings it to zero emits
the single terminal event. Using `Drop` as the completion signal means
cancellation is handled without special-casing, and an `emitted` flag guards the
zero-partition edge case. This is the same `Drop`-based technique
`datafusion-tracing` uses to harvest metrics, specialized here for once-per-run
event semantics rather than per-stream metric aggregation.

A *planning* failure is different: there's no plan to wrap, so the planner emits
FAIL directly.

### Runtime statistics, and where the row count actually lives

The COMPLETE event also carries an **`outputStatistics`** facet (`rowCount`,
`size`) per output dataset — the runtime "how much did we write." The obvious
source is the inner plan's native `MetricsSet`, the way `datafusion-tracing`
harvests metrics on completion. But for the case that matters here — a write
(`INSERT`/`CTAS`) — the root `DataSinkExec` returns `None` from `metrics()`; the
rows-written count isn't there. DataFusion instead reports it *in the result
stream*: a write yields a single batch with one `count` (UInt64) column whose
value is the number of rows written.

So `OpenLineageExec` watches the batches flowing through each partition's
stream, recognizes that `count`-batch shape, and accumulates the rows-written
total in `RunState`; `size` falls back to a `bytes_scanned`-style plan metric
when the plan exposes one. A read (`SELECT`) has no output dataset, so the facet
is simply absent. This is a good example of "read the engine, not the docs": the
authoritative signal was in the data path, not the metrics API.

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

## Where the events fire, and the session question

Because we hook *physical* planning, the run's identity is intrinsically
self-consistent: START and the `OpenLineageExec` that emits COMPLETE/FAIL both
originate from a single `create_physical_plan` call, so they share one `runId`
without any external bookkeeping. In hydrofoil today that call happens inside the
`do_get_*` RPC's streaming task (`get_flight_info_statement` only builds the
*logical* plan and hands back a handle), so the whole START→COMPLETE/FAIL
lifecycle lives in one task — the physical-plan wrapper "just works" here.

The session question is about a *different* axis: a client's logical operation
spans several RPCs (`CreatePreparedStatement` → `GetFlightInfo` → `DoGet` →
`Close`), and we'll eventually want one lineage run per logical operation, plus
per-session orchestration context that outlives a single statement. That needs
real session/statement management — a session keyed by a protocol-derived id
(Flight SQL handshake / cookie), statements owning their `runId` — replacing the
current demo `get_ctx` stub. That work is designed in `docs/session-management.md`
and intentionally deferred; the lineage layer is already structured to slot into
it (the `LineageContextProvider` reads context from the session, and a statement
could pre-mint the `runId` the planner uses).

## Patterns adopted vs. deferred

**Adopted** (validated by mature integrations, mostly Spark): planner-wrapping as
the hook; `TreeNodeVisitor` plan walk; the `DIRECT`/`INDIRECT` column-lineage
model; spec-exact facets with `_producer`/`_schemaURL`; async fail-safe emission;
standard env-var config; pluggable transport; **end-of-execution COMPLETE/FAIL
via a `Drop`-based physical-plan wrapper** (`OpenLineageExec`), so terminal events
reflect runtime outcome, not just planning success; **`outputStatistics`
facets** (rows written, from the write-result `count` batch).

**Deferred** (complexity that doesn't earn its place yet): the full
`OpenLineageEventHandlerFactory` ServiceLoader-style SPI for third-party facet
builders — Rust has no ServiceLoader, and a fixed visitor set plus trait seams
covers us until a second consumer appears; Kafka/GCS/composite transports;
JVM-specific circuit breakers (our bounded queue covers the safety goal); richer
input-side statistics (bytes/files scanned per source); real session/statement
management for cross-RPC, per-operation runs.

## The bigger picture

OpenLineage is one of three capabilities we layer onto the same DataFusion
session through its extension points — alongside **Cedar policy enforcement**
(authorize the optimized plan before execution) and **Unity Catalog resolution**
(resolve `catalog.schema.table` and vend object-store credentials at plan time).
The recurring pattern is the same: *intercept at planning, read the
`LogicalPlan`/`SessionState`, compose by wrapping rather than replacing.* That's
how you build a full-featured, governed object-store query service on DataFusion
without forking it — which is the story this design is one chapter of.
