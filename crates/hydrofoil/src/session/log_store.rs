//! The default Delta [`LogStore`] hydrofoil hands to the Delta kernel.
//!
//! [`DataFusionLogStore`] routes commits through the per-session object stores
//! (so credential isolation is preserved) and runs the kernel on the session's
//! `TaskContext`.

use std::sync::Arc;

use bytes::Bytes;
use datafusion::execution::TaskContext;
use delta_kernel::Engine;
use deltalake_core::{
    DeltaResult,
    delta_datafusion::engine::DataFusionEngine,
    kernel::Version,
    logstore::{
        self, CommitStrategy, Committer, FileSystemCommitter, LogStore, LogStoreConfig,
        ObjectStoreRef,
    },
};
use object_store::ObjectStore;

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

    async fn read_commit_entry(&self, version: Version) -> DeltaResult<Option<Bytes>> {
        logstore::read_commit_entry(self.prefixed_store.as_ref(), version).await
    }

    async fn get_latest_version(&self, current_version: Version) -> DeltaResult<Version> {
        logstore::get_latest_version(self, current_version).await
    }

    fn object_store(&self) -> Arc<dyn ObjectStore> {
        self.prefixed_store.clone()
    }

    fn root_object_store(&self) -> Arc<dyn ObjectStore> {
        self.root_store.clone()
    }

    fn config(&self) -> &LogStoreConfig {
        &self.config
    }

    fn engine(&self) -> Arc<dyn Engine> {
        DataFusionEngine::new_from_context(self.ctx.clone())
    }

    /// Commits are create-only (put-if-absent) writes through the per-session
    /// table store, so concurrent writers conflict instead of clobbering an
    /// existing version.
    fn committer(&self) -> Arc<dyn Committer> {
        Arc::new(FileSystemCommitter::new(
            self.prefixed_store.clone(),
            CommitStrategy::ConditionalPut,
        ))
    }
}
