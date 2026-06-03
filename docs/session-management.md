# Session management & lineage context (design)

> Status: **implemented** (2026-06). The session lifecycle described below is now
> wired: a three-layer Engine / Session / per-query-context model with
> protocol-derived session ids, per-statement run-id correlation, per-session
> credential isolation, and a per-query agent-context seam. The decisions are
> recorded as ADRs — see [`docs/adr/`](adr/README.md), specifically
> [0001](adr/0001-layered-session-context-model.md)–[0005](adr/0005-per-query-agent-governance-context.md).
> The taint ledger, central PDP, and agent PEP from
> [`platform-policy-architecture.md`](platform-policy-architecture.md) remain
> future work.

## Problem

A single logical client operation spans several Flight SQL RPCs, often on
different connections:

```
client.execute("SELECT …")
  ├─ get_flight_info_statement        # parse + plan, mint a statement handle
  │     └─ create_physical_plan       # <- OpenLineage START / COMPLETE fire here (v1)
  └─ do_get_statement(ticket)         # execute the handle, stream results
        └─ (future) end-of-stream     # <- where COMPLETE/FAIL should fire (follow-up)
```

Prepared statements add more hops:

```
CreatePreparedStatement (action)      # mint prepared handle
  ├─ get_flight_info_prepared_statement
  └─ do_get_prepared_statement(ticket)
ClosePreparedStatement (action)
```

Two consequences for lineage:

1. **Correlation across RPCs.** One query must carry one stable `runId` across
   START (emitted at plan time in `get_flight_info_*`) and COMPLETE/FAIL
   (emitted at execution time in `do_get_*`, once the follow-up lands). Today
   these are different RPCs with no shared identity.
2. **Per-request context.** Orchestration context (parent run/job) arrives as
   gRPC request metadata on the *first* RPC of an operation, but the
   `LineageContextProvider` only sees a `SessionState` during planning. The
   context has to be parked somewhere both the planner and later RPCs can read.

The current `get_ctx` caches a single `LakehouseCtx` under the literal key
`"key"` and reuses it for every request — fine for the demo, but it means
per-request context only ever reflects the first caller, and there is no notion
of a client session.

## Target session model

Introduce an explicit **session** keyed by a client-provided session id, and a
**statement** scoped to a session. Sketch (not yet implemented):

```
SessionId        # stable per client connection/session (Flight SQL handshake / cookie)
  └─ LakehouseCtx (SessionContext + policy + principal + unity + lineage context)
       └─ StatementId (Uuid)   -> { LogicalPlan, run_id, LineageContext snapshot }
```

- **Session identity comes from the protocol, not a constant.** Flight SQL
  supports session establishment via `handshake` and session affinity via the
  `set-cookie` / `cookie` headers (and `SetSessionOptions`/`GetSessionOptions`).
  A real `get_ctx` resolves the session id from the request metadata/cookie and
  looks up (or creates) the matching `LakehouseCtx`, instead of `"key"`.
- **Statements own the run identity.** When `get_flight_info_statement` mints a
  statement handle it should also mint the lineage `run_id` and snapshot the
  `LineageContext`, storing both alongside the plan in `self.statements`. The
  later `do_get_*` RPC looks the handle up and reuses the same `run_id` — so
  START (plan time) and COMPLETE/FAIL (execution time) correlate. This is the
  hook the execution-accurate COMPLETE/FAIL follow-up depends on.
- **Lifecycle.** Sessions expire (TTL / handshake close); statements are removed
  on `do_get` completion (statement) or `ClosePreparedStatement` (prepared).

### How context reaches the planner

`LineageContextProvider::context()` receives only a `SessionState`. The bridge
is a **typed `SessionConfig` extension**: parse the request metadata once (in
`get_ctx`), build a `LineageContext`, and attach it with
`SessionConfig::with_extension(Arc<LineageContext>)`. The
`HydrofoilContextProvider` reads it back via `config().get_extension()`. This
keeps DataFusion internals unaware of gRPC while letting per-session context
flow to planning. (Per-statement overrides, e.g. a distinct `run_id`, would be
threaded the same way once statements own run identity.)

## Header convention for parent-run context

OpenLineage defines **no** standard HTTP/gRPC header for forwarding parent-run
context (confirmed against the spec, the Java/Python clients, and the Spark /
Trino / Flink / dbt integrations — they all use native config/properties/env
vars; see issue OpenLineage#4412 for an in-flight JSON-envelope proposal). We
therefore define our own gRPC metadata keys, mirroring Spark's discrete
`spark.openlineage.parent*` property names (slash-safe; avoids the
slash-in-names parsing bug of the single `{ns}/{name}/{runId}` form):

| Metadata key (lowercase)                  | Maps to                         |
| ----------------------------------------- | ------------------------------- |
| `x-openlineage-parent-run-id`             | `parent.run.runId`              |
| `x-openlineage-parent-job-namespace`      | `parent.job.namespace`          |
| `x-openlineage-parent-job-name`           | `parent.job.name`               |
| `x-openlineage-root-parent-run-id`        | `parent.root.run.runId`         |
| `x-openlineage-root-parent-job-namespace` | `parent.root.job.namespace`     |
| `x-openlineage-root-parent-job-name`      | `parent.root.job.name`          |

Rules (from the `ParentRunFacet` schema): parent requires all three of
run-id/job-namespace/job-name together; root is optional but, if present,
requires all three of its fields. Partial sets are ignored. We watch
OpenLineage#4412 in case a delivery mechanism is standardized later.

## What landed

The session-management rework is implemented in
`crates/hydrofoil/src/{engine,session,server,agent,identity,lineage}.rs`:

- **Three-layer model** — `Engine` (process-wide) → `Session` (per connection,
  keyed by session id) → `LakehouseSession` (per query, with `SessionConfig`
  extensions). See [ADR-0001](adr/0001-layered-session-context-model.md).
- **Protocol-derived session ids** — minted in `do_handshake`, returned via
  `authorization: Bearer` + `x-session-id`, resolved from cookie/header/Bearer
  with a stable per-principal ephemeral fallback for no-handshake clients. The
  `get_ctx` per-principal singleton is gone. See
  [ADR-0002](adr/0002-flight-sql-session-identity.md).
- **Per-statement `run_id`** — minted and snapshotted into the session-scoped
  statement store at planning, reused at `do_get`, so OpenLineage START and
  COMPLETE/FAIL share one `runId`. See
  [ADR-0003](adr/0003-per-statement-run-id-correlation.md).
- **Per-session credential isolation** — each session owns its `RuntimeEnv` /
  object-store registry, so vended Unity Catalog credentials never cross
  principals. See [ADR-0004](adr/0004-per-session-credential-isolation.md).
- **Per-query agent context** — `x-hydrofoil-agent-*` metadata parsed into an
  `AgentContext` and attached as a `SessionConfig` extension per query (logged
  for now). See [ADR-0005](adr/0005-per-query-agent-governance-context.md).
- The `LineageContext` header parser and `HydrofoilContextProvider` (the earlier
  scaffolding) are now driven end-to-end through the session layer.

## Still later (explicitly out of scope)

- **Per-user credential vending** — UC still vends with a shared `UC_TOKEN`;
  threading the request identity/token into `for_table` is the next step (see
  [ADR-0004](adr/0004-per-session-credential-isolation.md)).
- A real **authentication interceptor** (mTLS / validated bearer token) upstream
  of principal resolution, splitting session identity from the auth token.
- The **agent PEP, session taint ledger, and central PDP** from
  [`platform-policy-architecture.md`](platform-policy-architecture.md), which
  will consume the `AgentContext` seam.
