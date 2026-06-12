# S10 — open-lineage producer correctness

| | |
|---|---|
| Status | ✅ **Done** — commit `01f4c09` (2026-06-12), incl. all C9 minors + drain API |
| Target repo | `open-lakehouse` (crates/open-lineage) |
| Depends on | — |
| Scope | One PR |
| Findings | C1+C2 (critical), C4 (major), C5-partial (major), C9-partial (minor) |

## Mission

You are working in `open-lakehouse`. `crates/open-lineage` is a hand-written
OpenLineage producer for DataFusion: it extracts lineage from logical plans
(`src/extract.rs`), builds run events (`src/builder.rs`, facets in `src/facets.rs`),
wraps execution to emit START at execution begin and COMPLETE/FAIL at stream end
(`src/exec.rs` `OpenLineageExec`, planner wiring in `src/planner.rs`), and ships
events via a non-blocking client (`src/client.rs`, olai-http transport). Envelope
shape, `_producer`/`_schemaURL` on facets, UUIDv7 run ids, and the lifecycle test
suite are solid. This session fixes the event-content and lifecycle defects.

**Project decision (already made — implement, don't relitigate):** column-level
lineage is disabled until a sound, scope-aware extraction exists. The current
implementation is name-based and scope-blind, and the facet is attached to the wrong
datasets; emitting it is actively misleading to consumers.

## Findings to fix

### C1+C2 [critical] Disable the column-lineage facet; remove the unsound extraction

- `crates/open-lineage/src/builder.rs:98-124` — `input_dataset_facets` attaches
  `columnLineage` to every **input** dataset; outputs get `facets:
  Default::default()` (the OpenLineage spec defines `ColumnLineageDatasetFacet` on
  *outputs*, keyed by output fields — as emitted, consumers see no column lineage at
  all).
- `crates/open-lineage/src/extract.rs:103-119, 168-201` — extraction maps column
  qualifiers to datasets by name (`dataset_for(rel)`), so aliases/CTEs
  (`SubqueryAlias`) fabricate datasets that don't exist; unqualified column refs are
  silently skipped; the map is keyed by bare output-column name with top-down
  visitation, so deeper same-named projections overwrite the real top-level mapping;
  there is no transitive resolution through intermediate projections.

**Fix:** remove the column-lineage emission (and the name-based extraction feeding
it) so events carry **table-level lineage only**. The crates are unpublished — prefer
deletion over feature-gating dead code. Leave one design note (doc comment or
`docs/open-lineage-design.md` addendum) sketching the sound approach for the future
implementation: bottom-up (`f_up`) per-node maps of output column → set of physical
`(dataset, column)`, resolved through SubqueryAlias/Projection/Aggregate/Join, with
only the root map published, facet attached to outputs.

### C4 [major] Terminal events carry plan-time `eventTime`

- `crates/open-lineage/src/builder.rs:128` — `event_time: Utc::now()` at event
  *construction*; `src/planner.rs:80` builds the COMPLETE template during planning;
  `src/exec.rs:96-121` emits the cloned template at stream end without refreshing.
  Every run appears to take ~0 ms; duration computations are garbage.

**Fix:** set `event.event_time = Utc::now().to_rfc3339()` inside
`RunState::emit_terminal` (both COMPLETE and FAIL branches). Add a test asserting
terminal `eventTime` > START `eventTime`.

### C5-partial [major] Terminal-event loss on execute-error; partition-counter desync

1. `crates/open-lineage/src/exec.rs:333` —
   `self.inner.execute(partition, context)?` returns early on error without
   `record_error`/decrement; neither COMPLETE nor FAIL is ever emitted (run stuck
   RUNNING). Execute-time errors (object-store auth, credential vending) are common.
   Fix: on `Err`, record the error and emit/decrement before propagating.
2. `crates/open-lineage/src/exec.rs:252-276` — `remaining` is fixed from the original
   inner plan's partition count, but `with_new_inner` swaps children without
   updating it → premature or missing terminal events after partitioning-changing
   rewrites. Fix: recompute in `with_new_inner`, or initialize `remaining` lazily
   from `properties()` at first `execute()`.
3. `crates/open-lineage/src/exec.rs:259` — the `max(1)` zero-partition guard never
   fires (with zero partitions `execute` is never called); align behavior with the
   comment or fix the comment as part of (1)/(2).

### C9-partial [minor]

1. **Schema facet uses the projected scan schema** —
   `crates/open-lineage/src/extract.rs:89-98`: after projection pushdown,
   `SELECT a FROM t` reports `t` as having only column `a`, causing flapping schema
   versions. Use the full table schema (`scan.source.schema()`).
2. **Duplicate inputs not deduped** — `extract.rs:120-123`: self-joins emit the same
   dataset twice. Dedupe by `(namespace, name)`.
3. **`nominalTime` serde landmine** — `crates/open-lineage/src/facets.rs:46-47`:
   `RunFacets.nominal_time` lacks `#[serde(rename = "nominalTime")]` (unlike
   `errorMessage`). Latent (always `None` today) — add the rename. Do NOT "fix"
   `processing_engine`; snake_case is correct per spec there.
4. **`processing_engine.version`** — `crates/open-lineage/src/config.rs:30-31`
   reports this crate's version as the engine version; use the `datafusion` crate
   version for `engine_version`, keep crate version for `adapter_version`.
5. **No drain on shutdown** — `crates/open-lineage/src/client.rs:37-50, 102-110`:
   bounded `try_send` queue (correct: query path never blocks), but there is no API
   to flush; process exit loses queued terminal events. Add an async
   `shutdown()`/`drain()` to `OpenLineageClient` (S11 wires it into hydrofoil). Add a
   dropped-events counter (queue-full and transport-failure paths) exposed via
   `tracing` at minimum.
6. **`as_any` contract violation** — `crates/open-lineage/src/exec.rs:301-304`
   returns the *inner* plan's `as_any`, so visitors can downcast the wrapper to the
   inner type and then mutate children with wrong assumptions, silently dropping the
   lineage node. Return `self`.
7. **`write_count` sniffing guard** — `exec.rs:406-419` treats any single-`UInt64`
   `count` column stream as a write count; it's gated today only by
   `outputs`-non-empty at `exec.rs:132-134`. Make that coupling explicit (only sniff
   when `complete.outputs` is non-empty) with a comment.
8. **Constructor panic outside Tokio** — `client.rs:39` (`tokio::spawn` in a sync
   constructor). Use `Handle::try_current()` with a clear error, or document the
   runtime requirement on the constructor.

## Constraints

- Crates are unpublished: prefer removing the unsound code over gating it.
- The existing lifecycle test suite (COMPLETE deferred to stream end, FAIL on
  mid-stream errors and dropped streams) must keep passing.
- Stage your changes and propose a commit message, but do **not** run `git commit` —
  the user signs commits. Attribute AI work as "AI assisted by Isaac" if attribution
  is included.

## Verification

- `cargo test -p open-lineage` with new tests: terminal eventTime ordering;
  execute()-error emits FAIL exactly once; partition-count change via
  `with_new_children` still emits exactly one terminal event; schema facet equals
  full table schema under projection; self-join dedup; no `columnLineage` key in
  emitted events.
- `cargo clippy -p open-lineage` clean; hydrofoil still builds
  (`cargo build -p hydrofoil`).
