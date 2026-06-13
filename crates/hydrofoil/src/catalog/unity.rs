use std::sync::Arc;

use datafusion::catalog::TableProvider;
use datafusion::error::DataFusionError;
use datafusion::execution::TaskContext;
use datafusion::sql::TableReference;
use datafusion_cedar::TableFacts;
use datafusion_unitycatalog::catalog::{TableProviderBuilder, TableProviderError};
use datafusion_unitycatalog::catalog::{ManagedReadState, resolve_managed_read_state};
use unitycatalog_client::UnityCatalogClient;
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
    /// UC client used to call the `/delta/v1` loadTable endpoint, which tells us
    /// whether a table is catalog-managed and, if so, supplies its ratified
    /// commit tail + latest version (the filesystem `_delta_log/` is not
    /// authoritative for managed tables).
    client: UnityCatalogClient,
    tag_provider: Arc<dyn TagProvider>,
}

impl LakehouseTableProviderBuilder {
    pub fn new(ctx: Arc<TaskContext>, client: UnityCatalogClient) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            client,
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

        // Ask the catalog whether this is a managed (coordinated-commit) table.
        // The `/delta/v1` loadTable response carries the table type and — for a
        // managed table — the unbackfilled commit tail + latest ratified version
        // a reader needs to materialize the catalog's snapshot.
        //
        // Time travel: the catalog-managed seam (`delta_managed_provider_for`)
        // accepts an `at_version`; the SQL `VERSION AS OF` plumbing into this
        // resolution path is a follow-up (see issue), so we pass `None` for now.
        let at_version = None;

        let loaded = match self
            .client
            .delta_v1()
            .load_table(&table.catalog_name, &table.schema_name, &table.name)
            .await
        {
            Ok(loaded) => loaded,
            // A6: `/delta/v1` is unavailable on this deployment (older OSS /
            // production Databricks). The legacy `tables` API already supplied the
            // storage location, so read the filesystem snapshot rather than failing
            // every Delta read. A genuine NoSuchTable / auth / other error
            // propagates (the predicate is the single source of truth, exported
            // from unitycatalog-rs).
            Err(e) if e.should_fall_back_to_legacy() => {
                tracing::debug!(
                    table = %table.full_name, error = %e,
                    "/delta/v1 loadTable unavailable; falling back to filesystem snapshot"
                );
                return self.ctx.lh().delta_provider_for(location, at_version).await;
            }
            Err(e) => return Err(DataFusionError::External(Box::new(e))),
        };

        // Resolve the read state once via the shared helper (no dispatch/fallback
        // logic is duplicated here — it all lives in unitycatalog-rs).
        match resolve_managed_read_state(&loaded)? {
            // The catalog is the source of truth: build from the ratified commit
            // tail + latest version rather than scanning `_delta_log/`.
            ManagedReadState::Managed { commits, latest } => {
                self.ctx
                    .lh()
                    .delta_managed_provider_for(location, &commits, latest as i64, at_version)
                    .await
            }
            // External / not-catalog-managed: the filesystem `_delta_log/` is
            // authoritative; the read must not set a catalog version.
            ManagedReadState::NotManaged => {
                self.ctx.lh().delta_provider_for(location, at_version).await
            }
        }
    }
}
