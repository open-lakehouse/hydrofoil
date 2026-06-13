# S02 — delta-rs: reject path-based commits on catalog-managed tables

| | |
|---|---|
| Target repo | `delta-rs` (fork; sibling checkout, e.g. `~/code/delta-rs`) |
| Follow-through | `open-lakehouse` (optional second part, needs that checkout too) |
| Depends on | — |
| Scope | One PR in delta-rs (+ small follow-up in open-lakehouse) |
| Findings | A1 (critical, source-verified) |

## Mission

You are working in a delta-rs fork used by the open-lakehouse stack. Unity Catalog
*catalog-managed* Delta tables (`catalogManaged` table feature) must have every commit
proposed to and ratified by the catalog — clients must never write
`_delta_log/<version>.json` directly. The catalog-coordinated commit path lives in
unitycatalog-rs (kernel `Committer` / `UCCommitter` on the buoyant delta-kernel fork);
delta-rs deliberately does **not** implement it. The bug: delta-rs currently allows its
legacy path-based commit machinery to run on catalog-managed tables anyway.

Protocol reference (download first):

```sh
mkdir -p /tmp/uc-specs
curl -sL https://raw.githubusercontent.com/unitycatalog/unitycatalog/main/spec/protocols/ManagedTablesSpec.md -o /tmp/uc-specs/ManagedTablesSpec.md
```

See "Write to the table" (~lines 72–76): every commit must be staged and proposed to
UC, which ratifies it; only ratified commits may appear as `_delta_log/<v>.json`.

## Finding to fix

### A1 [critical, source-verified] Path-based commit path open for catalog-managed tables

- `crates/core/src/kernel/transaction/protocol.rs:308-341` — the static
  `ProtocolChecker` allow-lists `TableFeature::CatalogManaged` (and `V2Checkpoint`,
  `VacuumProtocolCheck`) in **both** the reader and the writer feature sets. The
  in-code comment is explicit that catalog-coordinated commit resolution "is NOT
  implemented here and is owned by the catalog (Unity Catalog RS) side" — yet writer
  allow-listing means `CommitBuilder` will happily commit.
- `crates/core/src/delta_datafusion/table_provider/next/mod.rs:774-812` —
  `DeltaScan::insert_into` routes INSERTs into `DeltaDataSink`.
- `crates/core/src/delta_datafusion/table_provider/data_sink.rs:204-209` —
  `DeltaDataSink` commits via the legacy `CommitBuilder`, which PUTs
  `_delta_log/<v>.json` (put-if-absent) through the log store.

Consequence: an `INSERT INTO` a UC managed table through this provider publishes an
**unratified** commit. It can collide with or shadow a ratified-but-unpublished staged
commit at the same version, and it carries no in-commit timestamp (`CommitBuilder` has
no ICT handling; `inCommitTimestamp` is mandatory on managed tables). Against a real
UC deployment a READ-scoped credential usually makes the PUT fail; against a dev store
with broad credentials it succeeds and corrupts the table. In the open-lakehouse
deployment this is reachable from hydrofoil's Flight ingest
(`crates/hydrofoil/src/planner/flight.rs:48-54` → provider built in
`crates/hydrofoil/src/catalog/unity.rs:122-135`, log store at
`crates/hydrofoil/src/session/log_store.rs:78-107`).

**Fix (delta-rs):** keep `CatalogManaged` allow-listed for *reads*, but reject the
legacy commit path for catalog-managed tables. Concretely: in the pre-commit check
(`ProtocolChecker::can_commit`, or equivalently at the top of `CommitBuilder`'s build
and in `DeltaScan::insert_into`), error when
`table_configuration().is_catalog_managed()` and the log store is not a
catalog-committer-backed store. The error message should point writers at the
catalog-coordinated path (unitycatalog-rs `append_to_managed_table`). Choose the
narrowest seam that covers **all** legacy commit entry points (DeltaOps writes,
DeltaDataSink, merge/delete/update operations) — gating inside `CommitBuilder` /
`can_commit` covers them all; gating only `insert_into` does not.

**Follow-through (open-lakehouse, optional if checkout available):** route hydrofoil
managed-table inserts to the catalog-coordinated write
(`datafusion_unitycatalog::managed::append_to_managed_table` in unitycatalog-rs)
instead of `DeltaDataSink`, so legitimate INSERTs keep working once the gate lands. If
you cannot complete this here, ensure the delta-rs error message is actionable and
note the hydrofoil change as required follow-up in your summary.

## Constraints

- This is a fork the team controls; change behavior freely, but keep the diff minimal
  and well-commented — the comment at the allow-list already documents the intent,
  update it to describe the gate.
- Do not remove the read-side allow-listing (managed tables must stay readable).
- Stage your changes and propose a commit message, but do **not** run `git commit` —
  the user signs commits. Attribute AI work as "AI assisted by Isaac" if attribution
  is included.

## Verification

- New test: building a table whose protocol carries `catalogManaged` and attempting a
  commit through the legacy path (e.g. `DeltaOps::write` or `insert_into` with a plain
  log store) must fail with the new error; the same table must still open for read.
- `cargo test -p deltalake-core` (or the narrowest affected test targets) and
  `cargo clippy` clean on touched code.
- Confirm existing managed-table *read* tests still pass.
