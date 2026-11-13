//! The commit layer shared by every managed-table write.
//!
//! A write hands this module a [`KernelTxnStage`]: the file actions to commit, held as
//! plain Arrow batches in the kernel's own schemas. [`commit_managed`] then runs the
//! commit loop against Unity Catalog:
//!
//! 1. `loadTable`, then a kernel snapshot built from the catalog's ratified tail.
//! 2. `snapshot.update_table(..).build(engine, UnityCatalogCommitter)`.
//! 3. Stage the actions with [`stage_imperative`], then `commit`.
//! 4. On success, run the best-effort post-commit work (publish, backfill, metrics).
//!
//! Failed attempts are classified as follows (delta.yaml `updateTable` errors,
//! ManagedTablesSpec commit-errors table):
//!
//! - **409 conflict**: another writer took our version. Delete our staged commit, reload,
//!   rebuild the stage on the new snapshot, and retry.
//! - **429 resource-exhausted**: the unbackfilled tail is too long. Publish and backfill
//!   the tail, then retry.
//! - **500 commit-state-unknown**: the outcome is ambiguous. Reload and look for our
//!   exact commit. If it is there, the write succeeded. Never commit it again.
//! - **Ratified, then a kernel error**: success. The committer records the ratified
//!   version, so an error after UC accepted the commit can't cause a second commit.
//!
//! # Declarative-ready
//!
//! [`stage_imperative`] is the only function that calls kernel staging APIs. The stage
//! holds exactly what kernel's declarative transaction API takes:
//! - adds in `Transaction::add_files_schema`
//! - removes as selected rows in `scan_row_schema`
//!
//! Once kernel ships `Transaction::commit_declarative` (kernel main has
//! `Committer::commit_declarative` and the `*_ref` staging APIs; the commit itself is
//! still a stub), add a `stage_declarative` that registers the same batches as relations
//! in a `ScopedPlanExecutor` and calls `add_files_ref` / `remove_files_ref`. Nothing
//! outside this module has to change.

use std::sync::Arc;
use std::time::Duration;

use datafusion::arrow::array::{RecordBatch, StringArray};
use delta_kernel::engine::arrow_data::ArrowEngineData;
use delta_kernel::snapshot::{Snapshot as KernelSnapshot, SnapshotRef};
use delta_kernel::transaction::{CommitResult, Transaction, UpdateTableOperation};
use delta_kernel::{Engine, EngineData, FilteredEngineData, Version};
use delta_kernel_default_engine::DefaultEngine;
use delta_kernel_default_engine::executor::tokio::TokioBackgroundExecutor;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt};
use tracing::{debug, info, warn};
use unitycatalog_client::DeltaV1Client;
use unitycatalog_delta_api::models::{
    DeltaCommitReport, DeltaLoadTableResponse, DeltaReport, DeltaReportMetricsRequest,
    DeltaTableRequirement, DeltaTableUpdate, DeltaUpdateTableRequest,
};
use url::Url;

use crate::catalog::{
    ManagedReadState, ensure_trailing_slash, resolve_managed_read_state, to_log_tail,
};

use super::committer::UnityCatalogCommitter;
use super::create::CreateManagedTableError;

/// The engine every managed write runs on: the default engine over a credentialed,
/// bucket-rooted object store.
pub(crate) type ManagedEngine = DefaultEngine<TokioBackgroundExecutor>;

/// Maximum number of commit attempts before giving up (1 initial try + retries).
const MAX_COMMIT_ATTEMPTS: u32 = 5;
/// Base backoff between retry attempts; jittered and capped per attempt.
const RETRY_BASE_BACKOFF: Duration = Duration::from_millis(50);
const RETRY_MAX_BACKOFF: Duration = Duration::from_secs(2);

/// A managed table addressed the way the `/delta/v1` API addresses it.
#[derive(Clone)]
pub(crate) struct ManagedTableRef {
    pub client: Arc<DeltaV1Client>,
    pub catalog: String,
    pub schema: String,
    pub table: String,
}

