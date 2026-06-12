# S07 — hydrofoil: Flight server hardening + first server.rs tests

| | |
|---|---|
| Target repo | `open-lakehouse` (crates/hydrofoil) |
| Depends on | S01 (touches the same `SQLOptions` guards in server.rs — run after it to avoid conflicts) |
| Scope | One PR |
| Findings | B5 (major), B7 (major), B8 (major), B10-partial (minor) |

## Mission

You are working in `open-lakehouse`. `crates/hydrofoil/src/server.rs` (~900 lines)
implements the Flight SQL service: `get_flight_info_statement` plans a query and
returns a `FlightInfo` (schema + ticket); `do_get_statement` executes it. The Cedar
coarse authorization gate currently fires inside
`LakehouseSession::create_physical_plan` — i.e. at `DoGet` time. There is also an
HTTP query endpoint in `crates/hydrofoil/src/http.rs`. This session hardens the
request path: gate ordering, panic removal, honest RPC semantics, error hygiene — and
adds the first handler-level tests (the file currently has none).

## Findings to fix

### B5 [major] Result schema disclosed before authorization

- `crates/hydrofoil/src/server.rs:475-535` — `get_flight_info_statement` plans the
  query, runs `verify_plan` (DDL/DML guard only), and returns the result schema in
  the `FlightInfo`. The Cedar gate fires later, in `create_physical_plan` at `DoGet`.
  An unauthorized principal therefore learns column names/types of protected tables'
  query results. Same pattern in `do_action_create_prepared_statement` (returns
  `dataset_schema`).

**Fix:** run the coarse Cedar gate during `get_flight_info_statement` and
prepared-statement creation — plan → govern → optimize → `is_allowed` — before
returning any schema. Cheapest structure: produce the optimized+authorized plan once,
store it with the statement/ticket, and reuse it at `DoGet` instead of re-planning
(this also removes double planning). Keep the `DoGet` gate as defense in depth.

### B7 [major] Panics on the request path

- `crates/hydrofoil/src/server.rs:526-527` and `:568-569` —
  `.try_with_schema(plan.schema().as_arrow()).expect("encoding failed")`. A schema
  that fails IPC encoding panics the handler instead of returning a `Status`; a
  crafted schema is a DoS vector.

**Fix:** map to `Status` like the rest of the file
(`.map_err(|e| status!("encoding failed", e))?`). Grep the request path for other
`expect`/`unwrap` while there (e.g. the lineage-related `expect`s flagged at the same
lines) and convert any reachable from request input.

### B8 [major] `do_put_*_update` silently swallows writes

- `crates/hydrofoil/src/server.rs:713-741` — `do_put_statement_update` and
  `do_put_prepared_statement_update` return `Ok(-1)` without planning or executing
  anything. A client issuing `INSERT`/`UPDATE` via the update RPC believes it
  succeeded; nothing happens, and nothing is authorized.

**Fix:** return `Status::unimplemented` from both so clients don't silently lose
writes. (Implementing them properly means routing through the Cedar-gated
`execute_logical_plan` path — do that instead if it turns out small; otherwise leave
a TODO referencing this review.)

### B10-partial [minor] Error/disclosure and conformance hygiene

1. **`status!` leaks internals** — `crates/hydrofoil/src/server.rs:56-60` embeds
   `file!()`/`line!()` into client-visible `Status` messages, and
   `do_put_fallback` (`:798-834`) returns `Status::internal(format!("{e:?}"))` with
   debug-formatted internals. Keep file/line + debug detail in `tracing` logs; return
   clean, classified statuses to clients (`invalid_argument` for planning errors,
   `permission_denied` for authz, `internal` with a generic message otherwise).
2. **Metadata RPCs are dishonest** — `server.rs:327-401`:
   `get_flight_info_catalogs/schemas/tables` build tickets with no authz and no
   session resolution, while the matching `do_get_*` handlers are not overridden
   (arrow-flight returns unimplemented). Either implement them end-to-end with a
   Cedar gate per listed object, or return `unimplemented` from the
   `get_flight_info_*` side too so the contract is honest. The honest-unimplemented
   option is fine for this PR.
3. **CPU runtime IO verification** — `crates/hydrofoil/src/execution.rs:60-63`:
   `CpuRuntime` is built with `enable_time()` but not `enable_io()`, yet
   `create_logical_plan` runs on it (`server.rs:494`, `http.rs:173`) and UC
   resolution performs HTTP metadata calls and credential vending. Object stores are
   built `with_io_runtime(Handle::current())`, but the UC REST client's runtime
   affinity is not obviously guaranteed. Verify with a test or a runtime assertion;
   if UC metadata calls can run inline on the CPU runtime, either enable IO there or
   hop UC resolution onto the main runtime.
4. **stdout noise** — `crates/hydrofoil/src/execution.rs:48-53` uses
   `print!`/`println!`/`eprintln!` in `CpuRuntime::drop`; switch to `tracing`.

## Tests to add (currently zero for server.rs)

Handler-level tests (in-process service or direct handler invocation):

- A session under a static deny policy: `get_flight_info_statement` must deny
  *without* returning a schema (B5 regression).
- `do_get` after a successful `get_flight_info` executes without re-authorization
  errors (plan-reuse path).
- A malformed/failing schema path returns `Status`, not a panic (B7).
- `do_put_statement_update` returns `unimplemented` (B8).
- Session resolution: same `x-session-id` reuses the session and its original
  principal; missing session id creates an ephemeral session.

## Constraints

- Coordinate with S01's changes to the same `SQLOptions` call sites; rebase on it if
  it has landed.
- Crates are unpublished: change APIs freely, no compatibility shims.
- Stage your changes and propose a commit message, but do **not** run `git commit` —
  the user signs commits. Attribute AI work as "AI assisted by Isaac" if attribution
  is included.

## Verification

- `cargo test -p hydrofoil`, `cargo clippy -p hydrofoil` clean.
- Manual check against the live stack if available (`environments/`, `just`
  targets): an unauthorized query is denied at `GetFlightInfo`; an authorized query
  still round-trips via Flight SQL (e.g. the `notebooks/duckdb_flight.py` flow).
