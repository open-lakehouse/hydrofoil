//! Hermetic round trip for a catalog-managed Delta table: write through the UC
//! `/delta/v1` staging flow, then read the rows back with DataFusion.
//!
//! The server is mangrove's `uc-server` service built in-process
//! (`olai_uc_server::embed`), on an in-memory SQLite backend with a temporary
//! `file://` managed storage root, so the test needs no services or credentials.
//!
//! The lifecycle (all via `/delta/v1/...`):
//! 1. **createStagingTable** → table id + managed `location` + required protocol.
//! 2. Write `_delta_log/0.json` (protocol + metaData) to the vended location.
//! 3. **createTable** → the server validates the contract and registers v0.
//! 4. Write a parquet data file + a staged commit `_staged_commits/<v>.<uuid>.json`,
//!    then **updateTable** `add-commit` so the catalog ratifies v1.
//! 5. **loadTable** → assemble a catalog-managed snapshot from the ratified tail and
//!    `SELECT *` through `DeltaScanNext`.
//!
//! This moved here from mangrove's acceptance crate together with this crate; the
//! write side still runs there, with a kernel-only read-back.
#![cfg(feature = "delta")]

use std::collections::BTreeMap;
use std::path::Path as FsPath;
use std::sync::Arc;

use datafusion::arrow::array::{Int64Array, RecordBatch, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::prelude::SessionContext;
use datafusion_unitycatalog::RoutingObjectStore;
use datafusion_unitycatalog::catalog::{
    ManagedReadState, build_catalog_managed_snapshot, ensure_trailing_slash,
    resolve_managed_read_state,
};
use deltalake_core::delta_datafusion::DeltaScanNext;
use deltalake_core::delta_datafusion::engine::DataFusionEngine;
use deltalake_core::logstore::{StorageConfig, default_logstore};
use deltalake_core::parquet::arrow::ArrowWriter;
use object_store::local::LocalFileSystem;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload, path::Path};
use olai_http::CloudClient;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use unitycatalog_client::UnityCatalogClient;
use unitycatalog_delta_api::models::{
    DeltaCommit, DeltaCreateStagingTableRequest, DeltaCreateTableRequest, DeltaDataType,
    DeltaProtocol, DeltaStructField, DeltaStructType, DeltaTableRequirement, DeltaTableType,
    DeltaTableUpdate, DeltaUpdateTableRequest,
};
use unitycatalog_server::config::{
    Backend, Config, ConfigValue, LocalStorageConfig, SqliteBackendConfig,
};
use unitycatalog_server::embed;
use url::Url;

const CATALOG: &str = "unity";
const SCHEMA: &str = "default";
/// Fixed epoch-ms so the hand-rolled commits are deterministic across runs.
const CREATED_TS: i64 = 1_704_067_200_000;

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
        Ok(Self {
            client,
            _root: root,
            shutdown: Some(shutdown),
        })
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// In-memory SQLite, anonymous auth, `root` as the only allowed local root and as
/// the metastore's managed storage root.
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
    // The bundled UI is a separate build feature; don't ask for it.
    config.ui.serve = false;
    Ok(config)
}

#[tokio::test(flavor = "multi_thread")]
async fn managed_table_write_then_datafusion_read() -> Result<()> {
    let server = TestServer::start().await?;
    let client = &server.client;
    client.create_catalog(CATALOG).await?;
    client.create_schema(SCHEMA, CATALOG).await?;

    let table = "roundtrip";
    let batches = create_commit_read(client, table).await?;

    let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(rows, 3);
    let ids: Vec<i64> = batches
        .iter()
        .flat_map(|b| {
            b.column_by_name("id")
                .expect("id column")
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("id is Int64")
                .values()
                .to_vec()
        })
        .collect();
    assert_eq!(ids, vec![1, 2, 3]);
    Ok(())
}

