//! Apache Iceberg sink for the lineage events table.
//!
//! Writes Arrow `RecordBatch`es as Parquet data files into an Iceberg table,
//! commits them via a `fast_append` transaction, and creates the namespace +
//! table on first append if they do not yet exist.
//!
//! Production builds talk to an Iceberg REST catalog (e.g. Lakekeeper,
//! Polaris, Tabular). Tests use the in-process `MemoryCatalog` so we can
//! exercise the full create-and-append path without spinning up a server.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use deltalake::arrow::array::{Array, ArrayRef, BooleanArray, RecordBatch, StringArray};
use deltalake::arrow::compute::cast as arrow_cast;
use deltalake::arrow::compute::filter_record_batch;
use iceberg::spec::{
    DataFile, DataFileFormat, Literal, PartitionKey, Struct, Transform, UnboundPartitionSpec,
};
use iceberg::transaction::{ApplyTransactionAction, Transaction};
use iceberg::writer::base_writer::data_file_writer::DataFileWriterBuilder;
use iceberg::writer::file_writer::ParquetWriterBuilder;
use iceberg::writer::file_writer::location_generator::{
    DefaultFileNameGenerator, DefaultLocationGenerator,
};
use iceberg::writer::{IcebergWriter, IcebergWriterBuilder};
use iceberg::{Catalog, CatalogBuilder, ErrorKind, NamespaceIdent, TableCreation, TableIdent};
use iceberg_catalog_rest::{
    REST_CATALOG_PROP_URI, REST_CATALOG_PROP_WAREHOUSE, RestCatalogBuilder,
};
use parquet::file::properties::WriterProperties;

use crate::config::IcebergConfig;
use crate::writer::schema::{arrow_schema_with_field_ids, iceberg_schema};
use crate::writer::sink::{SinkError, TableSink};

/// Iceberg sink. Owns an `Arc<dyn Catalog>` plus the namespace/table
/// coordinates and partition layout it should use when creating the table.
pub struct IcebergSink {
    catalog: Arc<dyn Catalog>,
    namespace: NamespaceIdent,
    table: TableIdent,
    partition_cols: Vec<String>,
}

impl IcebergSink {
    /// Build an `IcebergSink` backed by an Iceberg REST catalog (Lakekeeper,
    /// Polaris, Tabular, ...). The catalog is loaded eagerly so any
    /// connectivity / configuration error surfaces at startup.
    pub async fn from_config(cfg: &IcebergConfig) -> Result<Self, IcebergSinkError> {
        let catalog = RestCatalogBuilder::default()
            .load("rest", build_rest_props(cfg))
            .await
            .map_err(IcebergSinkError::from)?;
        Self::with_catalog(
            Arc::new(catalog) as Arc<dyn Catalog>,
            cfg.namespace.clone(),
            cfg.table.clone(),
            cfg.partition_cols.clone(),
        )
    }

    /// Build an `IcebergSink` from an already-constructed catalog. This is
    /// the seam we use in tests with `MemoryCatalog`, but it is also the
    /// supported way to plug a custom or pre-warmed catalog into the sink.
    pub fn with_catalog(
        catalog: Arc<dyn Catalog>,
        namespace: String,
        table: String,
        partition_cols: Vec<String>,
    ) -> Result<Self, IcebergSinkError> {
        let namespace = NamespaceIdent::new(namespace);
        let table = TableIdent::new(namespace.clone(), table);
        Ok(Self {
            catalog,
            namespace,
            table,
            partition_cols,
        })
    }

