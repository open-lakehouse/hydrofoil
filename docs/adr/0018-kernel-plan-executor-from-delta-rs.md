# 0018 — Kernel declarative plans run on delta-rs's DataFusion executor

> Status: **Accepted** (2026-11). The executor changes live on the roeap/delta-rs
> `olai/kernel-executor-plans` branch, which the root `Cargo.toml` pins `deltalake-core` to.
> Guarded by `crates/datafusion-unitycatalog/tests/kernel_executor.rs`.

## Context

delta-kernel (`buoyant_kernel` 0.30.1 here) can hand data-heavy work to the engine as a
declarative plan, executed by the engine's `PlanExecutor` (`declarative-plans` feature). We
asked whether hydrofoil needs its own DataFusion-based executor for these plans.

It doesn't. One already exists, and it's already in our dependency graph:

- delta-rs has `crates/kernel-executor` (`deltalake-kernel-executor`, unpublished until
  delta-rs 2.0). It merges kernel's in-tree `datafusion-executor` with delta-rs's DataFusion
  engine code (delta-rs#4928, landed as #4938/#4951). The kernel maintainers then moved
  ownership to delta-rs: kernel#3167 ("DF now in delta-rs") and kernel#926 were closed
  2026-11-06. Kernel's own `datafusion-executor/` is an unpublished standalone workspace with
  no `PlanExecutor` impl, scans, or I/O.
- `deltalake_core::delta_datafusion::engine::DataFusionEngine`, which hydrofoil already
  uses (session, catalog schema, UC `catalog/delta.rs`), is that crate's engine.
  `Engine::plan_executor()` returns a `DataFusionPlanExecutor`, so every snapshot we load
  through it runs kernel's protocol + metadata replay as a DataFusion plan.

What kernel 0.30.x runs as plans: P&M replay (used automatically when the engine has an
executor), `Scan::declarative_metadata_scan_plan`, and checkpoint-shape reads. Commits, CRC,
table changes, checkpoint writes and parquet writes still go through the classic handlers.
The 0.28 → 0.30 bump added `ScopedPlanExecutor`/`RelationRef`/`RelationSource` and `WriteJson`,
the building blocks of kernel's declarative commits on `main`.

At the delta-rs rev we pinned before, the executor had these gaps:

- `RelationSource`, `WriteJson` and `DynamicScan` were rejected, and `ScopedPlanExecutor` was not
  implemented.
- `declarative_metadata_scan_plan` failed on every table. DataFusion 55's leaf-expression
  pushdown breaks on the executor's renaming projections (apache/datafusion#25446, fixed for
  56 by #25478). Its `CAST` also refuses to tighten nested nullability, which the scan key's
  `deletionVector` needs.
- An `unimplemented!` panic in expression conversion.

## Decision

Don't build a hydrofoil executor. Use delta-rs's, and do the work it needs in our delta-rs
fork, so it can be considered for upstreaming later:

- `fix(kernel-executor)`: error instead of panic for a non-junction operator.
- `feat(kernel-executor)`: `ScopedDataFusionPlanExecutor` (`DataFusionPlanExecutor::scoped`)
  with `RelationSource`. Also `WriteJson` (ordered inputs, schema check, atomic create or
  `FileAlreadyExists`) and `DynamicScan` (rejects rows with a deletion vector, as kernel's
  reference executor does).
- `fix(kernel-executor)`: the metadata scan plan runs. The executor's sessions leave out the two
  leaf-pushdown rules until DataFusion 56. Projections use a `declared_cast` UDF (Arrow's cast)
  where only nested nullability differs. A semi join's build side is narrowed to its keys.

`DefaultEngine` stays in `datafusion-unitycatalog`'s managed create/append path. Writes need
`write_parquet`, and the executor's writes are the classic handlers anyway.

## Consequences

- hydrofoil can run `declarative_metadata_scan_plan` on its DataFusion session today. The guard
  test checks that its live files match classic `scan_metadata` on a table with a checkpoint
  and an overwrite.
- delta-rs core still loads snapshots and commits through `log_store.engine()`, the
  `DefaultEngine`, so its own table provider doesn't use the plan path yet.
- Upstreaming candidates, all in `crates/kernel-executor`: the three commits above. The
  leaf-rule workaround is temporary either way.
- Revisit at the next kernel bump. Kernel `main` adds `PrefixSum`, the `FirstNonNull`
  aggregate and declarative commits. The executor's exhaustive aggregate match won't compile
  until `FirstNonNull` is lowered.
- Revisit at DataFusion 56: restore the leaf-pushdown rules (`LEAF_PUSHDOWN_RULES` in the
  executor's `query/mod.rs`).
- The wasm engine (`wasm/olai-delta-df`) is separate. It runs on the roeap
  `sm-plans-consolidation` kernel fork with DataFusion 54 and doesn't use the 0.30 plan IR.
  Converging it is its own decision.