impl ManagedTableRef {
    pub(crate) fn new(
        client: Arc<DeltaV1Client>,
        catalog: impl Into<String>,
        schema: impl Into<String>,
        table: impl Into<String>,
    ) -> Self {
        Self {
            client,
            catalog: catalog.into(),
            schema: schema.into(),
            table: table.into(),
        }
    }

    pub(crate) async fn load(&self) -> Result<LoadedTable, CreateManagedTableError> {
        let response = self
            .client
            .load_table(&self.catalog, &self.schema, &self.table)
            .await?;
        let table_id = response
            .metadata
            .properties
            .get("io.unitycatalog.tableId")
            .cloned()
            .ok_or_else(|| {
                CreateManagedTableError::other(format!(
                    "table {}.{}.{} is missing the required io.unitycatalog.tableId \
                     property (ManagedTablesSpec §table properties)",
                    self.catalog, self.schema, self.table
                ))
            })?;
        let location = Url::parse(&ensure_trailing_slash(&response.metadata.location))
            .map_err(|e| CreateManagedTableError::other(format!("invalid table location: {e}")))?;
        Ok(LoadedTable {
            response,
            table_id,
            location,
        })
    }

    pub(crate) fn committer(&self, table_id: &str) -> UnityCatalogCommitter {
        UnityCatalogCommitter::new(
            self.client.clone(),
            &self.catalog,
            &self.schema,
            &self.table,
            table_id,
        )
    }
}

/// One `loadTable` response with the two fields every commit needs pulled out.
pub(crate) struct LoadedTable {
    pub response: DeltaLoadTableResponse,
    pub table_id: String,
    pub location: Url,
}

impl LoadedTable {
    /// The catalog-managed snapshot this response describes.
    pub(crate) fn snapshot(
        &self,
        engine: &dyn Engine,
    ) -> Result<SnapshotRef, CreateManagedTableError> {
        build_snapshot(&self.response, &self.location, engine)
    }
}

/// The file actions of one commit, as Arrow batches in the kernel's schemas.
///
/// - `adds`: rows in `Transaction::add_files_schema` (what `write_parquet` returns).
/// - `removes`: the rows to remove, in `scan_row_schema` (what a scan's `scan_files`
///   carries), with the selection already applied.
#[derive(Debug, Clone, Default)]
pub(crate) struct KernelTxnStage {
    pub adds: Vec<RecordBatch>,
    pub removes: Vec<RecordBatch>,
}

impl KernelTxnStage {
    pub(crate) fn num_adds(&self) -> usize {
        self.adds.iter().map(RecordBatch::num_rows).sum()
    }

    pub(crate) fn num_removes(&self) -> usize {
        self.removes.iter().map(RecordBatch::num_rows).sum()
    }
}

/// Apply `stage` to an imperative kernel transaction.
pub(crate) fn stage_imperative(stage: &KernelTxnStage, txn: &mut Transaction) {
    for adds in &stage.adds {
        txn.add_files(Box::new(ArrowEngineData::new(adds.clone())));
    }
    for removes in &stage.removes {
        txn.remove_files(FilteredEngineData::with_all_rows_selected(Box::new(
            ArrowEngineData::new(removes.clone()),
        )));
    }
}

/// Convert engine data the default engine produced back into an Arrow batch.
pub(crate) fn into_record_batch(
    data: Box<dyn EngineData>,
) -> Result<RecordBatch, CreateManagedTableError> {
    Ok(ArrowEngineData::try_from_engine_data(data)
        .map_err(CreateManagedTableError::Kernel)?
        .into())
}

/// Every live file of `snapshot`, as remove rows in `scan_row_schema`.
pub(crate) fn remove_all_files(
    snapshot: &SnapshotRef,
    engine: &dyn Engine,
) -> Result<Vec<RecordBatch>, CreateManagedTableError> {
    let scan = snapshot.clone().scan_builder().build()?;
    let mut removes = Vec::new();
    for metadata in scan.scan_metadata(engine)? {
        let selected = metadata?.scan_files.apply_selection_vector()?;
        let batch = into_record_batch(selected)?;
        if batch.num_rows() > 0 {
            removes.push(batch);
        }
    }
    Ok(removes)
}

