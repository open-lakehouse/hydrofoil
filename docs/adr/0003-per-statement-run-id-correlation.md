# 0003 — Per-statement `run_id` for START/COMPLETE correlation

> Status: **Accepted** (2026-06). Implemented in `crates/hydrofoil/src/engine.rs`
> (`StoredStatement`) and `crates/hydrofoil/src/server.rs`. Refines
> [`docs/open-lineage-design.md`](../open-lineage-design.md) and
> [`docs/session-management.md`](../session-management.md).

## Context

A query's OpenLineage events fire from two different RPCs:

```text
get_flight_info_statement   → create_physical_plan   → START
do_get_statement(ticket)    → execution stream end   → COMPLETE / FAIL
```

`LineageContext.run_id` is `Option<Uuid>`; when `None`, the planner mints a fresh
`Uuid::now_v7()` at plan time. Because the two RPCs built independent sessions and
neither pinned a run id, START and COMPLETE were minted **separately** and never
shared a `runId` — so events for one query could not be joined downstream.

## Decision

Pin the run id at statement creation and carry it through the session-scoped
statement store:

- In `get_flight_info_statement` / `do_action_create_prepared_statement`: mint
  `run_id = Uuid::now_v7()`, build the `LineageContext` from the request metadata
  with `run_id` set, and store it in the planning session's statement store as
  `StoredStatement { plan, lineage, created_at }`. Plan through a per-query
  session decorated with that context, so **START** carries the pinned id.
- In `do_get_statement` / `do_get_prepared_statement`: look the statement up in
  the resolved session, reattach the **same** `LineageContext` snapshot via
  `lakehouse_for_query`, and execute. **COMPLETE/FAIL** (emitted by the existing
  `OpenLineageExec` / `TrackedStream` at stream end) inherit the pinned id.

No change to the OpenLineage crate is needed — correlation is achieved purely by
sharing the run id across the two RPCs' contexts.

## Consequences

- START and COMPLETE/FAIL for one query share one `runId`; a unit test
  (`engine::tests::pinned_run_id_correlates_start_and_complete`) guards this.
- The statement store is **session-scoped** (on `Session`), not a global map, so
  handles are naturally isolated per connection and torn down with the session.
- Statements are removed on `do_get` completion (statement path) or
  `ClosePreparedStatement` (prepared path), with a TTL sweep backstopping handles
  that are minted but never fetched.
- `run_id` and `sql` live inside the `LineageContext` snapshot rather than as
  separate `StoredStatement` fields, keeping one source of truth.
