use std::collections::HashMap;

use async_trait::async_trait;
use deltalake::arrow::array::RecordBatch;
use deltalake::protocol::SaveMode;
use deltalake::{DeltaOps, DeltaTable, DeltaTableBuilder};

use crate::config::Config;
use crate::writer::sink::{SinkError, TableSink};

/// Delta Lake sink for the lineage events table.
///
/// Owns a URI + `storage_options` map; opens (or creates) the table on every
/// `append` so concurrent writers from other engines see a fresh snapshot.
#[derive(Clone)]
pub struct DeltaWriter {
    table_uri: String,
    storage_options: HashMap<String, String>,
    partition_cols: Vec<String>,
}

/// Forward-looking alias matching the new `TableSink` naming convention.
pub type DeltaSink = DeltaWriter;

impl DeltaWriter {
    pub fn new(cfg: &Config) -> Self {
        Self {
            table_uri: cfg.delta.table_path.clone(),
            storage_options: cfg.storage_options.clone(),
            partition_cols: cfg.delta.partition_cols.clone(),
        }
    }

    pub async fn append(&self, batch: RecordBatch) -> Result<(), DeltaWriteError> {
        if batch.num_rows() == 0 {
            return Ok(());
        }

        let table = self.open_or_create_table().await?;
        let mut write_op = DeltaOps(table)
            .write(vec![batch])
            .with_save_mode(SaveMode::Append);

        if !self.partition_cols.is_empty() {
            write_op = write_op.with_partition_columns(self.partition_cols.clone());
        }

        write_op
            .await
            .map_err(|e| DeltaWriteError::Write(e.to_string()))?;

        Ok(())
    }

    async fn open_or_create_table(&self) -> Result<DeltaTable, DeltaWriteError> {
        let builder = DeltaTableBuilder::from_uri(&self.table_uri)
            .with_storage_options(self.storage_options.clone());

        match builder.build() {
            Ok(mut table) => {
                if table.load().await.is_ok() {
                    return Ok(table);
                }
                self.create_empty_table().await
            }
            Err(_) => self.create_empty_table().await,
        }
    }

    async fn create_empty_table(&self) -> Result<DeltaTable, DeltaWriteError> {
        let table = DeltaOps::try_from_uri_with_storage_options(
            &self.table_uri,
            self.storage_options.clone(),
        )
        .await
        .map_err(|e| DeltaWriteError::Create(e.to_string()))?;

        Ok(table.0)
    }
}

#[async_trait]
impl TableSink for DeltaWriter {
    fn name(&self) -> &'static str {
        "delta"
    }

    async fn append(&self, batch: RecordBatch) -> Result<(), SinkError> {
        DeltaWriter::append(self, batch)
            .await
            .map_err(|e| SinkError::Delta(e.to_string()))
    }
}

#[derive(Debug)]
pub enum DeltaWriteError {
    Create(String),
    Write(String),
}

impl std::fmt::Display for DeltaWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeltaWriteError::Create(msg) => write!(f, "failed to create delta table: {msg}"),
            DeltaWriteError::Write(msg) => write!(f, "failed to write to delta table: {msg}"),
        }
    }
}

impl std::error::Error for DeltaWriteError {}
