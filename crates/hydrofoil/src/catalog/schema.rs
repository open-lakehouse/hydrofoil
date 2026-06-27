use std::{any::Any, sync::Arc};

use dashmap::DashMap;
use datafusion::{
    catalog::{AsyncSchemaProvider, MemorySchemaProvider, SchemaProvider, Session},
    common::{Result, plan_datafusion_err},
    datasource::TableProvider,
};
use deltalake_core::{
    delta_datafusion::{DeltaScanConfig, DeltaScanNext, engine::DataFusionEngine},
    kernel::Snapshot,
};
use itertools::Itertools as _;
use tracing::instrument;

/// A schema provider that manages Delta Lake tables in a lakehouse.
///
/// Delta table prvoders are based on the underlying `Snapshot` and are cached
/// for efficient retrieval. A Snapshot represents the state of a Delta table at a specific
/// point in time. When the table is requested, it may have changed since it was last cached,
/// so the snapshot is updated to reflect the latest state of the table before creating
/// the table provider.
///
/// This schema provider also supports a fallback schema provider for non-Delta tables.
///
/// There is also the option to use the async schema provider pattern via the
/// `AsyncSchemaProvider` trait implementation. See a datafusion example
/// [here](https://github.com/apache/datafusion/blob/main/datafusion-examples/examples/data_io/remote_catalog.rs).
#[derive(Clone)]
pub struct LakehouseSchemaProvider {
    session: Arc<dyn Session>,
    tables: Arc<DashMap<String, Arc<Snapshot>>>,
    fallback: Arc<dyn SchemaProvider>,
}

impl std::fmt::Debug for LakehouseSchemaProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LakehouseSchemaProvider")
            .field(
                "table_names",
                &self.tables.iter().map(|e| e.key().clone()).collect_vec(),
            )
            .finish()
    }
}

impl LakehouseSchemaProvider {
    pub fn new(session: Arc<dyn Session>) -> Arc<Self> {
        Arc::new(Self {
            session,
            tables: Arc::new(DashMap::new()),
            fallback: Arc::new(MemorySchemaProvider::new()),
        })
    }
}

#[async_trait::async_trait]
impl SchemaProvider for LakehouseSchemaProvider {
    fn table_names(&self) -> Vec<String> {
        self.tables
            .iter()
            .map(|entry| entry.key().clone())
            .chain(self.fallback.table_names())
            .collect()
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            hydrofoil.table = name,
        )
    )]
    async fn table(&self, name: &str) -> Result<Option<Arc<dyn TableProvider>>> {
        let Some(snapshot) = self.tables.get(name).map(|s| s.value().clone()) else {
            return self.fallback.table(name).await;
        };
        let engine = DataFusionEngine::new_from_context(self.session.task_ctx());
        let snapshot = snapshot.update(engine, None).await.map_err(|e| {
            plan_datafusion_err!("Failed to update Delta snapshot for '{}': {}", name, e)
        })?;
        let config = DeltaScanConfig::new_from_session(self.session.as_ref());
        Ok(Some(Arc::new(DeltaScanNext::new(snapshot, config)?)))
    }

    fn register_table(
        &self,
        name: String,
        table: Arc<dyn TableProvider>,
    ) -> Result<Option<Arc<dyn TableProvider>>> {
        if (table.as_ref() as &dyn Any)
            .downcast_ref::<DeltaScanNext>()
            .is_some()
        {
            // TODO(migration): in deltalake-core 0.32 the inner `Snapshot` of a
            // `DeltaScanNext` is no longer publicly accessible (`snapshot()` is
            // now `pub(crate)`), so we can no longer cache the snapshot when a
            // delta provider is re-registered here. Snapshots are still cached
            // via `LakehouseTaskContext` when tables are first resolved, so this
            // path is a no-op for now; re-enable once an accessor is available
            // upstream (or restructure to register snapshots directly).
            return Ok(None);
        }
        self.tables.remove(&name);
        self.fallback.register_table(name, table)
    }

    fn table_exist(&self, name: &str) -> bool {
        self.tables.contains_key(name) || self.fallback.table_exist(name)
    }
}

#[async_trait::async_trait]
impl AsyncSchemaProvider for LakehouseSchemaProvider {
    #[instrument(
        skip_all,
        level = "info",
        fields(
            hydrofoil.table = name,
        )
    )]
    async fn table(&self, name: &str) -> Result<Option<Arc<dyn TableProvider>>> {
        <Self as SchemaProvider>::table(self, name).await
    }
}
