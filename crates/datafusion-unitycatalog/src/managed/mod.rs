//! Unity Catalog catalog-managed Delta table support: create + commit through the
//! kernel's [`Committer`](delta_kernel::committer::Committer) framework, backed by the
//! unitycatalog-rs [`DeltaV1Client`](unitycatalog_client::DeltaV1Client).
//!
//! - [`UnityCatalogCommitter`] — the catalog-managed committer (v0 publish, v≥1 stage + ratify).
//! - [`create_managed_table`] — staging → `kernel::create_table` (writes `0.json`) → `createTable`.
//! - [`append_to_managed_table`] / [`overwrite_managed_table`] — write data files once,
//!   then commit (v≥1) through the shared commit layer (`txn`): bounded
//!   conflict/throttle/ambiguity retry, then best-effort publish + backfill + metrics
//!   (ManagedTablesSpec §"Write to the table").
//! - [`ManagedDeltaTableProvider`] — a DataFusion provider for a managed table: reads through
//!   delta-rs, `INSERT INTO` / `INSERT OVERWRITE` through [`ManagedTableSink`].
//!
//! Design + rationale: see open-lakehouse `docs/adr/0010-catalog-managed-table-writes.md`.
//!
//! ## Upstream / follow-ups (tracked here so they travel with the code)
//!
//! - **delta-rs read path**: upstream delta-rs `main` (2.0, delta-rs#4809) reads catalog-managed
//!   tables — `Snapshot::try_new_with_engine_and_catalog` takes our UC log tail as a
//!   `CatalogLogTail`, and `CatalogManaged` / `VacuumProtocolCheck` are supported reader
//!   features. The `roeap/delta-rs` `olai/catalog-managed-2f01b956` pin only re-exports
//!   `to_delta_predicate` publicly (log explorer pushdown); drop the fork once 2.0 ships with it.
//! - **delta-rs write path**: delta-rs can't write catalog-managed tables yet (no ICT writes,
//!   `CatalogManaged` is not a writer feature, no catalog implementation of its
//!   `logstore::Committer` trait — delta-rs#4549). Until it does, creates and writes go
//!   through the kernel `Transaction` + [`UnityCatalogCommitter`] here.
//! - **Declarative commits**: `txn::stage_imperative` is the only caller of kernel staging
//!   APIs. When kernel ships `Transaction::commit_declarative`, a `stage_declarative`
//!   registers the same batches as plan relations; see the `txn` module docs.
//! - **Schema/property evolution on append**: ALTER-style commits (protocol/metadata/clustering
//!   changes) are rejected, not propagated — see `UnityCatalogCommitter`'s
//!   `validate_no_alter_table_changes`. Lifting this requires bundling
//!   `set-columns`/`set-properties`/`set-protocol` actions into the same `updateTable` call as
//!   `add-commit`.
//! - **Upstream home**: keep this glue dependency-light so it can move toward
//!   `delta-rs/crates/catalog-unity`, or adopt the buoyant `delta-kernel-unity-catalog` /
//!   `unity-catalog-delta-rest-client` crates once they stabilize.

mod append;
mod committer;
mod create;
mod sink;
#[cfg(test)]
mod test_support;
mod txn;

pub use append::{
    ManagedWriteMode, append_to_managed_table, overwrite_managed_table, write_managed_table,
    write_managed_table_with_store,
};
pub use committer::UnityCatalogCommitter;
pub use create::{
    CreateManagedTableError, ManagedTable, build_staging_store, create_managed_table,
    get_final_required_properties_for_uc, get_required_properties_for_disk,
};
pub use sink::{ManagedDeltaTableProvider, ManagedTableSink};
