# 0001 — Layered session/context model (Engine / Session / per-query)

> Status: **Accepted** (2026-06). Implemented in `crates/hydrofoil/src/engine.rs`
> and `crates/hydrofoil/src/session.rs`. Refines
> [`docs/session-management.md`](../session-management.md).

## Context

A single logical client operation spans several Flight SQL RPCs, and the server
needs to carry three *different* axes of context that change at different rates:

1. **Principal / identity** — authenticated, slow-changing, per connection.
2. **Session** — per client connection: catalogs, prepared statements, the
   credentials a connection may ever touch.
3. **Per-query context** — orchestration (lineage parent run) and, crucially,
   the *agent invocation*, which can differ from one query to the next *within a
   single session* (one human session may drive many distinct agent tasks).

The previous code collapsed these: `get_ctx` cached one `LakehouseCtx` per
principal uid and reused it for every request, with no notion of a connection
session and nowhere to hang per-query context.

We surveyed how serious DataFusion-based servers structure this:

- **Spice.ai** — a `SessionStore` (`Cache<session_id, ctx>`); session id minted
  at handshake; each session a `SessionStateBuilder::new_from_existing` fork of a
  base; principal bound at create.
- **InfluxDB IOx** — a long-lived `Executor`/`RuntimeEnv`; a fresh per-query
  `IOxSessionContext` with the trace span attached via
  `SessionConfig::with_extension`.
- **GreptimeDB** — an explicit per-connection `Session` vs. a per-query
  `QueryContext` with a request-scoped extension map.

All converge on the same cut: a long-lived engine, a per-connection session, and
a cheap per-query context carrying request-scoped data via config extensions.

## Decision

Adopt a three-layer model:

```text
Engine    — one per process; identity-independent inputs for building sessions
  └─ Session   — one per client connection; principal binding, statement store
       └─ LakehouseSession — one per query; cheap SessionState clone + per-request
                              SessionConfig extensions (lineage / agent context)
```

- **`Engine`** holds only identity-independent inputs (policy, Unity Catalog
  factory, OpenLineage client). `Engine::new_session(principal)` builds a fresh,
  principal-scoped `Session`.
- **`Session`** owns a DataFusion `SessionContext` and a statement store. It
  produces per-query `LakehouseSession`s via `lakehouse_for_query(lineage,
  agent)`, cloning its `SessionState` (cheap — internals are `Arc`-shared) and
  attaching per-request context as typed `SessionConfig` extensions.
- **`LakehouseSession`** is unchanged as the `Session`-trait impl that runs the
  govern → optimize → gate pipeline; it now reads lineage/agent context from the
  extensions.

Per-query agent context is read from request metadata at the call site and
attached to the per-query clone only — never persisted into the `Session` (see
[ADR-0005](0005-per-query-agent-governance-context.md)).

## Consequences

- Clean separation of the three context axes; the agent axis can vary per query
  while session and engine stay long-lived.
- `SessionConfig::with_extension` is the single sanctioned seam for threading
  per-request data through DataFusion's planner into our rules/providers — the
  same mechanism OpenLineage and Cedar already use.
- Notable departure from Spice/IOx: sessions are **not** forked from one shared
  base runtime, because that would share an object-store registry across
  principals — see [ADR-0004](0004-per-session-credential-isolation.md).
- `LakehouseCtx` is retained as a thin adapter (`Session::ctx()`) so the
  ingest / delta-connect paths compile unchanged.
