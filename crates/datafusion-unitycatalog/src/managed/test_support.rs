//! An in-process Unity Catalog server for unit tests of the managed write path.

use std::sync::Arc;

use datafusion::arrow::array::{Int64Array, RecordBatch, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use object_store::ObjectStore;
use object_store::local::LocalFileSystem;
use olai_http::CloudClient;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use unitycatalog_client::{DeltaV1Client, UnityCatalogClient};
use unitycatalog_server::config::{
    Backend, Config, ConfigValue, LocalStorageConfig, SqliteBackendConfig,
};
use unitycatalog_server::embed;
use url::Url;

pub(crate) const CATALOG: &str = "unity";
pub(crate) const SCHEMA: &str = "default";

/// A UC server on a random loopback port, with catalog `unity` and schema `default`.
pub(crate) struct TestServer {
    pub client: UnityCatalogClient,
    _root: tempfile::TempDir,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestServer {
    pub(crate) async fn start() -> TestServer {
        let root = tempfile::tempdir().unwrap();
        // macOS: `/tmp` is a symlink; server and `LocalFileSystem` must agree on paths.
        let canonical = std::fs::canonicalize(root.path()).unwrap();
        let mut config = Config {
            backend: Backend::Sqlite(SqliteBackendConfig {
                path: ConfigValue::Value(":memory:".to_string()),
            }),
            local_storage: LocalStorageConfig {
                allowed_roots: vec![canonical.to_str().unwrap().to_string()],
            },
            managed_storage_root: Some(Url::from_directory_path(&canonical).unwrap().to_string()),
            ..Default::default()
        };
        config.ui.serve = false;
        let router = embed::start(&config).await.unwrap().router;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown, signal) = oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = signal.await;
                })
                .await
        });

        let base_url = Url::parse(&format!("http://{addr}/api/2.1/unity-catalog")).unwrap();
        let client = UnityCatalogClient::new(CloudClient::new_unauthenticated(), base_url);
        client.create_catalog(CATALOG).await.unwrap();
        client.create_schema(SCHEMA, CATALOG).await.unwrap();
        TestServer {
            client,
            _root: root,
            shutdown: Some(shutdown),
        }
    }

    pub(crate) fn delta(&self) -> Arc<DeltaV1Client> {
        Arc::new(self.client.delta_v1())
    }

    /// Create the managed table `name` with [`table_schema`]. Returns its location.
    pub(crate) async fn create_table(&self, name: &str) -> Url {
        super::create_managed_table(
            self.delta(),
            CATALOG,
            SCHEMA,
            name,
            table_schema(),
            vec![],
            "test",
        )
        .await
        .unwrap()
        .location
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

pub(crate) fn local_store() -> Arc<dyn ObjectStore> {
    Arc::new(LocalFileSystem::new())
}

pub(crate) fn table_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("name", DataType::Utf8, true),
    ]))
}

pub(crate) fn batch(ids: &[i64]) -> RecordBatch {
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

/// Number of entries in `<location>/<relative>` with `extension`.
pub(crate) fn count_files(location: &Url, relative: &str, extension: &str) -> usize {
    std::fs::read_dir(location.to_file_path().unwrap().join(relative))
        .map(|dir| {
            dir.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|x| x == extension))
                .count()
        })
        .unwrap_or(0)
}
