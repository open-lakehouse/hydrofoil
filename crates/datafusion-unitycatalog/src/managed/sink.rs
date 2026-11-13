//! DataFusion `INSERT` for catalog-managed tables.
//!
//! [`ManagedDeltaTableProvider`] wraps the provider that reads the table (delta-rs's
//! `DeltaScanNext` over a catalog-managed snapshot) and leaves reads to it untouched. Its
//! `insert_into` plans a [`DataSinkExec`] over a [`ManagedTableSink`], which commits
//! through Unity Catalog via [`write_with_store`](super::append::write_with_store). Writes
//! to a managed table therefore never reach delta-rs's own commit path, which would write
//! `_delta_log/` directly.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::Session;
use datafusion::common::{Constraints, Result, Statistics, exec_datafusion_err, not_impl_err};
use datafusion::datasource::TableProvider;
use datafusion::datasource::sink::{DataSink, DataSinkExec};
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::logical_expr::dml::InsertOp;
use datafusion::logical_expr::{Expr, LogicalPlan, TableProviderFilterPushDown, TableType};
use datafusion::physical_plan::{DisplayAs, DisplayFormatType, ExecutionPlan};
use futures::TryStreamExt;
use object_store::ObjectStore;
use unitycatalog_object_store::{TableOperation, UnityObjectStoreFactory};

use super::append::{ManagedWriteMode, write_with_store};
use super::txn::ManagedTableRef;
use crate::sql::UnityFactoryExt;

/// `engineInfo` recorded in commits written through SQL.
const ENGINE_INFO: &str = "olai-uc-datafusion";

/// Where a managed write gets its write-capable object store.
#[derive(Clone)]
enum WriteAccess {
    /// Vend read-write credentials for the table from the session's
    /// [`UnityFactoryExt`] at planning time.
    FromSession,
    /// Use this client and bucket-rooted store (embedders that manage credentials
    /// themselves, and tests).
    Explicit {
        client: Arc<unitycatalog_client::DeltaV1Client>,
        store: Arc<dyn ObjectStore>,
    },
}

/// A DataFusion provider for a Unity Catalog catalog-managed Delta table.
///
/// Reads delegate to the wrapped provider. `INSERT INTO` and `INSERT OVERWRITE` commit
/// through Unity Catalog.
pub struct ManagedDeltaTableProvider {
    inner: Arc<dyn TableProvider>,
    catalog: String,
    schema: String,
    table: String,
    access: WriteAccess,
}

impl fmt::Debug for ManagedDeltaTableProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedDeltaTableProvider")
            .field("table", &self.full_name())
            .finish_non_exhaustive()
    }
}

impl ManagedDeltaTableProvider {
    /// Wrap `inner`, the read provider of the managed table `catalog.schema.table`.
    ///
    /// Writes vend credentials through the session's [`UnityFactoryExt`]; planning an
    /// `INSERT` fails if the session has none.
    pub fn new(
        inner: Arc<dyn TableProvider>,
        catalog: impl Into<String>,
        schema: impl Into<String>,
        table: impl Into<String>,
    ) -> Self {
        Self {
            inner,
            catalog: catalog.into(),
            schema: schema.into(),
            table: table.into(),
            access: WriteAccess::FromSession,
        }
    }

    /// Write through `client` and `store` instead of vending from the session.
    ///
    /// `store` must be bucket-rooted (paths are full object keys) and allow writes under
    /// the table location.
    pub fn with_write_store(
        mut self,
        client: Arc<unitycatalog_client::DeltaV1Client>,
        store: Arc<dyn ObjectStore>,
    ) -> Self {
        self.access = WriteAccess::Explicit { client, store };
        self
    }

    fn full_name(&self) -> String {
        format!("{}.{}.{}", self.catalog, self.schema, self.table)
    }

