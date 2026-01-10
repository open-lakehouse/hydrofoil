use std::sync::{Arc, OnceLock};

use arrow::array::RecordBatch;
use bytes::Bytes;
use datafusion::{
    catalog::Session,
    error::Result,
    execution::{SessionStateBuilder, TaskContext},
    prelude::{SessionConfig, SessionContext},
};
use datafusion_tracing::{
    InstrumentationOptions, instrument_with_info_spans, pretty_format_compact_batch,
};
use delta_kernel::Engine;
use deltalake_core::{
    DeltaResult,
    logstore::{ObjectStoreRef, commit_uri_from_version},
};
use deltalake_core::{
    Path,
    delta_datafusion::engine::AsObjectStoreUrl as _,
    logstore::{LogStore, StorageConfig, logstore_with},
};
use deltalake_core::{delta_datafusion::engine::DataFusionEngine, logstore::LogStoreConfig};
use deltalake_core::{kernel::transaction::TransactionError, logstore::CommitOrBytes};
use instrumented_object_store::instrument_object_store;
use object_store::{Attributes, Error as ObjectStoreError, ObjectStore, PutOptions, TagSet};
use object_store::{aws::AmazonS3Builder, client::SpawnedReqwestConnector, prefix::PrefixStore};
use tokio::runtime::Handle;
use tracing::{error, instrument, warn};
use url::Url;
use uuid::Uuid;

use crate::external_tables::DeltaTableFactory;

pub trait SessionExt {
    /// Get a Delta [`LogStore`] for the given location
    ///
    /// # Arguments
    ///
    /// * `location` - The URL location of the Delta table root
    ///   (i.e., where the `_delta_log` directory is located)
    fn delta_logstore_for(&self, location: &Url) -> Result<Arc<dyn LogStore>>;
}

impl<S: Session + ?Sized> SessionExt for S {
    fn delta_logstore_for(&self, location: &Url) -> Result<Arc<dyn LogStore>> {
        let object_store_url = location.as_object_store_url();
        let root_store = self.runtime_env().object_store(object_store_url)?;
        let table_path = Path::from_url_path(location.path())?;
        let prefixed_store = Arc::new(PrefixStore::new(root_store.clone(), table_path));
        let storage_config = StorageConfig::default();
        Ok(
            logstore_with(root_store.clone(), location, storage_config.clone()).unwrap_or_else(
                |_| {
                    warn!(
                        "No registered log store factory for scheme '{}'. Using default.",
                        location.scheme()
                    );
                    DataFusionLogStore::new(
                        prefixed_store,
                        root_store,
                        LogStoreConfig::new(location, storage_config),
                        self.task_ctx(),
                    )
                },
            ),
        )
    }
}

pub fn create_session(session_id: impl Into<Option<Uuid>>) -> Result<SessionContext> {
    let options = InstrumentationOptions::builder()
        .record_metrics(true)
        .preview_limit(5)
        .preview_fn(Arc::new(|batch: &RecordBatch| {
            pretty_format_compact_batch(batch, 64, 3, 10).map(|fmt| fmt.to_string())
        }))
        .build();

    let instrument_rule = instrument_with_info_spans!(
        options: options,
    );

    let session_config = SessionConfig::from_env()?.with_information_schema(true);
    let session_state = SessionStateBuilder::new_with_default_features()
        .with_session_id(session_id.into().unwrap_or_else(Uuid::new_v4).to_string())
        .with_config(session_config)
        .with_table_factory(
            DeltaTableFactory::FILE_FORMAT.to_string(),
            DeltaTableFactory::new(),
        )
        .with_physical_optimizer_rule(instrument_rule)
        .build();
    let ctx = SessionContext::new_with_state(session_state);

    update_session(&ctx.state())?;

    Ok(ctx)
}

fn update_session(session: &dyn Session) -> Result<()> {
    add_seaweedfs(session, Handle::current())?;
    Ok(())
}

fn add_seaweedfs(session: &dyn Session, handle: Handle) -> Result<()> {
    let object_store = Arc::new(
        AmazonS3Builder::new()
            .with_http_connector(SpawnedReqwestConnector::new(handle))
            .with_access_key_id("seaweed-key-id")
            .with_secret_access_key("seaweed-access-key")
            .with_endpoint("http://localhost:8333/")
            .with_bucket_name("open-lakehouse")
            .with_allow_http(true)
            .build()?,
    );
    let instrumented = instrument_object_store(object_store, "seaweedfs");
    let url = url::Url::parse("s3://open-lakehouse/").unwrap();
    session
        .runtime_env()
        .register_object_store(&url, instrumented);
    Ok(())
}

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
    /// Create a new instance of [`DefaultLogStore`]
    fn new(
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

    async fn read_commit_entry(&self, _version: i64) -> DeltaResult<Option<Bytes>> {
        todo!("read_commit_entry")
    }

    #[instrument(skip_all, level = "info")]
    async fn write_commit_entry(
        &self,
        version: i64,
        commit_or_bytes: CommitOrBytes,
        _: Uuid,
    ) -> Result<(), TransactionError> {
        error!("using legacy log store APIs.");

        match commit_or_bytes {
            CommitOrBytes::LogBytes(log_bytes) => self
                .object_store(None)
                .put_opts(
                    &commit_uri_from_version(version),
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
            _ => unreachable!(), // Default log store should never get a tmp_commit, since this is for conditional put stores
        };
        Ok(())
    }

    async fn abort_commit_entry(
        &self,
        _version: i64,
        _commit_or_bytes: CommitOrBytes,
        _: Uuid,
    ) -> Result<(), TransactionError> {
        todo!("abort_commit_entry")
    }

    async fn get_latest_version(&self, _current_version: i64) -> DeltaResult<i64> {
        todo!("not get_latest_version")
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