    async fn ensure_table(&self) -> Result<iceberg::table::Table, IcebergSinkError> {
        // Try to load the table directly. We accept three flavours of
        // "doesn't exist yet" and fall through to create on any of them:
        //   1. `ErrorKind::TableNotFound` / `NamespaceNotFound` — what
        //      MemoryCatalog (and a fully-conformant REST impl) reports.
        //   2. `ErrorKind::Unexpected` whose message contains
        //      "does not exist" / "NoSuchTable" / "NoSuchNamespace" — what
        //      iceberg-rust 0.7's REST client returns when Lakekeeper
        //      replies 404 with a `NoSuchTableException` body. This is a
        //      known iceberg-rust quirk; without this branch every first
        //      write against Lakekeeper fails with
        //      `iceberg: Unexpected => Tried to load a table that does not exist`.
        match self.catalog.load_table(&self.table).await {
            Ok(t) => return Ok(t),
            Err(e) if is_not_found(&e) => {}
            Err(e) => return Err(IcebergSinkError::from(e)),
        }

        // Best-effort namespace create. Ignore the "already exists" case so
        // two concurrent first-time writers can race without one of them
        // failing the RPC.
        if let Err(e) = self
            .catalog
            .create_namespace(&self.namespace, HashMap::new())
            .await
            && e.kind() != ErrorKind::NamespaceAlreadyExists
        {
            return Err(IcebergSinkError::from(e));
        }

        let schema = iceberg_schema().map_err(IcebergSinkError::other)?;

        // typed-builder's `partition_spec(...)` advances the type state, so
        // we have to call `.build()` inside each branch rather than mutate a
        // single binding.
        let creation = if self.partition_cols.is_empty() {
            TableCreation::builder()
                .name(self.table.name().to_string())
                .schema(schema.clone())
                .properties(HashMap::new())
                .build()
        } else {
            let mut spec_builder = UnboundPartitionSpec::builder().with_spec_id(0);
            for col in &self.partition_cols {
                let source_id = schema.field_id_by_name(col).ok_or_else(|| {
                    IcebergSinkError::other(format!(
                        "iceberg partition column '{col}' is not present in arrow_schema()",
                    ))
                })?;
                spec_builder = spec_builder
                    .add_partition_field(source_id, col.clone(), Transform::Identity)
                    .map_err(IcebergSinkError::from)?;
            }
            TableCreation::builder()
                .name(self.table.name().to_string())
                .schema(schema.clone())
                .partition_spec(spec_builder.build())
                .properties(HashMap::new())
                .build()
        };

        match self.catalog.create_table(&self.namespace, creation).await {
            Ok(t) => Ok(t),
            // Lost the create race with another writer — load whatever the
            // winner produced and use that. Same Unexpected/text-based
            // tolerance as `is_not_found` because Lakekeeper REST surfaces
            // 409 TableAlreadyExists as `Unexpected` too.
            Err(e) if is_already_exists(&e) => self
                .catalog
                .load_table(&self.table)
                .await
                .map_err(IcebergSinkError::from),
            Err(e) => Err(IcebergSinkError::from(e)),
        }
    }

    async fn append_inner(&self, batch: RecordBatch) -> Result<(), IcebergSinkError> {
        if batch.num_rows() == 0 {
            return Ok(());
        }

        let table = self.ensure_table().await?;

        // Iceberg's ParquetWriter enforces that every Arrow field carries a
        // `PARQUET:field_id` metadata entry that maps back to the iceberg
        // schema's field id. `arrow_schema_with_field_ids()` is just our
        // canonical Arrow schema with that metadata attached.
        //
        // We additionally cast column types to match what Iceberg expects
        // internally — the only real divergence today is the timestamp
        // timezone string: Delta uses `"UTC"` while Iceberg's
        // `Timestamptz` materializes back to `"+00:00"` per
        // `iceberg::arrow::UTC_TIME_ZONE`. The cast for that case is just a
        // metadata change on the column, not a data rewrite.
        let tagged_schema = arrow_schema_with_field_ids().map_err(IcebergSinkError::other)?;
        let coerced_columns: Result<Vec<ArrayRef>, IcebergSinkError> = batch
            .columns()
            .iter()
            .zip(tagged_schema.fields().iter())
            .map(|(col, field)| {
                if col.data_type() == field.data_type() {
                    Ok(col.clone())
                } else {
                    arrow_cast(col, field.data_type()).map_err(|e| {
                        IcebergSinkError::other(format!(
                            "failed to cast column '{}' from {:?} to {:?}: {e}",
                            field.name(),
                            col.data_type(),
                            field.data_type(),
                        ))
                    })
                }
            })
            .collect();
        let coerced_batch = RecordBatch::try_new(tagged_schema, coerced_columns?).map_err(|e| {
            IcebergSinkError::other(format!(
                "failed to rebind RecordBatch onto field-id-tagged schema: {e}"
            ))
        })?;

        // Group rows by partition value (or treat the whole batch as a
        // single group when the table is unpartitioned). Each group becomes
        // its own Parquet data file with the matching `PartitionKey`.
        let groups = self.split_by_partition(&table, &coerced_batch)?;

        // Use a per-call nanos suffix on the filename so subsequent appends
        // don't collide with files committed by earlier appends —
        // `DefaultFileNameGenerator` only auto-uniques *within* one writer
        // instance.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);

        let mut all_data_files: Vec<DataFile> = Vec::new();
        for (idx, (partition_struct, group_batch)) in groups.into_iter().enumerate() {
            let suffix = format!("{nanos}-{idx}");
            let files = self
                .write_one_data_file(&table, partition_struct, group_batch, suffix)
                .await?;
            all_data_files.extend(files);
        }

        if all_data_files.is_empty() {
            return Ok(());
        }

        let tx = Transaction::new(&table);
        let action = tx.fast_append().add_data_files(all_data_files);
        let tx = action.apply(tx).map_err(IcebergSinkError::from)?;
        tx.commit(self.catalog.as_ref())
            .await
            .map_err(IcebergSinkError::from)?;
        Ok(())
    }

