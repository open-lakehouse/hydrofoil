use std::sync::Arc;

use datafusion::catalog::TableProvider;
use datafusion::error::DataFusionError;
use datafusion::execution::TaskContext;
use deltalake_datafusion::catalog::unity::{TableProviderBuilder, TableProviderError};
use unitycatalog_common::models::tables::v1::Table;
use url::Url;

use crate::session::TaskExt;

/// Builds Delta [`TableProvider`]s for Unity Catalog tables, backed by the
/// host session's task context.
///
/// The generic resolver in `deltalake-datafusion` handles UC metadata lookup,
/// credential vending, and object store registration; it delegates the actual
/// Delta provider construction here because that requires the log-store and
/// kernel-engine wiring owned by [`crate::session::LakehouseTaskContext`].
pub struct LakehouseTableProviderBuilder {
    ctx: Arc<TaskContext>,
}

impl LakehouseTableProviderBuilder {
    pub fn new(ctx: Arc<TaskContext>) -> Arc<Self> {
        Arc::new(Self { ctx })
    }
}

impl std::fmt::Debug for LakehouseTableProviderBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LakehouseTableProviderBuilder")
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl TableProviderBuilder for LakehouseTableProviderBuilder {
    async fn build_delta(
        &self,
        location: &Url,
        _table: &Table,
    ) -> Result<Arc<dyn TableProvider>, TableProviderError> {
        self.ctx
            .lh()
            .delta_provider_for(location, None)
            .await
            .map_err(|e: DataFusionError| e)
    }
}
