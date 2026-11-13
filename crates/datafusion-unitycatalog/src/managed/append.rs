//! Append to, or overwrite, an existing Unity Catalog catalog-managed Delta table.
//!
//! Both write their data files once, through the kernel's write context, then commit
//! through [`commit_managed`](super::txn::commit_managed). Retries re-stage the same
//! files on the fresh snapshot, so a conflict never rewrites data. An overwrite also
//! re-derives its removes from that snapshot, so it removes exactly the files live at the
//! version it commits on top of.
//!
//! Post-commit work (publish, backfill, metrics) is best-effort; see [`super::txn`].

use std::sync::Arc;

use datafusion::arrow::array::RecordBatch;
use datafusion::arrow::datatypes::{DataType, Schema as ArrowSchema};
use delta_kernel::Version;
use delta_kernel::engine::arrow_conversion::TryIntoArrow;
use delta_kernel::engine::arrow_data::ArrowEngineData;
use delta_kernel::transaction::UpdateTableOperation;
use delta_kernel_default_engine::DefaultEngineBuilder;
use object_store::ObjectStore;
use tracing::warn;
use unitycatalog_client::DeltaV1Client;
use unitycatalog_object_store::{TableOperation, UnityObjectStoreFactory};

use super::create::CreateManagedTableError;
use super::txn::{
    CommitOptions, KernelTxnStage, ManagedEngine, ManagedTableRef, add_paths, commit_managed,
    delete_data_files, into_record_batch, remove_all_files,
};

/// How a write combines with the table's existing data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedWriteMode {
    /// Add the new rows.
    Append,
    /// Replace every row with the new ones.
    Overwrite,
}

/// Append `batch` as a new commit to the existing managed table `catalog.schema.table`.
///
/// Returns the committed table version. `engine_info` is recorded in the commit's
/// `commitInfo`. On success the ratified commit is published, backfilled, and its metrics
/// reported on a best-effort basis (a failure there does not fail the ratified write).
pub async fn append_to_managed_table(
    factory: Arc<UnityObjectStoreFactory>,
    catalog: &str,
    schema: &str,
    table: &str,
    batch: RecordBatch,
    engine_info: &str,
) -> Result<Version, CreateManagedTableError> {
    write_managed_table(
        factory,
        catalog,
        schema,
        table,
        vec![batch],
        ManagedWriteMode::Append,
        engine_info,
    )
    .await
}

/// Replace the contents of the managed table `catalog.schema.table` with `batch`.
pub async fn overwrite_managed_table(
    factory: Arc<UnityObjectStoreFactory>,
    catalog: &str,
    schema: &str,
    table: &str,
    batch: RecordBatch,
    engine_info: &str,
) -> Result<Version, CreateManagedTableError> {
    write_managed_table(
        factory,
        catalog,
        schema,
        table,
        vec![batch],
        ManagedWriteMode::Overwrite,
        engine_info,
    )
    .await
}

/// Write `batches` to the managed table `catalog.schema.table` in `mode`, vending
/// read-write credentials for it from `factory`.
pub async fn write_managed_table(
    factory: Arc<UnityObjectStoreFactory>,
    catalog: &str,
    schema: &str,
    table: &str,
    batches: Vec<RecordBatch>,
    mode: ManagedWriteMode,
    engine_info: &str,
) -> Result<Version, CreateManagedTableError> {
    let target = ManagedTableRef::new(
        Arc::new(factory.unity_client().delta_v1()),
        catalog,
        schema,
        table,
    );
    // The table exists, so vend by name.
    let uc_store = factory
        .for_table(
            format!("{catalog}.{schema}.{table}"),
            TableOperation::ReadWrite,
        )
        .await?;
    let store = uc_store.root();
    write_with_store(&target, store, batches, mode, engine_info).await
}

/// Write `batches` to the managed table `catalog.schema.table` through `client` and
/// `store`, for callers that manage credentials themselves.
///
/// `store` must be bucket-rooted (paths are full object keys, as the kernel default
/// engine expects) and allow writes under the table location.
#[allow(clippy::too_many_arguments)]
pub async fn write_managed_table_with_store(
    client: Arc<DeltaV1Client>,
    catalog: &str,
    schema: &str,
    table: &str,
    store: Arc<dyn ObjectStore>,
    batches: Vec<RecordBatch>,
    mode: ManagedWriteMode,
    engine_info: &str,
) -> Result<Version, CreateManagedTableError> {
    let target = ManagedTableRef::new(client, catalog, schema, table);
    write_with_store(&target, store, batches, mode, engine_info).await
}