    /// Build a single Parquet data file for `batch`, optionally pinned to a
    /// specific `Struct` of partition values for partitioned tables. Returns
    /// the resulting `DataFile` handles ready to be added to a `fast_append`
    /// transaction.
    ///
    /// `partition_struct = None` is used for unpartitioned tables;
    /// `Some(...)` is used for partitioned tables where the caller has
    /// already grouped rows by partition value.
    async fn write_one_data_file(
        &self,
        table: &iceberg::table::Table,
        partition_struct: Option<Struct>,
        batch: RecordBatch,
        file_suffix: String,
    ) -> Result<Vec<DataFile>, IcebergSinkError> {
        let location_generator = DefaultLocationGenerator::new(table.metadata().clone())
            .map_err(IcebergSinkError::from)?;
        let file_name_generator = DefaultFileNameGenerator::new(
            self.table.name().to_string(),
            Some(file_suffix),
            DataFileFormat::Parquet,
        );

        let partition_spec = table.metadata().default_partition_spec();
        let partition_key = partition_struct.clone().map(|s| {
            PartitionKey::new(
                partition_spec.as_ref().clone(),
                table.metadata().current_schema().clone(),
                s,
            )
        });
        let parquet_builder = ParquetWriterBuilder::new(
            WriterProperties::default(),
            table.metadata().current_schema().clone(),
            partition_key,
            table.file_io().clone(),
            location_generator,
            file_name_generator,
        );
        let data_file_builder =
            DataFileWriterBuilder::new(parquet_builder, partition_struct, partition_spec.spec_id());
        let mut writer = data_file_builder
            .build()
            .await
            .map_err(IcebergSinkError::from)?;
        writer.write(batch).await.map_err(IcebergSinkError::from)?;
        writer.close().await.map_err(IcebergSinkError::from)
    }

