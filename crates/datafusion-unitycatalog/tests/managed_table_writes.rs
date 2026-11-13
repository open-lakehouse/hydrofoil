//! Hermetic tests for catalog-managed writes through the kernel commit layer.
//!
//! Every test runs mangrove's UC server in-process (in-memory SQLite, a temporary
//! `file://` managed storage root), creates the table with `create_managed_table`, and
//! commits through the real `UnityCatalogCommitter`: staged commit, `add-commit`
//! ratification, publish, backfill.
#![cfg(feature = "delta")]

use std::path::Path as FsPath;
use std::sync::Arc;

use datafusion::arrow::array::{Int64Array, RecordBatch, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::prelude::SessionContext;
use datafusion_unitycatalog::RoutingObjectStore;
use datafusion_unitycatalog::catalog::{
    ManagedReadState, build_catalog_managed_snapshot, ensure_trailing_slash,
    resolve_managed_read_state,
};
use datafusion_unitycatalog::managed::{
    ManagedDeltaTableProvider, ManagedWriteMode, create_managed_table,
    write_managed_table_with_store,
};
use deltalake_core::delta_datafusion::DeltaScanNext;
use deltalake_core::delta_datafusion::engine::DataFusionEngine;
use deltalake_core::logstore::{StorageConfig, default_logstore};
use object_store::ObjectStore;
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use olai_http::CloudClient;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use unitycatalog_client::{DeltaV1Client, UnityCatalogClient};
use unitycatalog_server::config::{
    Backend, Config, ConfigValue, LocalStorageConfig, SqliteBackendConfig,
};
use unitycatalog_server::embed;
use url::Url;

const CATALOG: &str = "unity";
const SCHEMA: &str = "default";
const ENGINE_INFO: &str = "managed-table-writes-test";

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// An in-process UC server on a random loopback port. Stops when dropped.
struct TestServer {
    client: UnityCatalogClient,
    _root: tempfile::TempDir,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestServer {
    async fn start() -> Result<Self> {
        let root = tempfile::tempdir()?;
        // On macOS `/tmp` is a symlink to `/private/tmp`; the server and
        // `LocalFileSystem` must agree on one spelling of every path.
        let canonical = std::fs::canonicalize(root.path())?;
        let router = embed::start(&server_config(&canonical)?).await?.router;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let (shutdown, signal) = oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = signal.await;
                })
                .await
        });

        let base_url = Url::parse(&format!("http://{addr}/api/2.1/unity-catalog"))?;
        let client = UnityCatalogClient::new(CloudClient::new_unauthenticated(), base_url);
        client.create_catalog(CATALOG).await?;
        client.create_schema(SCHEMA, CATALOG).await?;
        Ok(Self {
            client,
            _root: root,
            shutdown: Some(shutdown),
        })
    }

    fn delta(&self) -> Arc<DeltaV1Client> {
        Arc::new(self.client.delta_v1())
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

fn server_config(root: &FsPath) -> Result<Config> {
    let root_str = root.to_str().ok_or("storage root is not UTF-8")?;
    let root_url = Url::from_directory_path(root).map_err(|()| "storage root is not absolute")?;
    let mut config = Config {
        backend: Backend::Sqlite(SqliteBackendConfig {
            path: ConfigValue::Value(":memory:".to_string()),
        }),
        local_storage: LocalStorageConfig {
            allowed_roots: vec![root_str.to_string()],
        },
        managed_storage_root: Some(root_url.to_string()),
        ..Default::default()
    };
    config.ui.serve = false;
    Ok(config)
}

/// The bucket-rooted store every `file://` table is written and read through.
fn local_store() -> Arc<dyn ObjectStore> {
    Arc::new(LocalFileSystem::new())
}

fn table_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("name", DataType::Utf8, true),
    ]))
}

fn batch(ids: &[i64]) -> RecordBatch {
    let names: Vec<String> = ids.iter().map(|i| format!("row-{i}")).collect();
    RecordBatch::try_new(
        table_schema(),
        vec![
            Arc::new(Int64Array::from(ids.to_vec())),
            Arc::new(StringArray::from(names)),
        ],
    )
    .unwrap()
}