/// Create a managed table through the staging flow, commit one data file, and
/// `SELECT * ORDER BY id` it back through a catalog-managed snapshot.
async fn create_commit_read(client: &UnityCatalogClient, table: &str) -> Result<Vec<RecordBatch>> {
    let delta = client.delta_v1();

    // --- createStagingTable → table id + managed location. ---
    let staging = delta
        .create_staging_table(
            CATALOG,
            SCHEMA,
            &DeltaCreateStagingTableRequest {
                name: table.to_string(),
            },
        )
        .await?;
    let table_id = staging.table_id.clone();
    let location = Url::parse(&ensure_trailing_slash(&staging.location))?;
    assert_eq!(
        location.scheme(),
        "file",
        "the test server vends file:// locations"
    );
    let local_location = canonicalize_file_location(&location)?;

    let store: Arc<dyn ObjectStore> =
        Arc::new(LocalFileSystem::new().with_automatic_cleanup(false));
    let ctx = SessionContext::new();
    register_routing_store(&ctx, &local_location, store.clone())?;

    // Drive the protocol + properties from the staging response: the server
    // rejects a create that does not honor exactly what it advertised.
    let protocol = staging.required_protocol.clone();
    let writer_features = protocol.writer_features.clone().unwrap_or_default();
    let properties = required_properties(&staging.required_properties, &table_id, &writer_features);

    // --- _delta_log/0.json, then createTable at v0. ---
    store
        .put(
            &child_path(&local_location, "_delta_log/00000000000000000000.json"),
            PutPayload::from(
                build_zero_commit(&table_id, table, &protocol, &properties).into_bytes(),
            ),
        )
        .await?;
    delta
        .create_table(
            CATALOG,
            SCHEMA,
            &DeltaCreateTableRequest {
                name: table.to_string(),
                location: location.to_string(),
                table_type: DeltaTableType::Managed,
                data_source_format: None,
                comment: None,
                columns: managed_columns(),
                partition_columns: None,
                protocol: protocol.clone(),
                properties: properties.clone(),
                domain_metadata: None,
                last_commit_timestamp_ms: CREATED_TS,
                uniform: None,
            },
        )
        .await?;

    // --- commit v1: data file + staged commit + add-commit. ---
    let arrow_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("name", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        arrow_schema,
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec!["alice", "bob", "carol"])),
        ],
    )?;
    let data_file_name = "part-00000-roundtrip.parquet";
    let data_bytes = write_parquet(&batch)?;
    let data_size = data_bytes.len() as i64;
    store
        .put(
            &child_path(&local_location, data_file_name),
            PutPayload::from(data_bytes),
        )
        .await?;

    let commit_ts = CREATED_TS + 1000;
    let commit_file_name = "00000000000000000001.00000000-0000-0000-0000-0000000000a1.json";
    let commit_json = build_data_commit(1, commit_ts, data_file_name, data_size, batch.num_rows());
    let commit_size = commit_json.len() as i64;
    store
        .put(
            &child_path(
                &local_location,
                &format!("_delta_log/_staged_commits/{commit_file_name}"),
            ),
            PutPayload::from(commit_json.into_bytes()),
        )
        .await?;
    delta
        .update_table(
            CATALOG,
            SCHEMA,
            table,
            &DeltaUpdateTableRequest {
                requirements: vec![DeltaTableRequirement::AssertTableUuid {
                    uuid: table_id.clone(),
                }],
                updates: vec![DeltaTableUpdate::AddCommit {
                    commit: DeltaCommit {
                        version: 1,
                        timestamp: commit_ts,
                        file_name: commit_file_name.to_string(),
                        file_size: commit_size,
                        file_modification_timestamp: commit_ts,
                    },
                    uniform: None,
                }],
            },
        )
        .await?;

    // --- loadTable → catalog-managed snapshot → SELECT. ---
    let loaded = delta.load_table(CATALOG, SCHEMA, table).await?;
    let ManagedReadState::Managed { commits, latest } = resolve_managed_read_state(&loaded)? else {
        return Err("expected a catalog-managed table after createTable".into());
    };

    let config = StorageConfig::default();
    let prefixed = config.decorate_store(store.clone(), &local_location)?;
    let log_store = default_logstore(Arc::from(prefixed), store.clone(), &local_location, &config);
    let engine = DataFusionEngine::new_from_context(ctx.task_ctx());
    // The snapshot builder reads `_delta_log` through an internal `block_on`; against
    // real IO that must not run directly on a runtime worker, so hand the worker's
    // other tasks off first (needs the multi-thread runtime).
    let snapshot = tokio::task::block_in_place(|| {
        build_catalog_managed_snapshot(
            engine.clone(),
            &local_location,
            &commits,
            latest as i64,
            None,
        )
    })
    .await?;

    let provider = DeltaScanNext::builder()
        .with_snapshot(Arc::new(snapshot))
        .with_log_store(log_store)
        .await?;
    ctx.register_table(table, provider)?;
    Ok(ctx
        .sql(&format!("SELECT * FROM {table} ORDER BY id"))
        .await?
        .collect()
        .await?)
}