/// The `path` column of add rows, relative to the table root.
pub(crate) fn add_paths(adds: &[RecordBatch]) -> Vec<String> {
    adds.iter()
        .filter_map(|batch| {
            batch
                .column_by_name("path")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>().cloned())
        })
        .flat_map(|paths| {
            paths
                .iter()
                .flatten()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Why a managed commit failed.
#[derive(Debug)]
pub(crate) struct CommitFailure {
    pub error: CreateManagedTableError,
    /// UC may have ratified the commit after all (a 500, a transport error, …). The
    /// caller must not delete files the commit references.
    pub outcome_unknown: bool,
}

impl From<CreateManagedTableError> for CommitFailure {
    fn from(error: CreateManagedTableError) -> Self {
        // Errors raised outside the UC request (load, snapshot, staging) happen before
        // anything is proposed, so the outcome is known: nothing was committed.
        Self {
            error,
            outcome_unknown: false,
        }
    }
}

/// What a commit attempt needs besides the stage.
pub(crate) struct CommitOptions<'a> {
    pub operation: UpdateTableOperation,
    pub engine_info: &'a str,
    pub data_change: bool,
    /// Rows the commit adds, for the UC commit report.
    pub num_rows_inserted: i64,
}

/// Commit through Unity Catalog with bounded retries, then run post-commit work.
///
/// `first` is the table state the caller already loaded for its first attempt (so it can
/// derive the write context from it). Later attempts reload. `stage_for` builds the stage
/// for an attempt's snapshot. It runs again on every attempt, so operations whose actions
/// depend on table state (overwrite) see the state they actually commit on top of.
///
/// `store` is the bucket-rooted store the engine writes through. It deletes the staged
/// commit of a conflicted attempt; the kernel storage handler has no delete.
pub(crate) async fn commit_managed<F>(
    target: &ManagedTableRef,
    engine: &ManagedEngine,
    store: &Arc<dyn ObjectStore>,
    first: LoadedTable,
    options: CommitOptions<'_>,
    mut stage_for: F,
) -> Result<Version, CommitFailure>
where
    F: FnMut(&SnapshotRef) -> Result<KernelTxnStage, CreateManagedTableError>,
{
    let mut outcome_unknown = false;
    let mut next = Some(first);
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        let loaded = match next.take() {
            Some(loaded) => loaded,
            None => target.load().await?,
        };
        let snapshot = loaded.snapshot(engine)?;
        let stage = stage_for(&snapshot)?;

        let committer = target.committer(&loaded.table_id);
        let mut txn = snapshot
            .clone()
            .update_table(
                options.operation.clone(),
                options.engine_info,
                options.data_change,
            )
            .build(engine, Box::new(committer.clone()))
            .map_err(CreateManagedTableError::Kernel)?;
        stage_imperative(&stage, &mut txn);

        let result = txn.commit(engine);

        // UC accepted the commit: it landed, whatever the kernel did afterwards.
        if let Some(version) = committer.last_ratified() {
            let version = Version::try_from(version)
                .map_err(|_| CreateManagedTableError::other("negative ratified version"))?;
            let post = match &result {
                Ok(CommitResult::Committed(c)) => c.post_commit_snapshot().cloned(),
                Ok(_) => None,
                Err(e) => {
                    warn!(version, error = %e, "kernel error after UC ratified the commit; treating as committed");
                    None
                }
            };
            info!(version, attempt, "committed managed-table write");
            run_post_commit(
                target,
                &loaded.table_id,
                engine,
                post,
                version,
                &stage,
                &options,
            )
            .await;
            return Ok(version);
        }

        let retry = match result {
            Ok(CommitResult::Committed(c)) => {
                // A catalog committer always records the ratified version; reaching this
                // means the commit went through some other path. Trust the kernel.
                let version = c.commit_version();
                let post = c.post_commit_snapshot().cloned();
                run_post_commit(
                    target,
                    &loaded.table_id,
                    engine,
                    post,
                    version,
                    &stage,
                    &options,
                )
                .await;
                return Ok(version);
            }
            Ok(CommitResult::Conflicted(c)) => {
                warn!(
                    conflict_version = c.conflict_version(),
                    attempt, "commit conflict; reloading and retrying"
                );
                delete_staged_commit(store, &loaded.location, &committer).await;
                Retry::Again
            }
            Ok(CommitResult::Retryable(rt)) => {
                handle_typed_retry(
                    target, &committer, engine, store, &loaded, attempt, rt.error,
                )
                .await
            }
            Err(kernel_err) => {
                handle_typed_retry(
                    target, &committer, engine, store, &loaded, attempt, kernel_err,
                )
                .await
            }
        };

        match retry {
            Retry::Landed(version) => {
                info!(
                    version,
                    attempt, "commit-state-unknown resolved: our commit landed"
                );
                // Publishing and metrics ride on the next write; the snapshot we have is
                // from before the commit.
                return Ok(version);
            }
            Retry::Again if attempt < MAX_COMMIT_ATTEMPTS => backoff(attempt).await,
            Retry::Again => {
                return Err(CommitFailure {
                    error: CreateManagedTableError::other(format!(
                        "managed-table commit failed after {attempt} attempts"
                    )),
                    outcome_unknown,
                });
            }
            Retry::AgainOutcomeUnknown if attempt < MAX_COMMIT_ATTEMPTS => {
                outcome_unknown = true;
                backoff(attempt).await;
            }
            Retry::AgainOutcomeUnknown => {
                return Err(CommitFailure {
                    error: CreateManagedTableError::other(format!(
                        "managed-table commit failed after {attempt} attempts; the last \
                         attempt's outcome is unknown"
                    )),
                    outcome_unknown: true,
                });
            }
            Retry::Fail {
                error,
                outcome_unknown: this_unknown,
            } => {
                return Err(CommitFailure {
                    error,
                    outcome_unknown: outcome_unknown || this_unknown,
                });
            }
        }
    }
}