/// Create the managed table `name`. Returns its storage location.
async fn create(server: &TestServer, name: &str) -> Result<Url> {
    let table = create_managed_table(
        server.delta(),
        CATALOG,
        SCHEMA,
        name,
        table_schema(),
        vec![],
        ENGINE_INFO,
    )
    .await?;
    Ok(table.location)
}

async fn write(
    server: &TestServer,
    name: &str,
    ids: &[i64],
    mode: ManagedWriteMode,
) -> Result<u64> {
    Ok(write_managed_table_with_store(
        server.delta(),
        CATALOG,
        SCHEMA,
        name,
        local_store(),
        vec![batch(ids)],
        mode,
        ENGINE_INFO,
    )
    .await?)
}

/// A session with `name` registered as a writable managed table at the catalog's
/// current version.
async fn session_with_table(server: &TestServer, name: &str) -> Result<SessionContext> {
    let delta = server.client.delta_v1();
    let loaded = delta.load_table(CATALOG, SCHEMA, name).await?;
    let ManagedReadState::Managed { commits, latest } = resolve_managed_read_state(&loaded)? else {
        return Err("expected a catalog-managed table".into());
    };
    let location = Url::parse(&ensure_trailing_slash(&loaded.metadata.location))?;

    let ctx = SessionContext::new();
    let router = RoutingObjectStore::new();
    router.register(
        Path::from_url_path(location.path()).unwrap_or_default(),
        local_store(),
    );
    ctx.runtime_env()
        .register_object_store(&Url::parse("file:///")?, Arc::new(router));

    let config = StorageConfig::default();
    let store = local_store();
    let prefixed = config.decorate_store(store.clone(), &location)?;
    let log_store = default_logstore(Arc::from(prefixed), store, &location, &config);
    let engine = DataFusionEngine::new_from_context(ctx.task_ctx());
    let snapshot = tokio::task::block_in_place(|| {
        build_catalog_managed_snapshot(engine, &location, &commits, latest as i64, None)
    })
    .await?;
    let read = DeltaScanNext::builder()
        .with_snapshot(Arc::new(snapshot))
        .with_log_store(log_store)
        .await?;
    let provider = ManagedDeltaTableProvider::new(read, CATALOG, SCHEMA, name)
        .with_write_store(server.delta(), local_store());
    ctx.register_table(name, Arc::new(provider))?;
    Ok(ctx)
}

/// All `id`s of `name`, sorted, read through a fresh catalog-managed snapshot.
async fn ids(server: &TestServer, name: &str) -> Result<Vec<i64>> {
    let ctx = session_with_table(server, name).await?;
    let batches = ctx
        .sql(&format!("SELECT id FROM {name} ORDER BY id"))
        .await?
        .collect()
        .await?;
    Ok(batches
        .iter()
        .flat_map(|b| {
            b.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("id is Int64")
                .values()
                .to_vec()
        })
        .collect())
}

async fn latest_and_backfilled(server: &TestServer, name: &str) -> Result<(i64, Option<i64>)> {
    let loaded = server
        .client
        .delta_v1()
        .load_table(CATALOG, SCHEMA, name)
        .await?;
    let latest = loaded.latest_table_version.ok_or("no latest version")?;
    let unpublished_min = loaded
        .commits
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|c| c.version)
        .min();
    Ok((latest, unpublished_min))
}

fn published_commit(location: &Url, version: u64) -> std::path::PathBuf {
    location
        .to_file_path()
        .unwrap()
        .join(format!("_delta_log/{version:020}.json"))
}