/// Write `batches` to `target` through `store`, a bucket-rooted store with write access
/// to the table location.
pub(crate) async fn write_with_store(
    target: &ManagedTableRef,
    store: Arc<dyn ObjectStore>,
    batches: Vec<RecordBatch>,
    mode: ManagedWriteMode,
    engine_info: &str,
) -> Result<Version, CreateManagedTableError> {
    let engine: ManagedEngine = DefaultEngineBuilder::new(store.clone()).build();

    // Load once to derive the write context and write the data files.
    let first = target.load().await?;
    let snapshot = first.snapshot(&engine)?;
    let written_schema = snapshot.schema();
    let location = first.location.clone();
    let probe = snapshot
        .clone()
        .update_table(UpdateTableOperation::Write, engine_info, true)
        .build(&engine, Box::new(target.committer(&first.table_id)))?;
    if !probe.logical_partition_columns().is_empty() {
        return Err(CreateManagedTableError::other(format!(
            "writes to partitioned managed table '{}.{}.{}' are not yet supported",
            target.catalog, target.schema, target.table
        )));
    }
    let write_context = probe.write_state()?.write_context_builder().build()?;
    drop(probe);

    // The kernel writes whatever columns a batch has, so check them against the table
    // before anything reaches storage.
    let table_schema: ArrowSchema = written_schema
        .as_ref()
        .try_into_arrow()
        .map_err(|e| CreateManagedTableError::other(format!("table schema to arrow: {e}")))?;
    for batch in &batches {
        check_batch_schema(&table_schema, batch.schema_ref())?;
    }

    let num_rows: i64 = batches.iter().map(|b| b.num_rows() as i64).sum();
    let mut adds = Vec::with_capacity(batches.len());
    for batch in batches.into_iter().filter(|b| b.num_rows() > 0) {
        let add = engine
            .write_parquet(&ArrowEngineData::new(batch), &write_context)
            .await?;
        adds.push(into_record_batch(add)?);
    }
    if adds.is_empty() && mode == ManagedWriteMode::Append {
        // Nothing to commit. Report the current version.
        return Ok(snapshot.version());
    }
    let data_files = add_paths(&adds);

    let options = CommitOptions {
        operation: UpdateTableOperation::Write,
        engine_info,
        data_change: true,
        num_rows_inserted: num_rows,
    };
    let result = commit_managed(target, &engine, &store, first, options, |snapshot| {
        // The data files were written against `written_schema`. If another writer changed
        // the schema since, they no longer match it.
        if snapshot.schema() != written_schema {
            return Err(CreateManagedTableError::other(
                "the table schema changed during the write; retry the write",
            ));
        }
        let removes = match mode {
            ManagedWriteMode::Append => Vec::new(),
            ManagedWriteMode::Overwrite => remove_all_files(snapshot, &engine)?,
        };
        Ok(KernelTxnStage {
            adds: adds.clone(),
            removes,
        })
    })
    .await;

    match result {
        Ok(version) => Ok(version),
        Err(failure) => {
            if failure.outcome_unknown {
                warn!(
                    files = data_files.len(),
                    "managed write failed with an unknown outcome; keeping its data files"
                );
            } else {
                delete_data_files(&store, &location, &data_files).await;
            }
            Err(failure.error)
        }
    }
}

/// Reject a batch whose columns differ from the table's in name, order, or type. String
/// and binary view types match their plain counterparts (DataFusion reads Delta strings as
/// `Utf8View`; both encode the same Parquet column).
fn check_batch_schema(
    table: &ArrowSchema,
    batch: &ArrowSchema,
) -> Result<(), CreateManagedTableError> {
    fn normalize(dt: &DataType) -> DataType {
        match dt {
            DataType::Utf8View | DataType::LargeUtf8 => DataType::Utf8,
            DataType::BinaryView | DataType::LargeBinary => DataType::Binary,
            other => other.clone(),
        }
    }
    let matches = table.fields().len() == batch.fields().len()
        && table.fields().iter().zip(batch.fields()).all(|(t, b)| {
            t.name() == b.name() && normalize(t.data_type()) == normalize(b.data_type())
        });
    if matches {
        return Ok(());
    }
    let describe = |s: &ArrowSchema| {
        s.fields()
            .iter()
            .map(|f| format!("{}: {}", f.name(), f.data_type()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    Err(CreateManagedTableError::other(format!(
        "batch schema [{}] does not match the table schema [{}]",
        describe(batch),
        describe(table)
    )))
}

#[cfg(test)]
mod tests {
    use datafusion::arrow::datatypes::{Field, Schema};

    use super::*;

    fn schema(fields: &[(&str, DataType)]) -> ArrowSchema {
        Schema::new(
            fields
                .iter()
                .map(|(n, t)| Field::new(*n, t.clone(), true))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn batch_schema_must_match_table_columns() {
        let table = schema(&[("id", DataType::Int64), ("name", DataType::Utf8)]);
        assert!(check_batch_schema(&table, &table).is_ok());
        // View types read back by DataFusion are accepted.
        assert!(
            check_batch_schema(
                &table,
                &schema(&[("id", DataType::Int64), ("name", DataType::Utf8View)])
            )
            .is_ok()
        );
        assert!(check_batch_schema(&table, &schema(&[("other", DataType::Int64)])).is_err());
        assert!(
            check_batch_schema(
                &table,
                &schema(&[("name", DataType::Utf8), ("id", DataType::Int64)])
            )
            .is_err()
        );
        assert!(
            check_batch_schema(
                &table,
                &schema(&[("id", DataType::Int32), ("name", DataType::Utf8)])
            )
            .is_err()
        );
    }
}
