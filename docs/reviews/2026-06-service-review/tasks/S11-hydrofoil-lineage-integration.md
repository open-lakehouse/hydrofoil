# S11 — hydrofoil: lineage integration fixes

| | |
|---|---|
| Status | ✅ **Done** — commit `4e759dd` (2026-06-12), incl. drain wiring + ADR 0003 update |
| Target repo | `open-lakehouse` (crates/hydrofoil, touches crates/open-lineage config surface) |
| Depends on | S10 (uses its `shutdown()`/drain API; job-name change pairs with S13's read model) |
| Scope | One PR |
| Findings | C5-partial (major), C7-partial (major), C9-partial (minor) |

## Mission

You are working in `open-lakehouse`. Hydrofoil emits OpenLineage run events around
Flight SQL queries: gRPC metadata → per-request lineage context → an instrumented
planner wraps the physical plan in `OpenLineageExec`, which emits START at execution
and COMPLETE/FAIL at stream end. This session fixes how hydrofoil *uses* the producer:
run identity, job identity, failure coverage, config plumbing, and shutdown.

## Findings to fix

### C5-partial [major] Prepared statements re-executed under one pinned runId

- `crates/hydrofoil/src/server.rs:666-683` — `do_get_prepared_statement` replans via
  the instrumented planner with `stored.lineage`, whose run_id was pinned at
  `get_flight_info`/creation (`server.rs:487`). Executing a prepared statement N
  times emits N START + N terminal events under the **same** runId — exactly-once is
  violated, and a FAIL from execution 2 can overwrite a COMPLETE from execution 1 in
  any state store.

**Fix:** mint a fresh run_id per execution, keeping the planning/creation run id as
the `parent` run facet (ADR 0003 covers run-id correlation — stay consistent with
it). Test: two executions of one prepared statement produce two distinct runIds with
the same parent.

### C7-partial [major] Constant job name collapses the lineage graph to one node

- `crates/open-lineage/src/builder.rs:139-141` — job name defaults to the constant
  `"datafusion_query"`; hydrofoil never sets `job_name`
  (`crates/hydrofoil/src/server.rs:218-231`). Every query is the same Marquez job;
  combined with the read-side latest-event-wins model, the UI shows only the most
  recent query's lineage.

**Fix:** derive a stable per-statement job name in hydrofoil's lineage context:
client-supplied job header if present (extend the gRPC metadata parsing), else a
normalized-SQL hash (e.g. `query-<first-12-hex>` over whitespace-normalized SQL).
Namespace stays config-driven. Document the header in the module docs.

### C9-partial [minor]

1. **Queries failing at logical planning emit nothing** —
   `crates/hydrofoil/src/server.rs:492-496`: START is only emitted inside
   `create_physical_plan`; SQL/resolution errors during `create_logical_plan` are
   invisible to lineage. Emit a START+FAIL (or a single FAIL with the error facet)
   from the server when logical planning errors, using the same run/job identity.
2. **Lineage noise from internal queries** —
   `crates/hydrofoil/src/session/mod.rs:749-756`: every session with a lineage
   client instruments everything, so information_schema / metadata RPC queries emit
   events with fresh run ids and the default job name. Suppress when the extracted
   lineage has no inputs and no outputs, or when the statement comes from a metadata
   RPC path.
3. **Env-var config bridge** — `crates/hydrofoil/src/main.rs:49-52` does
   `unsafe { env::set_var }` for the OpenLineage namespace, re-read deep in the
   request path (`server.rs:226`, `session/mod.rs:754`) via
   `OpenLineageConfig::default()`. Build one `OpenLineageConfig` at startup from the
   hydrofoil TOML config and pass it through `FlightSqlServiceImpl`/session
   construction; delete the env bridge.
4. **Wire producer shutdown drain** — call S10's `OpenLineageClient::shutdown()`
   from hydrofoil's shutdown path so queued terminal events flush on exit.

## Constraints

- Run-id semantics must stay consistent with ADR 0003
  (`docs/adr/0003-per-statement-run-id-correlation.md`) — update the ADR if the
  parent-run refinement changes its wording.
- Crates are unpublished: change APIs freely, no compatibility shims.
- Stage your changes and propose a commit message, but do **not** run `git commit` —
  the user signs commits. Attribute AI work as "AI assisted by Isaac" if attribution
  is included.

## Verification

- `cargo test -p hydrofoil -p open-lineage` with new tests: distinct runIds per
  prepared-statement execution (same parent); job-name derivation (header wins, hash
  fallback, stable across identical SQL); planning-failure emits FAIL;
  information_schema query emits no events.
- Live-stack check if available (`environments/`, `just`): run two different queries
  via `notebooks/duckdb_flight.py` and confirm the lineage UI / read API shows two
  distinct jobs.