#[tokio::test(flavor = "multi_thread")]
async fn append_commits_publishes_and_reads_back() -> Result<()> {
    let server = TestServer::start().await?;
    let location = create(&server, "appends").await?;

    assert_eq!(
        write(&server, "appends", &[1, 2], ManagedWriteMode::Append).await?,
        1
    );
    assert_eq!(
        write(&server, "appends", &[3], ManagedWriteMode::Append).await?,
        2
    );

    assert_eq!(ids(&server, "appends").await?, vec![1, 2, 3]);
    // Both commits were published, and UC dropped them from its unpublished tail.
    assert!(published_commit(&location, 1).exists());
    assert!(published_commit(&location, 2).exists());
    let (latest, unpublished) = latest_and_backfilled(&server, "appends").await?;
    assert_eq!(latest, 2);
    assert_eq!(unpublished, None, "every commit is backfilled");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn overwrite_replaces_all_rows() -> Result<()> {
    let server = TestServer::start().await?;
    create(&server, "overwrites").await?;

    write(&server, "overwrites", &[1, 2], ManagedWriteMode::Append).await?;
    write(&server, "overwrites", &[3], ManagedWriteMode::Append).await?;
    let version = write(
        &server,
        "overwrites",
        &[10, 11],
        ManagedWriteMode::Overwrite,
    )
    .await?;
    assert_eq!(version, 3);
    assert_eq!(ids(&server, "overwrites").await?, vec![10, 11]);

    // An append after the overwrite sees only the overwritten state.
    write(&server, "overwrites", &[12], ManagedWriteMode::Append).await?;
    assert_eq!(ids(&server, "overwrites").await?, vec![10, 11, 12]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_appends_both_land() -> Result<()> {
    let server = TestServer::start().await?;
    let location = create(&server, "concurrent").await?;

    let (a, b) = tokio::join!(
        write(&server, "concurrent", &[1, 2], ManagedWriteMode::Append),
        write(&server, "concurrent", &[3, 4], ManagedWriteMode::Append),
    );
    let mut versions = vec![a?, b?];
    versions.sort();
    assert_eq!(versions, vec![1, 2]);
    assert_eq!(ids(&server, "concurrent").await?, vec![1, 2, 3, 4]);

    // A conflicted attempt's staged commit is deleted, so only the two winners remain,
    // and the loser re-staged its data file instead of writing a second one.
    let staged = std::fs::read_dir(
        location
            .to_file_path()
            .unwrap()
            .join("_delta_log/_staged_commits"),
    )?
    .count();
    assert_eq!(staged, 2, "one staged commit per ratified version");
    let data_files = std::fs::read_dir(location.to_file_path().unwrap())?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "parquet"))
        .count();
    assert_eq!(data_files, 2, "each write produced exactly one data file");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn sql_insert_into_and_overwrite() -> Result<()> {
    let server = TestServer::start().await?;
    create(&server, "sql_writes").await?;

    let ctx = session_with_table(&server, "sql_writes").await?;
    ctx.sql("INSERT INTO sql_writes VALUES (1, 'a'), (2, 'b')")
        .await?
        .collect()
        .await?;
    assert_eq!(ids(&server, "sql_writes").await?, vec![1, 2]);

    let ctx = session_with_table(&server, "sql_writes").await?;
    ctx.sql("INSERT OVERWRITE sql_writes VALUES (7, 'z')")
        .await?
        .collect()
        .await?;
    assert_eq!(ids(&server, "sql_writes").await?, vec![7]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_write_leaves_no_orphaned_files() -> Result<()> {
    let server = TestServer::start().await?;
    let location = create(&server, "rejected").await?;

    // A batch whose schema does not match the table fails before anything is proposed.
    let wrong = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "other",
            DataType::Int64,
            true,
        )])),
        vec![Arc::new(Int64Array::from(vec![1]))],
    )?;
    let result = write_managed_table_with_store(
        server.delta(),
        CATALOG,
        SCHEMA,
        "rejected",
        local_store(),
        vec![wrong],
        ManagedWriteMode::Append,
        ENGINE_INFO,
    )
    .await;
    assert!(result.is_err());
    let data_files = std::fs::read_dir(location.to_file_path().unwrap())?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "parquet"))
        .count();
    assert_eq!(data_files, 0);
    assert_eq!(latest_and_backfilled(&server, "rejected").await?.0, 0);
    Ok(())
}