/// createTable `properties` from the staging response's `required_properties`:
/// `null` (server-chooses) values get a default, plus `io.unitycatalog.tableId` and
/// a `delta.feature.<name>=supported` flag per required writer feature.
fn required_properties(
    required: &BTreeMap<String, Option<String>>,
    table_id: &str,
    writer_features: &[String],
) -> BTreeMap<String, String> {
    let mut p: BTreeMap<String, String> = required
        .iter()
        .map(|(k, v)| (k.clone(), v.clone().unwrap_or_else(|| "true".to_string())))
        .collect();
    p.insert("io.unitycatalog.tableId".into(), table_id.into());
    for feat in writer_features {
        p.entry(format!("delta.feature.{feat}"))
            .or_insert_with(|| "supported".to_string());
    }
    p
}

fn managed_columns() -> DeltaStructType {
    let field = |name: &str, ty: &str| DeltaStructField {
        name: name.into(),
        data_type: DeltaDataType::Primitive(ty.into()),
        nullable: true,
        metadata: Default::default(),
    };
    DeltaStructType {
        type_tag: Default::default(),
        fields: vec![field("id", "long"), field("name", "string")],
    }
}

/// `_delta_log/0.json`: commitInfo + protocol + metaData, from the values
/// negotiated with the server.
fn build_zero_commit(
    table_id: &str,
    table_name: &str,
    protocol: &DeltaProtocol,
    properties: &BTreeMap<String, String>,
) -> String {
    let schema_string = json!({
        "type": "struct",
        "fields": [
            {"name": "id", "type": "long", "nullable": true, "metadata": {}},
            {"name": "name", "type": "string", "nullable": true, "metadata": {}}
        ]
    })
    .to_string();
    let commit_info = json!({
        "commitInfo": {
            "timestamp": CREATED_TS,
            "inCommitTimestamp": CREATED_TS,
            "operation": "CREATE TABLE",
            "operationParameters": {}
        }
    });
    let protocol = json!({
        "protocol": {
            "minReaderVersion": protocol.min_reader_version,
            "minWriterVersion": protocol.min_writer_version,
            "readerFeatures": protocol.reader_features.clone().unwrap_or_default(),
            "writerFeatures": protocol.writer_features.clone().unwrap_or_default()
        }
    });
    let metadata = json!({
        "metaData": {
            "id": table_id,
            "name": table_name,
            "format": {"provider": "parquet", "options": {}},
            "schemaString": schema_string,
            "partitionColumns": [],
            "configuration": properties,
            "createdTime": CREATED_TS
        }
    });
    format!("{commit_info}\n{protocol}\n{metadata}\n")
}

fn build_data_commit(
    version: i64,
    commit_ts: i64,
    data_file_name: &str,
    data_size: i64,
    num_rows: usize,
) -> String {
    let commit_info = json!({
        "commitInfo": {
            "timestamp": commit_ts,
            "inCommitTimestamp": commit_ts,
            "operation": "WRITE",
            "operationParameters": {"mode": "Append"},
            "operationMetrics": {"numFiles": "1", "numOutputRows": num_rows.to_string()},
            "version": version
        }
    });
    let add = json!({
        "add": {
            "path": data_file_name,
            "partitionValues": {},
            "size": data_size,
            "modificationTime": commit_ts,
            "dataChange": true
        }
    });
    format!("{commit_info}\n{add}\n")
}

/// `object_store::Path` for `<table-path>/<rel>` on a `LocalFileSystem` rooted at `/`.
fn child_path(location: &Url, rel: &str) -> Path {
    let base = location.path().trim_matches('/');
    Path::from(format!("{base}/{rel}"))
}

/// The vended location canonicalized for local I/O (`LocalFileSystem` resolves
/// symlinks on read). Creates the table directory first, since it doesn't exist yet.
fn canonicalize_file_location(location: &Url) -> Result<Url> {
    let dir = location
        .to_file_path()
        .map_err(|()| format!("location {location} is not a local file path"))?;
    std::fs::create_dir_all(&dir)?;
    let canonical = std::fs::canonicalize(&dir)?;
    Ok(Url::from_directory_path(&canonical)
        .map_err(|()| format!("canonical path {} is not a URL", canonical.display()))?)
}

fn write_parquet(batch: &RecordBatch) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut writer = ArrowWriter::try_new(&mut buf, batch.schema(), None)?;
    writer.write(batch)?;
    writer.close()?;
    Ok(buf)
}

/// Register a path-dispatching routing store under the `file://` bucket key, the
/// way the UC resolver does.
fn register_routing_store(
    ctx: &SessionContext,
    location: &Url,
    store: Arc<dyn ObjectStore>,
) -> Result<()> {
    let router = RoutingObjectStore::new();
    router.register(
        Path::from_url_path(location.path()).unwrap_or_default(),
        store,
    );
    ctx.runtime_env()
        .register_object_store(&Url::parse("file:///")?, Arc::new(router));
    Ok(())
}