    /// Split `batch` into one sub-batch per distinct partition value.
    ///
    /// v1 limitations: partitioning is supported only when every field in
    /// the table's `default_partition_spec` is an `Identity` transform on a
    /// `Utf8` source column. This covers the lineage events table's natural
    /// partitioning on `event_kind`. For unpartitioned tables, the entire
    /// `batch` is returned as a single group with `partition_struct = None`.
    fn split_by_partition(
        &self,
        table: &iceberg::table::Table,
        batch: &RecordBatch,
    ) -> Result<Vec<(Option<Struct>, RecordBatch)>, IcebergSinkError> {
        let spec = table.metadata().default_partition_spec();
        let part_fields = spec.fields();
        if part_fields.is_empty() {
            return Ok(vec![(None, batch.clone())]);
        }

        // For each partition field, locate the source column index in the
        // batch and verify it's a Utf8 string. The partition layout is fixed
        // at table creation, so this loop runs once per write.
        let schema = table.metadata().current_schema().clone();
        let mut col_indices: Vec<usize> = Vec::with_capacity(part_fields.len());
        for pf in part_fields {
            if pf.transform != Transform::Identity {
                return Err(IcebergSinkError::other(format!(
                    "iceberg sink only supports Identity-transform partition fields in v1, got {:?} for '{}'",
                    pf.transform, pf.name,
                )));
            }
            let source = schema.field_by_id(pf.source_id).ok_or_else(|| {
                IcebergSinkError::other(format!(
                    "partition field '{}' references unknown source id {}",
                    pf.name, pf.source_id,
                ))
            })?;
            let idx = batch.schema().index_of(source.name.as_str()).map_err(|e| {
                IcebergSinkError::other(format!(
                    "partition source column '{}' not found in batch: {e}",
                    source.name,
                ))
            })?;
            let col = batch.column(idx);
            if col.as_any().downcast_ref::<StringArray>().is_none() {
                return Err(IcebergSinkError::other(format!(
                    "iceberg sink only supports string-typed Identity partitions in v1, '{}' is {:?}",
                    source.name,
                    col.data_type(),
                )));
            }
            col_indices.push(idx);
        }

        // Build a row-keyed map of partition values → row indices. Keys are
        // `Vec<Option<String>>` so null partition values are first-class.
        let n_rows = batch.num_rows();
        let mut groups: HashMap<Vec<Option<String>>, Vec<usize>> = HashMap::new();
        for row in 0..n_rows {
            let mut key = Vec::with_capacity(col_indices.len());
            for &idx in &col_indices {
                let arr = batch
                    .column(idx)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .expect("checked above");
                key.push(if arr.is_null(row) {
                    None
                } else {
                    Some(arr.value(row).to_string())
                });
            }
            groups.entry(key).or_default().push(row);
        }

        let mut out = Vec::with_capacity(groups.len());
        for (key_vals, row_ids) in groups {
            let mut mask = vec![false; n_rows];
            for r in &row_ids {
                mask[*r] = true;
            }
            let filter_arr = BooleanArray::from(mask);
            let filtered = filter_record_batch(batch, &filter_arr)
                .map_err(|e| IcebergSinkError::other(e.to_string()))?;
            let literals: Vec<Option<Literal>> = key_vals
                .into_iter()
                .map(|v| v.map(Literal::string))
                .collect();
            let part_struct = Struct::from_iter(literals);
            out.push((Some(part_struct), filtered));
        }
        Ok(out)
    }
}

