//! The default Delta [`LogStore`] hydrofoil hands to the Delta kernel.
//!
//! [`DataFusionLogStore`] routes commits through the per-session object stores
//! (so credential isolation is preserved) and runs the kernel on the session's
//! `TaskContext`. The three read/abort/get-latest methods are the deprecated
//! legacy `LogStore` APIs in delta-rs; hydrofoil does not use them, so they
//! return a clean "not implemented" error rather than panicking.

use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use datafusion::execution::TaskContext;
use delta_kernel::Engine;
use deltalake_core::{
    DeltaResult, DeltaTableError,
    delta_datafusion::engine::DataFusionEngine,
    kernel::transaction::TransactionError,
    logstore::{CommitOrBytes, LogStore, LogStoreConfig, ObjectStoreRef, commit_uri_from_version},
};
use object_store::{Attributes, Error as ObjectStoreError, ObjectStore, PutOptions, TagSet};
use tracing::{error, instrument};
use uuid::Uuid;

/// Shared `PutOptions` for commit writes: create-only, so concurrent writers
/// conflict instead of clobbering an existing version.
fn put_options() -> &'static PutOptions {
    static PUT_OPTS: OnceLock<PutOptions> = OnceLock::new();
    PUT_OPTS.get_or_init(|| PutOptions {
        mode: object_store::PutMode::Create, // Creates if file doesn't exists yet
        tags: TagSet::default(),
        attributes: Attributes::default(),
        extensions: Default::default(),
    })
}

/// Default [`LogStore`] implementation
#[derive(Debug, Clone)]
pub struct DataFusionLogStore {
    prefixed_store: ObjectStoreRef,
    root_store: ObjectStoreRef,
    config: LogStoreConfig,
    ctx: Arc<TaskContext>,
}

impl DataFusionLogStore {
    /// Create a new instance of [`DataFusionLogStore`]
    pub(crate) fn new(
        prefixed_store: ObjectStoreRef,
        root_store: ObjectStoreRef,
        config: LogStoreConfig,
        ctx: Arc<TaskContext>,
    ) -> Arc<Self> {
        Arc::new(Self {
            prefixed_store,
            root_store,
            config,
            ctx,
        })
    }
}

#[async_trait::async_trait]
impl LogStore for DataFusionLogStore {
    fn name(&self) -> String {
        "DataFusionLogStore".into()
    }

    async fn read_commit_entry(&self, _version: u64) -> DeltaResult<Option<Bytes>> {
        // Deliberately unsupported: this is one of the deprecated/legacy `LogStore`
        // APIs in delta-rs that hydrofoil does not use. Return a clean error rather
        // than panicking; drop the override entirely once delta-rs removes it.
        Err(DeltaTableError::generic(
            "DataFusionLogStore::read_commit_entry is not implemented (deprecated LogStore API)",
        ))
    }

    #[instrument(skip_all, level = "info")]
    async fn write_commit_entry(
        &self,
        version: u64,
        commit_or_bytes: CommitOrBytes,
        _: Uuid,
    ) -> Result<(), TransactionError> {
        error!("using legacy log store APIs.");

        match commit_or_bytes {
            CommitOrBytes::LogBytes(log_bytes) => self
                .object_store(None)
                .put_opts(
                    &commit_uri_from_version(Some(version)),
                    log_bytes.into(),
                    put_options().clone(),
                )
                .await
                .map_err(|err| -> TransactionError {
                    match err {
                        ObjectStoreError::AlreadyExists { .. } => {
                            TransactionError::VersionAlreadyExists(version)
                        }
                        _ => TransactionError::from(err),
                    }
                })?,
            // Default log store should never get a tmp_commit, since this is for conditional put stores
            _ => unreachable!("unreachable in write_commit_entry"),
        };
        Ok(())
    }

    async fn abort_commit_entry(
        &self,
        _version: u64,
        _commit_or_bytes: CommitOrBytes,
        _: Uuid,
    ) -> Result<(), TransactionError> {
        // Deliberately unsupported (deprecated LogStore API); see read_commit_entry.
        Err(TransactionError::LogStoreError {
            msg: "DataFusionLogStore::abort_commit_entry is not implemented (deprecated LogStore API)"
                .to_string(),
            source: "not implemented".into(),
        })
    }

    async fn get_latest_version(&self, _current_version: u64) -> DeltaResult<u64> {
        // Deliberately unsupported (deprecated LogStore API); see read_commit_entry.
        Err(DeltaTableError::generic(
            "DataFusionLogStore::get_latest_version is not implemented (deprecated LogStore API)",
        ))
    }

    fn object_store(&self, _: Option<Uuid>) -> Arc<dyn ObjectStore> {
        self.prefixed_store.clone()
    }

    fn root_object_store(&self, _: Option<Uuid>) -> Arc<dyn ObjectStore> {
        self.root_store.clone()
    }

    fn config(&self) -> &LogStoreConfig {
        &self.config
    }

    fn engine(&self, _operation_id: Option<Uuid>) -> Arc<dyn Engine> {
        DataFusionEngine::new_from_context(self.ctx.clone())
    }
}
