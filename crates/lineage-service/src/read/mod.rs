//! Read layer: a Marquez-compatible REST API served over the Delta events
//! table the ingest path writes to.
//!
//! The ingest side ([`crate::writer`]) only ever appends raw OpenLineage events.
//! Marquez's web UI, by contrast, expects a *materialized* model — namespaces,
//! jobs, datasets, and a lineage graph with edges. This module reconstructs that
//! model on read by querying the events table with DataFusion ([`queries`]) and
//! shaping the result into Marquez's JSON contract ([`model`]). [`http`] mounts
//! the endpoints the UI needs under `/api/v1`.
//!
//! The store re-opens the Delta table on every query so freshly ingested events
//! are visible without a restart. Query volume for a lineage UI is low, so the
//! per-request `open_table` cost is acceptable. The queries push their column
//! projection (and, for the events/run-facets endpoints, row filters and
//! pagination) down into DataFusion so we don't materialize the whole log per
//! request. The model-folding endpoints still scan every event row each call;
//! that grows unbounded with the log. FOLLOW-UP: maintain a materialized model
//! (incremental fold on ingest, or a periodic snapshot) and/or partition/index
//! the events table so reads don't re-fold history. Deferred — out of scope for
//! the read-correctness pass.

pub mod http;
pub mod model;
pub mod queries;

use std::collections::HashMap;

use deltalake::datafusion::prelude::SessionContext;
use deltalake::{DeltaTableError, ensure_table_uri, open_table_with_storage_options};

use crate::config::Config;

/// Column names of the events table, kept in one place so the read queries don't
/// drift from [`crate::writer::schema::arrow_schema`]. Referenced in
/// [`queries`].
pub mod columns {
    pub const EVENT_KIND: &str = "event_kind";
    pub const EVENT_TYPE: &str = "event_type";
    pub const EVENT_TIME: &str = "event_time";
    pub const RUN_ID: &str = "run_id";
    pub const JOB_NAMESPACE: &str = "job_namespace";
    pub const JOB_NAME: &str = "job_name";
    pub const DATASET_NAMESPACE: &str = "dataset_namespace";
    pub const DATASET_NAME: &str = "dataset_name";
    pub const INPUTS_JSON: &str = "inputs_json";
    pub const OUTPUTS_JSON: &str = "outputs_json";
    pub const COLUMN_LINEAGE_JSON: &str = "column_lineage_json";
    pub const RAW_JSON: &str = "raw_json";
}

/// Table name the events table is registered under inside the per-query
/// DataFusion session.
const EVENTS_TABLE: &str = "events";

/// Errors surfaced by the read layer. The HTTP layer maps these onto status
/// codes (404 for not-found, 500 otherwise).
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("failed to open delta table: {0}")]
    OpenTable(String),

    #[error("query failed: {0}")]
    Query(String),

    #[error("not found: {0}")]
    NotFound(String),
}

impl From<DeltaTableError> for ReadError {
    fn from(e: DeltaTableError) -> Self {
        ReadError::OpenTable(e.to_string())
    }
}

impl From<deltalake::datafusion::error::DataFusionError> for ReadError {
    fn from(e: deltalake::datafusion::error::DataFusionError) -> Self {
        ReadError::Query(e.to_string())
    }
}

/// Read-only handle over the lineage events Delta table.
///
/// Holds the table location + storage options (lifted from [`Config`]) and opens
/// a fresh [`SessionContext`] with the table registered for each query. Cheap to
/// clone — it is just the URI and the options map.
#[derive(Clone)]
pub struct LineageStore {
    table_uri: String,
    storage_options: HashMap<String, String>,
}

impl LineageStore {
    /// Build a store from the service config. Uses the same `table_path` and
    /// `storage_options` the Delta writer uses, so reads and writes target the
    /// same table regardless of `delta.mode`.
    ///
    /// Note: for the Unity Catalog delta modes the writer resolves the location
    /// through the catalog; the read path here only supports the local /
    /// object-store URI in `delta.table_path`. The Marquez UI is a local-stack
    /// demo concern, so this is sufficient for now. We emit a loud startup
    /// warning when the mode is not `local`, because the read path then reads
    /// `delta.table_path` (likely empty) while ingest writes to the catalog —
    /// silently rendering an empty UI is the trap this warning prevents.
    pub fn from_config(cfg: &Config) -> Self {
        let mode = cfg.delta.mode.as_str();
        if !matches!(mode, "local" | "raw") {
            tracing::warn!(
                delta.mode = mode,
                table_path = %cfg.delta.table_path,
                "lineage read API reads the local delta.table_path, but delta.mode is not `local`; \
                 ingest writes through Unity Catalog, so the Marquez UI will likely render empty. \
                 Point the read path at the catalog-resolved location or switch to `local` mode."
            );
        }
        Self {
            table_uri: cfg.delta.table_path.clone(),
            storage_options: cfg.storage_options.clone(),
        }
    }

    /// Open the events table and register it on a fresh session as `events`.
    ///
    /// Returns `Ok(None)` when the table does not exist yet (no events have ever
    /// been ingested) so callers can return empty results rather than an error —
    /// a brand-new deployment should render an empty UI, not a 500.
    pub(crate) async fn session(&self) -> Result<Option<SessionContext>, ReadError> {
        let url =
            ensure_table_uri(&self.table_uri).map_err(|e| ReadError::OpenTable(e.to_string()))?;

        let table = match open_table_with_storage_options(url, self.storage_options.clone()).await {
            Ok(t) => t,
            // Table not created yet — treat as empty rather than an error.
            Err(DeltaTableError::NotATable(_)) | Err(DeltaTableError::InvalidTableLocation(_)) => {
                return Ok(None);
            }
            Err(e) => return Err(ReadError::OpenTable(e.to_string())),
        };

        let ctx = SessionContext::new();
        table
            .update_datafusion_session(&ctx.state())
            .map_err(|e| ReadError::OpenTable(e.to_string()))?;
        let provider = table
            .table_provider()
            .await
            .map_err(|e| ReadError::OpenTable(e.to_string()))?;
        ctx.register_table(EVENTS_TABLE, provider)?;
        Ok(Some(ctx))
    }
}