#[async_trait]
impl TableSink for IcebergSink {
    fn name(&self) -> &'static str {
        "iceberg"
    }

    async fn append(&self, batch: RecordBatch) -> Result<(), SinkError> {
        self.append_inner(batch)
            .await
            .map_err(|e| SinkError::Iceberg(e.to_string()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IcebergSinkError {
    #[error("iceberg: {0}")]
    Iceberg(#[from] iceberg::Error),
    #[error("iceberg sink: {0}")]
    Other(String),
}

impl IcebergSinkError {
    fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}

/// Returns true if `e` represents a "table or namespace does not exist"
/// condition. Handles both the in-process `MemoryCatalog` (which reports
/// `TableNotFound`/`NamespaceNotFound` cleanly) and `iceberg-catalog-rest`
/// 0.7's quirk of bubbling 404s up as `ErrorKind::Unexpected` with the
/// upstream message embedded.
fn is_not_found(e: &iceberg::Error) -> bool {
    if matches!(
        e.kind(),
        ErrorKind::TableNotFound | ErrorKind::NamespaceNotFound
    ) {
        return true;
    }
    if e.kind() == ErrorKind::Unexpected {
        let msg = e.to_string();
        return msg.contains("does not exist")
            || msg.contains("NoSuchTable")
            || msg.contains("NoSuchNamespace");
    }
    false
}

/// Returns true if `e` represents a "already exists" condition, used to
/// detect that another writer won the create-table race. Mirrors
/// `is_not_found` with the analogous Lakekeeper REST text fallback.
fn is_already_exists(e: &iceberg::Error) -> bool {
    if e.kind() == ErrorKind::TableAlreadyExists {
        return true;
    }
    if e.kind() == ErrorKind::Unexpected {
        let msg = e.to_string();
        return msg.contains("already exists") || msg.contains("AlreadyExists");
    }
    false
}

/// Build the REST catalog property map from an [`IcebergConfig`].
///
/// Pulled out of `from_config` so the property wiring (URI, warehouse, and the
/// optional bearer `token`) can be asserted in a unit test without standing up
/// a real REST catalog.
pub(crate) fn build_rest_props(cfg: &IcebergConfig) -> HashMap<String, String> {
    let mut props = HashMap::new();
    props.insert(REST_CATALOG_PROP_URI.to_string(), cfg.catalog_uri.clone());
    props.insert(
        REST_CATALOG_PROP_WAREHOUSE.to_string(),
        cfg.warehouse.clone(),
    );
    if let Some(token) = cfg.token.as_ref() {
        // Standard Iceberg REST property name; the Lakekeeper REST catalog
        // honours it as a Bearer auth token. Covered by `build_rest_props_*`
        // unit tests; exercised against a live Lakekeeper by the `#[ignore]`d
        // `iceberg_integration_test::auth_*` tests.
        props.insert("token".to_string(), token.clone());
    }
    props
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::schema::arrow_schema;
    use deltalake::arrow::array::{StringArray, TimestampMicrosecondArray};
    use iceberg::memory::{MEMORY_CATALOG_WAREHOUSE, MemoryCatalogBuilder};
    use std::sync::Arc as StdArc;
    use tempfile::tempdir;

    /// Spin up an in-process `MemoryCatalog` rooted at `warehouse_uri`
    /// (e.g. `file:///tmp/warehouse`) and wrap it in an `IcebergSink`.
    /// Test-only — production callers go through `IcebergSink::from_config`.
    async fn sink_for_memory_catalog(
        warehouse_uri: String,
        namespace: &str,
        table: &str,
        partition_cols: Vec<String>,
    ) -> IcebergSink {
        let cat = MemoryCatalogBuilder::default()
            .load(
                "memory",
                HashMap::from([(MEMORY_CATALOG_WAREHOUSE.to_string(), warehouse_uri)]),
            )
            .await
            .unwrap();
        IcebergSink::with_catalog(
            Arc::new(cat) as Arc<dyn Catalog>,
            namespace.to_string(),
            table.to_string(),
            partition_cols,
        )
        .unwrap()
    }

    fn one_row_batch(kind: &str) -> RecordBatch {
        let schema = arrow_schema();
        RecordBatch::try_new(
            schema,
            vec![
                StdArc::new(StringArray::from(vec![kind])),
                StdArc::new(StringArray::from(vec![Some("START")])),
                StdArc::new(
                    TimestampMicrosecondArray::from(vec![1_700_000_000_000_000i64])
                        .with_timezone("UTC"),
                ),
                StdArc::new(StringArray::from(vec!["test-producer"])),
                StdArc::new(StringArray::from(vec![None::<&str>])),
                StdArc::new(StringArray::from(vec![Some("run-1")])),
                StdArc::new(StringArray::from(vec![Some("ns")])),
                StdArc::new(StringArray::from(vec![Some("job1")])),
                StdArc::new(StringArray::from(vec![None::<&str>])),
                StdArc::new(StringArray::from(vec![None::<&str>])),
                StdArc::new(StringArray::from(vec![None::<&str>])),
                StdArc::new(StringArray::from(vec![None::<&str>])),
                StdArc::new(StringArray::from(vec![None::<&str>])),
                StdArc::new(StringArray::from(vec![None::<&str>])),
                StdArc::new(StringArray::from(vec![Some("{}")])),
            ],
        )
        .unwrap()
    }

    fn warehouse_uri(dir: &std::path::Path) -> String {
        format!("file://{}", dir.display())
    }

    /// Build an `n`-row batch where row `i` has `event_kind = kinds[i]`.
    /// Used to exercise the partition-fanout path of `split_by_partition`.
    fn mixed_kind_batch(kinds: &[&str]) -> RecordBatch {
        let n = kinds.len();
        let nulls: Vec<Option<&str>> = vec![None; n];
        let schema = arrow_schema();
        let kind_col = StringArray::from(kinds.to_vec());
        let etype_col = StringArray::from(vec![Some("START"); n]);
        let etime_col = TimestampMicrosecondArray::from(
            (0..n as i64)
                .map(|i| 1_700_000_000_000_000 + i)
                .collect::<Vec<_>>(),
        )
        .with_timezone("UTC");
        let producer_col = StringArray::from(vec!["test-producer"; n]);
        let raw_col = StringArray::from(vec![Some("{}"); n]);

        RecordBatch::try_new(
            schema,
            vec![
                StdArc::new(kind_col),
                StdArc::new(etype_col),
                StdArc::new(etime_col),
                StdArc::new(producer_col),
                StdArc::new(StringArray::from(nulls.clone())),
                StdArc::new(StringArray::from(nulls.clone())),
                StdArc::new(StringArray::from(nulls.clone())),
                StdArc::new(StringArray::from(nulls.clone())),
                StdArc::new(StringArray::from(nulls.clone())),
                StdArc::new(StringArray::from(nulls.clone())),
                StdArc::new(StringArray::from(nulls.clone())),
                StdArc::new(StringArray::from(nulls.clone())),
                StdArc::new(StringArray::from(nulls.clone())),
                StdArc::new(StringArray::from(nulls)),
                StdArc::new(raw_col),
            ],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn empty_batch_is_a_noop() {
        let tmp = tempdir().unwrap();
        let sink =
            sink_for_memory_catalog(warehouse_uri(tmp.path()), "lineage", "events", vec![]).await;

        let empty = RecordBatch::new_empty(arrow_schema());
        sink.append(empty).await.unwrap();

        // No namespace and no table should have been provisioned. With
        // MemoryCatalog `table_exists` errors when the namespace is missing,
        // so probe via `namespace_exists` directly.
        assert!(
            !sink
                .catalog
                .namespace_exists(&sink.namespace)
                .await
                .unwrap(),
            "empty append must not provision a namespace",
        );
    }

    #[tokio::test]
    async fn create_and_append_unpartitioned() {
        let tmp = tempdir().unwrap();
        let sink =
            sink_for_memory_catalog(warehouse_uri(tmp.path()), "lineage", "events", vec![]).await;

        sink.append(one_row_batch("run")).await.unwrap();
        assert!(sink.catalog.table_exists(&sink.table).await.unwrap());

        sink.append(one_row_batch("job")).await.unwrap();
        let table = sink.catalog.load_table(&sink.table).await.unwrap();
        assert!(
            table.metadata().current_snapshot().is_some(),
            "table should have at least one snapshot after two appends",
        );
    }

    #[tokio::test]
    async fn create_with_identity_partition_on_event_kind() {
        let tmp = tempdir().unwrap();
        let sink = sink_for_memory_catalog(
            warehouse_uri(tmp.path()),
            "lineage",
            "events",
            vec!["event_kind".into()],
        )
        .await;

        sink.append(one_row_batch("run")).await.unwrap();
        let table = sink.catalog.load_table(&sink.table).await.unwrap();
        let spec = table.metadata().default_partition_spec();
        assert!(
            !spec.fields().is_empty(),
            "expected an identity partition spec on event_kind",
        );
        let f = &spec.fields()[0];
        assert_eq!(f.name, "event_kind");
        assert_eq!(f.transform, Transform::Identity);
    }

    /// 3-row batch with two distinct `event_kind` values exercises the
    /// `split_by_partition` fanout: one Parquet file per partition value
    /// committed in a single transaction. We verify the on-disk layout
    /// directly because iceberg-rust 0.7's snapshot summary doesn't always
    /// populate `added-data-files` on the fast-append path.
    #[tokio::test]
    async fn partition_fanout_writes_one_file_per_partition_value() {
        let tmp = tempdir().unwrap();
        let sink = sink_for_memory_catalog(
            warehouse_uri(tmp.path()),
            "lineage",
            "events",
            vec!["event_kind".into()],
        )
        .await;

        let batch = mixed_kind_batch(&["run", "job", "run"]);
        sink.append(batch).await.unwrap();

        let table = sink.catalog.load_table(&sink.table).await.unwrap();
        assert!(
            table.metadata().current_snapshot().is_some(),
            "expected a snapshot after a partitioned append",
        );

        // The Hive-style partition layout written by `DefaultLocationGenerator`
        // produces one directory per partition value under the table's
        // `data/` prefix. Two distinct event_kind values → two dirs.
        let data_dir = tmp.path().join("lineage").join("events").join("data");
        let part_dirs: Vec<_> = std::fs::read_dir(&data_dir)
            .expect("data dir must exist after partitioned write")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        let mut sorted = part_dirs.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["event_kind=job".to_string(), "event_kind=run".to_string()],
            "expected one partition dir per distinct event_kind, got {:?}",
            part_dirs,
        );
    }

    /// Lakekeeper's REST catalog (via iceberg-catalog-rest 0.7) reports a
    /// 404 NoSuchTable as `ErrorKind::Unexpected` with the message
    /// "Tried to load a table that does not exist" — verify our helper
    /// classifies it as "not found" so `ensure_table` falls through to
    /// create instead of failing the very first append.
    #[test]
    fn is_not_found_handles_lakekeeper_unexpected_404() {
        let lakekeeper_404 = iceberg::Error::new(
            ErrorKind::Unexpected,
            "Tried to load a table that does not exist",
        );
        assert!(is_not_found(&lakekeeper_404));

        let memory_404 = iceberg::Error::new(ErrorKind::TableNotFound, "no such table");
        assert!(is_not_found(&memory_404));

        let auth_500 = iceberg::Error::new(ErrorKind::Unexpected, "boom");
        assert!(
            !is_not_found(&auth_500),
            "Unexpected without a not-found phrase must NOT be treated as 404",
        );
    }

    #[test]
    fn is_already_exists_handles_lakekeeper_409() {
        let kind_match = iceberg::Error::new(ErrorKind::TableAlreadyExists, "table already exists");
        assert!(is_already_exists(&kind_match));

        let lakekeeper_409 =
            iceberg::Error::new(ErrorKind::Unexpected, "Table lineage.events already exists");
        assert!(is_already_exists(&lakekeeper_409));

        let unrelated = iceberg::Error::new(ErrorKind::Unexpected, "boom");
        assert!(!is_already_exists(&unrelated));
    }

    #[tokio::test]
    async fn unknown_partition_column_is_rejected() {
        let tmp = tempdir().unwrap();
        let sink = sink_for_memory_catalog(
            warehouse_uri(tmp.path()),
            "lineage",
            "events",
            vec!["does_not_exist".into()],
        )
        .await;

        let err = sink.append(one_row_batch("run")).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("does_not_exist"),
            "error should mention the missing column, got: {msg}",
        );
    }

    fn iceberg_config(token: Option<&str>) -> IcebergConfig {
        IcebergConfig {
            catalog_uri: "http://lakekeeper:8181/catalog".into(),
            warehouse: "lineage".into(),
            namespace: "lineage".into(),
            table: "events".into(),
            partition_cols: vec!["event_kind".into()],
            token: token.map(|t| t.to_string()),
        }
    }

    #[test]
    fn build_rest_props_sets_uri_and_warehouse() {
        let props = build_rest_props(&iceberg_config(None));
        assert_eq!(
            props.get(REST_CATALOG_PROP_URI).map(String::as_str),
            Some("http://lakekeeper:8181/catalog"),
        );
        assert_eq!(
            props.get(REST_CATALOG_PROP_WAREHOUSE).map(String::as_str),
            Some("lineage"),
        );
    }

    #[test]
    fn build_rest_props_omits_token_when_absent() {
        let props = build_rest_props(&iceberg_config(None));
        assert!(
            !props.contains_key("token"),
            "no `token` property should be set when the config token is None",
        );
    }

    #[test]
    fn build_rest_props_sets_token_when_present() {
        let props = build_rest_props(&iceberg_config(Some("secret-bearer")));
        assert_eq!(
            props.get("token").map(String::as_str),
            Some("secret-bearer"),
            "the bearer token must be forwarded under the REST `token` property",
        );
    }
}
