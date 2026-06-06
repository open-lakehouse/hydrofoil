use std::sync::Arc;

use datafusion::catalog::TableProvider;
use datafusion::error::DataFusionError;
use datafusion::execution::TaskContext;
use datafusion::sql::TableReference;
use datafusion_cedar::TableFacts;
use deltalake_datafusion::catalog::unity::{TableProviderBuilder, TableProviderError};
use unitycatalog_common::models::tables::v1::Table;
use url::Url;

use crate::catalog::tags::{
    CatalogFactSinkExt, ConventionTagProvider, TagProvider, table_acl_facts,
};
use crate::session::TaskExt;

/// Builds Delta [`TableProvider`]s for Unity Catalog tables, backed by the
/// host session's task context.
///
/// The generic resolver in `deltalake-datafusion` handles UC metadata lookup,
/// credential vending, and object store registration; it delegates the actual
/// Delta provider construction here because that requires the log-store and
/// kernel-engine wiring owned by [`crate::session::LakehouseTaskContext`].
///
/// This is also the **resource/catalog PIP seam**: as each table resolves, its
/// catalog facts (owner / readers / writers / classification tags) are gathered
/// from the UC [`Table`] and recorded into the session's
/// [`CatalogFactSink`](datafusion_cedar::CatalogFactSink) (read from the
/// `CatalogFactSinkExt` config extension) so the policy layer can fold them into
/// Cedar evaluation.
pub struct LakehouseTableProviderBuilder {
    ctx: Arc<TaskContext>,
    tag_provider: Arc<dyn TagProvider>,
}

impl LakehouseTableProviderBuilder {
    pub fn new(ctx: Arc<TaskContext>) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            tag_provider: Arc::new(ConventionTagProvider),
        })
    }

    /// Gather this table's catalog facts and record them into the session's
    /// fact sink, keyed by the table's fully-qualified reference. A classify
    /// error is logged and treated as "no tags" — it must not block resolution;
    /// the reader/writer ACL facts (and the coarse gate) still apply.
    async fn record_facts(&self, table: &Table) {
        let Some(ext) = self
            .ctx
            .session_config()
            .get_extension::<CatalogFactSinkExt>()
        else {
            return;
        };

        let (owner, readers, writers) = table_acl_facts(table);
        let classification = self
            .tag_provider
            .classify(table)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, table = %table.full_name, "tag classification failed; recording no tags");
                Default::default()
            });

        let table_ref = TableReference::full(
            table.catalog_name.clone(),
            table.schema_name.clone(),
            table.name.clone(),
        );
        ext.0.record(
            table_ref,
            TableFacts {
                owner,
                readers,
                writers,
                tags: classification.table_tags,
                column_tags: classification.column_tags,
            },
        );
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
        table: &Table,
    ) -> Result<Arc<dyn TableProvider>, TableProviderError> {
        // Gather catalog facts for this table (best-effort; never blocks the
        // provider build).
        self.record_facts(table).await;

        self.ctx
            .lh()
            .delta_provider_for(location, None)
            .await
            .map_err(|e: DataFusionError| e)
    }
}