    async fn write_target(
        &self,
        state: &dyn Session,
    ) -> Result<(ManagedTableRef, Arc<dyn ObjectStore>)> {
        let (client, store) = match &self.access {
            WriteAccess::Explicit { client, store } => (client.clone(), store.clone()),
            WriteAccess::FromSession => {
                let factory: Arc<UnityObjectStoreFactory> = state
                    .config()
                    .get_extension::<UnityFactoryExt>()
                    .map(|ext| ext.0.clone())
                    .ok_or_else(|| {
                        exec_datafusion_err!(
                            "cannot write managed table {}: Unity Catalog is not configured \
                             on this session",
                            self.full_name()
                        )
                    })?;
                let uc_store = factory
                    .for_table(self.full_name(), TableOperation::ReadWrite)
                    .await
                    .map_err(|e| exec_datafusion_err!("vend write credentials: {e}"))?;
                (Arc::new(factory.unity_client().delta_v1()), uc_store.root())
            }
        };
        let target = ManagedTableRef::new(client, &self.catalog, &self.schema, &self.table);
        Ok((target, store))
    }
}

#[async_trait]
impl TableProvider for ManagedDeltaTableProvider {
    fn schema(&self) -> SchemaRef {
        self.inner.schema()
    }

    fn constraints(&self) -> Option<&Constraints> {
        self.inner.constraints()
    }

    fn table_type(&self) -> TableType {
        self.inner.table_type()
    }

    fn get_logical_plan(&'_ self) -> Option<Cow<'_, LogicalPlan>> {
        self.inner.get_logical_plan()
    }

    fn get_column_default(&self, column: &str) -> Option<&Expr> {
        self.inner.get_column_default(column)
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        self.inner.scan(state, projection, filters, limit).await
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> Result<Vec<TableProviderFilterPushDown>> {
        self.inner.supports_filters_pushdown(filters)
    }

    fn statistics(&self) -> Option<Statistics> {
        self.inner.statistics()
    }

    async fn insert_into(
        &self,
        state: &dyn Session,
        input: Arc<dyn ExecutionPlan>,
        insert_op: InsertOp,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let mode = match insert_op {
            InsertOp::Append => ManagedWriteMode::Append,
            InsertOp::Overwrite => ManagedWriteMode::Overwrite,
            InsertOp::Replace => {
                return not_impl_err!(
                    "REPLACE INTO is not supported on managed table {}",
                    self.full_name()
                );
            }
        };
        let (target, store) = self.write_target(state).await?;
        let sink = ManagedTableSink {
            target,
            store,
            mode,
            schema: self.schema(),
        };
        Ok(Arc::new(DataSinkExec::new(input, Arc::new(sink), None)))
    }
}

/// Commits the rows of an `INSERT` to a managed table as a single Unity Catalog commit.
pub struct ManagedTableSink {
    target: ManagedTableRef,
    store: Arc<dyn ObjectStore>,
    mode: ManagedWriteMode,
    schema: SchemaRef,
}

impl fmt::Debug for ManagedTableSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedTableSink")
            .field(
                "table",
                &format_args!(
                    "{}.{}.{}",
                    self.target.catalog, self.target.schema, self.target.table
                ),
            )
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

impl DisplayAs for ManagedTableSink {
    fn fmt_as(&self, _t: DisplayFormatType, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ManagedTableSink: table={}.{}.{}, mode={:?}",
            self.target.catalog, self.target.schema, self.target.table, self.mode
        )
    }
}

#[async_trait]
impl DataSink for ManagedTableSink {
    fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    async fn write_all(
        &self,
        data: SendableRecordBatchStream,
        _context: &Arc<TaskContext>,
    ) -> Result<u64> {
        let batches: Vec<_> = data.try_collect().await?;
        let rows: u64 = batches.iter().map(|b| b.num_rows() as u64).sum();
        write_with_store(
            &self.target,
            self.store.clone(),
            batches,
            self.mode,
            ENGINE_INFO,
        )
        .await
        .map_err(|e| exec_datafusion_err!("managed write failed: {e}"))?;
        Ok(rows)
    }
}
