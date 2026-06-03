# Session management & lineage context (design)

> Status: **design + scaffolding only.** This documents where we want session
> management to go and lands the small, non-invasive pieces needed to forward
> OpenLineage context. It does **not** rewire the server's session lifecycle —
> the current `get_ctx` singleton stub stays in place for now.

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

## What lands now vs. later

**Now (this change):**
- Header key constants + a parser from gRPC `MetadataMap` → `LineageContext`
  (`crates/hydrofoil/src/lineage.rs`).
- `HydrofoilContextProvider` implementing `LineageContextProvider` by reading a
  `LineageContext` from a `SessionConfig` extension.
- Unit tests for header parsing and provider read-back.

**Later (explicitly out of scope here):**
- Replacing the `get_ctx` `"key"` singleton with protocol-derived session ids
  and a real session/statement store.
- Minting + storing per-statement `run_id` so COMPLETE/FAIL correlate with START
  across RPCs (prerequisite for the execution-accurate COMPLETE/FAIL follow-up).
- Attaching the parsed `LineageContext` as a `SessionConfig` extension inside
  `get_ctx` (wired once session management is reworked).
