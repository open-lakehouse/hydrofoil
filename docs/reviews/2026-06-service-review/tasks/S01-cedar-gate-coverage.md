# S01 — Cedar gate coverage: subqueries, unmodeled nodes, statements

| | |
|---|---|
| Target repo | `open-lakehouse` (crates/datafusion-cedar, crates/hydrofoil) |
| Depends on | — |
| Scope | One PR |
| Findings | B1 (critical, source-verified), B6 (major), B10-partial (minor) |

## Mission

You are working in the `open-lakehouse` repository. Hydrofoil is a DataFusion-based
query server exposing a Flight SQL API; `crates/datafusion-cedar` evaluates Cedar
policies against the logical plan before execution (coarse allow/deny in
`LakehouseSession::create_physical_plan`, plus row-filter/column-mask governance via
Cedar partial evaluation). The engine is designed default-deny and fail-closed. This
session closes three holes where plan shapes escape policy evaluation entirely.

## Findings to fix

### B1 [critical, source-verified] Subquery scans bypass authorization AND governance

- `crates/datafusion-cedar/src/visitor.rs:281` — `authorize_plan` calls
  `plan.visit(&mut visitor)`.
- `crates/datafusion-cedar/src/govern.rs:73` — `govern_plan`'s `TableCollector` calls
  `plan.visit(&mut collector)`.

DataFusion's `TreeNode::apply_children` for `LogicalPlan` walks `self.inputs()` only —
it does **not** descend into subquery *expressions* (`Expr::ScalarSubquery`,
`InSubquery`, `Exists`). The optimizer decorrelates many subqueries into joins, but
correlated scalar subqueries in projection lists (and other non-decorrelatable forms)
survive as subquery expressions. A table read only through such a subquery produces no
`read_table` Cedar request (so the coarse gate allows the query) and no `TablePolicy`
(so its row filters / column masks are never injected). Example bypass:
`SELECT (SELECT secret FROM protected.t LIMIT 1)`.

Contrast: the DDL/DML guard (`verify_plan` calls at
`crates/hydrofoil/src/server.rs:248` and `:504`) already uses
`visit_with_subqueries` — the two passes disagree on tree coverage.

**Fix:** change both `plan.visit(...)` calls to `plan.visit_with_subqueries(...)`.
Add a regression test: a correlated scalar subquery over a protected table must be
denied by the coarse gate, and (separately) must receive the table's row filter when a
filtering policy applies.

### B6 [major] Unmodeled plan nodes default to allow; `Statement` nodes ungated

- `crates/datafusion-cedar/src/visitor.rs:125` — `AuthorizationVisitor::f_down`'s
  catch-all arm is `_ => {}`: any node other than `TableScan`/`Ddl`/`Dml`/recognized
  `Extension` generates no Cedar request, so the gate allows it.
- `LogicalPlan::Statement` (`SET VARIABLE`, `PREPARE`, …) is unmodeled, and all three
  `SQLOptions` guards only set `.with_allow_ddl(false).with_allow_dml(false)` —
  `allow_statements` defaults to `true` (confirmed for DataFusion 53). A
  `SET datafusion.…` therefore executes with no policy evaluation. Guard sites:
  `crates/hydrofoil/src/server.rs:244`, `crates/hydrofoil/src/server.rs:501`,
  `crates/hydrofoil/src/http.rs:180`.

**Fix:** add `.with_allow_statements(false)` to all three `SQLOptions` builders. In
the visitor, treat unmodeled *state-changing* top-level nodes (`Statement`, `Copy`)
like the existing `DdlStatement` `other =>` arm — emit a `DenyUnsupported` action
(fail-closed) rather than silently allowing. Pure relational nodes (projections,
joins, …) staying request-free is correct — their `TableScan` leaves carry the
authorization.

### B10-partial [minor] `||` residuals don't fold `true` guards

- `crates/datafusion-cedar/src/translate.rs:111` — `&&` goes through `translate_and`
  (which folds literal `true` guards); `||` uses the generic `binary`. A residual of
  shape `true || resource.x == …` translates to `lit(true).or(col("x").eq(…))` =
  always-true, silently dropping the row filter.

**Fix:** apply the same fold logic to `||`: a literal `true` operand makes the
disjunction always-true (this is then an *intentional* allow, fine); a literal `false`
operand should drop to the other side. Add unit tests mirroring the existing
`translate_and` fold tests.

### B10-partial [minor] Ingest pseudo-table triggers a spurious `read_table` request

- `crates/hydrofoil/src/planner/flight.rs:45` — `plan_ingest` builds
  `LogicalPlanBuilder::scan("input", …)` for the Flight DoPut data stream;
  `crates/datafusion-cedar/src/visitor.rs:74` lowers that scan to `read_table` on
  `Table::"input"`. A policy that doesn't permit reading `input` wrongly denies
  legitimate ingest; conversely a broad `read_table` permit becomes load-bearing for
  writes.

**Fix:** mark the streaming source so the visitor skips it — e.g. a reserved
namespace/marker on the scan that the visitor recognizes, or skip scans whose
provider is the ingest stream source. Keep the *write* authorization for the target
table intact.

## Constraints

- Fix inside `datafusion-cedar`/`hydrofoil` at the layer named per finding; no
  workarounds at call sites.
- Crates are unpublished: change APIs freely, do not add compatibility shims.
- Match surrounding code style and comment density.
- Stage your changes (`git add`) and propose a commit message, but do **not** run
  `git commit` — the user signs commits. Attribute AI work as "AI assisted by Isaac"
  if attribution is included.

## Verification

- `cargo test -p datafusion-cedar -p hydrofoil` (workspace root of open-lakehouse).
- New tests required: (1) correlated-scalar-subquery denial + row-filter injection,
  (2) `SET` statement denied through all three guard sites (at minimum the session
  path), (3) `||` residual fold cases, (4) ingest plan no longer emits a
  `read_table("input")` request.
- `cargo clippy --workspace` clean for the touched crates.