/// What to do after a failed commit attempt.
enum Retry {
    /// Our commit was found in the table: the write succeeded at this version.
    Landed(Version),
    /// Nothing was committed; retry.
    Again,
    /// Retry, but this attempt may have been ratified without us seeing it.
    AgainOutcomeUnknown,
    /// Give up.
    Fail {
        error: CreateManagedTableError,
        outcome_unknown: bool,
    },
}

/// Classify a failed attempt by the typed UC error the committer recorded.
async fn handle_typed_retry(
    target: &ManagedTableRef,
    committer: &UnityCatalogCommitter,
    engine: &ManagedEngine,
    store: &Arc<dyn ObjectStore>,
    loaded: &LoadedTable,
    attempt: u32,
    kernel_err: delta_kernel::KernelError,
) -> Retry {
    let Some(uc_err) = committer.take_last_error() else {
        // No UC error: the failure came before the UC request (staging, validation).
        // Validation errors are deterministic, so only retry IO-style failures.
        if matches!(kernel_err, delta_kernel::KernelError::IOError(_)) {
            warn!(attempt, error = %kernel_err, "retryable kernel commit error; retrying");
            return Retry::Again;
        }
        return Retry::Fail {
            error: CreateManagedTableError::Kernel(kernel_err),
            outcome_unknown: false,
        };
    };

    if uc_err.is_commit_state_unknown() {
        return match commit_landed(target, committer, store).await {
            Ok(Some(version)) => Retry::Landed(version),
            Ok(None) => {
                warn!(
                    attempt,
                    "commit-state-unknown: our commit is absent; retrying"
                );
                Retry::AgainOutcomeUnknown
            }
            Err(error) => Retry::Fail {
                error,
                outcome_unknown: true,
            },
        };
    }

    if uc_err.is_resource_exhausted() {
        warn!(
            attempt,
            "resource-exhausted (429); backfilling pending tail before retry"
        );
        if let Err(e) = publish_and_backfill_loaded(target, engine, loaded).await {
            warn!(error = %e, "backfill before retry failed; retrying anyway");
        }
        return Retry::Again;
    }

    // Any other UC error is permanent. A definite 4xx rejection means nothing was
    // ratified; anything else (transport error, 5xx) might have been.
    let outcome_unknown = !is_definite_rejection(&uc_err);
    if !outcome_unknown {
        delete_staged_commit(store, &loaded.location, committer).await;
    }
    Retry::Fail {
        error: CreateManagedTableError::Client(uc_err),
        outcome_unknown,
    }
}

/// Whether UC answered with a 4xx, so the commit was definitely not ratified.
pub(crate) fn is_definite_rejection(err: &unitycatalog_client::Error) -> bool {
    match err {
        unitycatalog_client::Error::Delta(model) => (400..500).contains(&model.code),
        unitycatalog_client::Error::Api(api) => (400..500).contains(&api.http_status()),
        _ => false,
    }
}

/// Resolve a commit-state-unknown by looking for the commit we proposed.
///
/// It landed if UC lists our staged file (same version, same file name) in the ratified
/// tail. If the tail has moved past our version, the version may have been published
/// and dropped from the tail. In that case, compare the published `<v>.json` byte for
/// byte with our staged file: publishing copies the ratified staged file, so equal
/// bytes mean ours is the commit that won. `latest_table_version >= v` alone is not
/// enough, since a concurrent writer may own that version.
async fn commit_landed(
    target: &ManagedTableRef,
    committer: &UnityCatalogCommitter,
    store: &Arc<dyn ObjectStore>,
) -> Result<Option<Version>, CreateManagedTableError> {
    let Some((version, file_name)) = committer.last_proposed() else {
        return Ok(None);
    };
    let landed = Version::try_from(version)
        .map_err(|_| CreateManagedTableError::other("negative proposed commit version"))?;
    let loaded = target.load().await?;
    let in_tail = loaded
        .response
        .commits
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .any(|c| c.version == version && c.file_name == file_name);
    if in_tail {
        return Ok(Some(landed));
    }
    if loaded
        .response
        .latest_table_version
        .is_none_or(|latest| latest < version)
    {
        return Ok(None);
    }
    let staged = staged_commit_path(&loaded.location, &file_name)?;
    let published = published_commit_path(&loaded.location, landed)?;
    let ours = store.get(&staged).await?.bytes().await?;
    match store.get(&published).await {
        Ok(theirs) => Ok((theirs.bytes().await? == ours).then_some(landed)),
        Err(object_store::Error::NotFound { .. }) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn table_relative_path(location: &Url, relative: &str) -> Result<Path, CreateManagedTableError> {
    let url = location
        .join(relative)
        .map_err(|e| CreateManagedTableError::other(format!("invalid table path: {e}")))?;
    Path::from_url_path(url.path())
        .map_err(|e| CreateManagedTableError::other(format!("invalid table path: {e}")))
}

fn staged_commit_path(location: &Url, file_name: &str) -> Result<Path, CreateManagedTableError> {
    table_relative_path(location, &format!("_delta_log/_staged_commits/{file_name}"))
}

fn published_commit_path(
    location: &Url,
    version: Version,
) -> Result<Path, CreateManagedTableError> {
    table_relative_path(location, &format!("_delta_log/{version:020}.json"))
}

/// Delete the staged commit the committer last proposed. Only call this once UC has
/// definitely not ratified it (a conflict or a 4xx).
async fn delete_staged_commit(
    store: &Arc<dyn ObjectStore>,
    location: &Url,
    committer: &UnityCatalogCommitter,
) {
    let Some((_, file_name)) = committer.last_proposed() else {
        return;
    };
    let result = match staged_commit_path(location, &file_name) {
        Ok(path) => store
            .delete(&path)
            .await
            .map_err(CreateManagedTableError::from),
        Err(e) => Err(e),
    };
    match result {
        Ok(()) => debug!(%file_name, "deleted rejected staged commit"),
        Err(e) => warn!(%file_name, error = %e, "failed to delete rejected staged commit"),
    }
}

/// Delete data files written for a commit that definitely did not land.
pub(crate) async fn delete_data_files(
    store: &Arc<dyn ObjectStore>,
    location: &Url,
    paths: &[String],
) {
    for relative in paths {
        let result = match table_relative_path(location, relative) {
            Ok(path) => store
                .delete(&path)
                .await
                .map_err(CreateManagedTableError::from),
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            warn!(path = %relative, error = %e, "failed to delete orphaned data file");
        }
    }
}

/// Best-effort post-commit work: publish the ratified tail to `_delta_log/`, notify UC
/// with `set-latest-backfilled-version`, and report commit metrics. The commit is already
/// ratified, so a failure here is logged and never turns into a failed write.
async fn run_post_commit(
    target: &ManagedTableRef,
    table_id: &str,
    engine: &ManagedEngine,
    post_commit_snapshot: Option<SnapshotRef>,
    version: Version,
    stage: &KernelTxnStage,
    options: &CommitOptions<'_>,
) {
    match post_commit_snapshot {
        Some(snapshot) => match publish_and_backfill(target, table_id, engine, &snapshot).await {
            Ok(published_to) => debug!(published_to, "published + backfilled"),
            Err(e) => {
                warn!(error = %e, "best-effort publish/backfill failed; commit remains ratified")
            }
        },
        // The next write (or the 429 handler) catches up.
        None => debug!(version, "no post-commit snapshot; deferring publish"),
    }

    let request = DeltaReportMetricsRequest {
        table_id: table_id.to_string(),
        report: Some(DeltaReport {
            commit_report: Some(DeltaCommitReport {
                num_files_added: Some(stage.num_adds() as i64),
                num_files_removed: Some(stage.num_removes() as i64),
                num_rows_inserted: Some(options.num_rows_inserted),
                ..Default::default()
            }),
        }),
    };
    if let Err(e) = target
        .client
        .report_metrics(&target.catalog, &target.schema, &target.table, &request)
        .await
    {
        warn!(version, error = %e, "best-effort report_metrics failed");
    }
}

/// Publish `snapshot`'s unpublished catalog commits, then tell UC how far we published.
/// Returns the version published up to.
async fn publish_and_backfill(
    target: &ManagedTableRef,
    table_id: &str,
    engine: &dyn Engine,
    snapshot: &SnapshotRef,
) -> Result<i64, CreateManagedTableError> {
    // `publish` copies each staged commit to its published path (already-published files
    // are fine) and is a no-op when nothing is unpublished.
    let committer = target.committer(table_id);
    let published = snapshot.publish(engine, &committer)?;
    let published_to: i64 = published
        .version()
        .try_into()
        .map_err(|_| CreateManagedTableError::other("published version does not fit into i64"))?;

    let request = DeltaUpdateTableRequest {
        requirements: vec![DeltaTableRequirement::AssertTableUuid {
            uuid: table_id.to_string(),
        }],
        updates: vec![DeltaTableUpdate::SetLatestBackfilledVersion {
            latest_published_version: published_to,
        }],
    };
    target
        .client
        .update_table(&target.catalog, &target.schema, &target.table, &request)
        .await?;
    debug!(published_to, "set-latest-backfilled-version");
    Ok(published_to)
}

/// Publish and backfill the tail of a freshly loaded table (the 429 handler).
async fn publish_and_backfill_loaded(
    target: &ManagedTableRef,
    engine: &dyn Engine,
    loaded: &LoadedTable,
) -> Result<(), CreateManagedTableError> {
    let fresh = target.load().await?;
    let snapshot = fresh.snapshot(engine)?;
    debug_assert_eq!(fresh.table_id, loaded.table_id);
    publish_and_backfill(target, &fresh.table_id, engine, &snapshot).await?;
    Ok(())
}

/// Build a kernel snapshot from the catalog's ratified state (the catalog is the source
/// of truth). Shares [`resolve_managed_read_state`] with the read path, so the latest
/// version resolves identically (never from `metadata.last-commit-version`, review
/// finding A5). The table is known managed here, so `NotManaged` is an error.
fn build_snapshot(
    loaded: &DeltaLoadTableResponse,
    location: &Url,
    engine: &dyn Engine,
) -> Result<SnapshotRef, CreateManagedTableError> {
    let (commits, latest) = match resolve_managed_read_state(loaded)
        .map_err(|e| CreateManagedTableError::other(e.to_string()))?
    {
        ManagedReadState::Managed { commits, latest } => (commits, latest),
        ManagedReadState::NotManaged => {
            return Err(CreateManagedTableError::other(format!(
                "table at '{location}' is not catalog-managed; cannot build a managed snapshot"
            )));
        }
    };
    KernelSnapshot::builder_for(location.as_str())
        .with_log_tail(
            to_log_tail(location, &commits)
                .map_err(|e| CreateManagedTableError::other(e.to_string()))?,
        )
        .with_max_catalog_version(latest)
        .build(engine)
        .map_err(CreateManagedTableError::Kernel)
}

/// Jittered, capped exponential backoff between commit attempts.
async fn backoff(attempt: u32) {
    let exp = RETRY_BASE_BACKOFF.saturating_mul(1u32 << attempt.min(5));
    let capped = exp.min(RETRY_MAX_BACKOFF);
    // Deterministic jitter from the attempt count avoids pulling in an RNG dependency.
    let jitter = Duration::from_millis((attempt as u64 * 7) % 25);
    tokio::time::sleep(capped + jitter).await;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::arrow::array::{Int64Array, StringArray};
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use unitycatalog_client::{Error, UcApiError};
    use unitycatalog_delta_api::models::{DeltaErrorModel, DeltaErrorType};
    use url::Url;

    use super::*;

    fn delta_error(error_type: DeltaErrorType, code: u16) -> Error {
        Error::Delta(DeltaErrorModel {
            message: "x".into(),
            error_type,
            code,
            stack: None,
        })
    }

    #[test]
    fn definite_rejection_is_4xx_only() {
        assert!(is_definite_rejection(&delta_error(
            DeltaErrorType::NoSuchTableException,
            404
        )));
        assert!(is_definite_rejection(&Error::Api(
            UcApiError::PermissionDenied {
                message: "no".into()
            }
        )));
        assert!(!is_definite_rejection(&delta_error(
            DeltaErrorType::CommitStateUnknownException,
            500
        )));
        assert!(!is_definite_rejection(&Error::Generic(
            "connection reset".into()
        )));
    }

    #[test]
    fn add_paths_reads_the_path_column() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("path", DataType::Utf8, true),
            Field::new("size", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec![
                    Some("a.parquet"),
                    None,
                    Some("b.parquet"),
                ])),
                Arc::new(Int64Array::from(vec![1, 2, 3])),
            ],
        )
        .unwrap();
        assert_eq!(add_paths(&[batch]), vec!["a.parquet", "b.parquet"]);
    }

    #[test]
    fn commit_paths_resolve_under_the_table_root() {
        let location = Url::parse("file:///data/tbl/").unwrap();
        assert_eq!(
            staged_commit_path(&location, "00000000000000000003.abc.json")
                .unwrap()
                .as_ref(),
            "data/tbl/_delta_log/_staged_commits/00000000000000000003.abc.json"
        );
        assert_eq!(
            published_commit_path(&location, 3).unwrap().as_ref(),
            "data/tbl/_delta_log/00000000000000000003.json"
        );
    }

    /// Force a conflict: while the first attempt stages (its snapshot is at v0), another
    /// writer commits v1. Our v1 proposal conflicts, its staged commit is deleted, and the
    /// retry re-stages the same data file on v1 and lands as v2.
    #[tokio::test(flavor = "multi_thread")]
    async fn conflict_deletes_staged_commit_and_restages_same_files() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use delta_kernel_default_engine::DefaultEngineBuilder;

        use crate::managed::append::{ManagedWriteMode, write_with_store};
        use crate::managed::test_support::{self, CATALOG, SCHEMA, TestServer};

        let server = TestServer::start().await;
        let location = server.create_table("conflicts").await;
        let target = ManagedTableRef::new(server.delta(), CATALOG, SCHEMA, "conflicts");
        let store = test_support::local_store();
        let engine: ManagedEngine = DefaultEngineBuilder::new(store.clone()).build();

        // Write our data file against v0.
        let first = target.load().await.unwrap();
        let snapshot = first.snapshot(&engine).unwrap();
        let probe = snapshot
            .clone()
            .update_table(UpdateTableOperation::Write, "test", true)
            .build(&engine, Box::new(target.committer(&first.table_id)))
            .unwrap();
        let write_context = probe
            .write_state()
            .unwrap()
            .write_context_builder()
            .build()
            .unwrap();
        let add = engine
            .write_parquet(
                &ArrowEngineData::new(test_support::batch(&[1, 2])),
                &write_context,
            )
            .await
            .unwrap();
        let adds = vec![into_record_batch(add).unwrap()];

        let calls = AtomicUsize::new(0);
        let competitor = target.clone();
        let competitor_store = store.clone();
        let version = commit_managed(
            &target,
            &engine,
            &store,
            first,
            CommitOptions {
                operation: UpdateTableOperation::Write,
                engine_info: "test",
                data_change: true,
                num_rows_inserted: 2,
            },
            |_snapshot| {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    // The competing writer commits v1 before our first attempt does.
                    let handle = tokio::runtime::Handle::current();
                    tokio::task::block_in_place(|| {
                        handle.block_on(write_with_store(
                            &competitor,
                            competitor_store.clone(),
                            vec![test_support::batch(&[3])],
                            ManagedWriteMode::Append,
                            "competitor",
                        ))
                    })
                    .unwrap();
                }
                Ok(KernelTxnStage {
                    adds: adds.clone(),
                    removes: vec![],
                })
            },
        )
        .await
        .unwrap();

        assert_eq!(version, 2, "the retry lands after the competitor's v1");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "exactly one conflict and one retry"
        );
        // Our rejected v1 proposal was deleted: only the two ratified commits remain.
        assert_eq!(
            test_support::count_files(&location, "_delta_log/_staged_commits", "json"),
            2
        );
        // The retry re-staged our one data file: two data files in total.
        assert_eq!(test_support::count_files(&location, "", "parquet"), 2);
    }

    /// After a commit-state-unknown, a version that was published and dropped from the
    /// tail counts as ours only if the published file is byte-for-byte our staged one.
    #[tokio::test(flavor = "multi_thread")]
    async fn commit_landed_matches_published_bytes_not_version() {
        use object_store::PutPayload;

        use crate::managed::append::{ManagedWriteMode, write_with_store};
        use crate::managed::test_support::{self, CATALOG, SCHEMA, TestServer};

        let server = TestServer::start().await;
        let location = server.create_table("landed").await;
        let target = ManagedTableRef::new(server.delta(), CATALOG, SCHEMA, "landed");
        let store = test_support::local_store();
        // v1 lands, is published, and leaves UC's tail.
        write_with_store(
            &target,
            store.clone(),
            vec![test_support::batch(&[1])],
            ManagedWriteMode::Append,
            "test",
        )
        .await
        .unwrap();
        let loaded = target.load().await.unwrap();
        assert!(loaded.response.commits.as_deref().unwrap_or(&[]).is_empty());
        let staged_dir = location
            .to_file_path()
            .unwrap()
            .join("_delta_log/_staged_commits");
        let winner = std::fs::read_dir(&staged_dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name()
            .into_string()
            .unwrap();

        let committer = target.committer(&loaded.table_id);
        // Our proposal is the published v1: landed.
        committer.set_last_proposed(1, &winner);
        assert_eq!(
            commit_landed(&target, &committer, &store).await.unwrap(),
            Some(1)
        );

        // A different proposal for v1 (a lost race): latest >= 1, yet not ours.
        let loser = "00000000000000000001.00000000-0000-0000-0000-00000000dead.json";
        store
            .put(
                &staged_commit_path(&location, loser).unwrap(),
                PutPayload::from_static(b"{\"commitInfo\":{}}\n"),
            )
            .await
            .unwrap();
        committer.set_last_proposed(1, loser);
        assert_eq!(
            commit_landed(&target, &committer, &store).await.unwrap(),
            None
        );

        // A proposal beyond the latest version can't have landed.
        committer.set_last_proposed(2, loser);
        assert_eq!(
            commit_landed(&target, &committer, &store).await.unwrap(),
            None
        );
    }

    #[test]
    fn stage_counts_rows() {
        let schema = Arc::new(Schema::new(vec![Field::new("path", DataType::Utf8, true)]));
        let batch = RecordBatch::try_new(schema, vec![Arc::new(StringArray::from(vec!["a", "b"]))])
            .unwrap();
        let stage = KernelTxnStage {
            adds: vec![batch.clone()],
            removes: vec![batch.clone(), batch],
        };
        assert_eq!(stage.num_adds(), 2);
        assert_eq!(stage.num_removes(), 4);
    }
}
