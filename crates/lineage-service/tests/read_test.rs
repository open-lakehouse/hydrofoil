//! End-to-end tests for the Marquez-compatible read layer: write known events to
//! a temp Delta table, then assert each derived endpoint reconstructs the
//! expected model and lineage graph.

use std::sync::Arc;

use deltalake::arrow::array::{RecordBatch, StringArray, TimestampMicrosecondArray};

use lineage_service::config::{Config, DeltaConfig};
use lineage_service::read::LineageStore;
use lineage_service::writer::delta::DeltaWriter;
use lineage_service::writer::schema::arrow_schema;

fn local_config(path: &str) -> Config {
    Config {
        delta: DeltaConfig {
            table_path: path.to_string(),
            partition_cols: vec![],
            ..DeltaConfig::default()
        },
        ..Config::default()
    }
}

/// One `run` event row: a job in `etl` reading `raw.orders` and writing
/// `marts.daily_orders`.
fn run_event_batch() -> RecordBatch {
    let schema = arrow_schema();
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["run"])), // event_kind
            Arc::new(StringArray::from(vec![Some("COMPLETE")])), // event_type
            Arc::new(
                // event_time
                TimestampMicrosecondArray::from(vec![1_700_000_000_000_000i64])
                    .with_timezone("UTC"),
            ),
            Arc::new(StringArray::from(vec!["test-producer"])), // producer
            Arc::new(StringArray::from(vec![None::<&str>])),    // schema_url
            Arc::new(StringArray::from(vec![Some("run-1")])),   // run_id
            Arc::new(StringArray::from(vec![Some("etl")])),     // job_namespace
            Arc::new(StringArray::from(vec![Some("build_daily")])), // job_name
            Arc::new(StringArray::from(vec![None::<&str>])),    // dataset_namespace
            Arc::new(StringArray::from(vec![None::<&str>])),    // dataset_name
            Arc::new(StringArray::from(vec![None::<&str>])),    // facets_json
            Arc::new(StringArray::from(vec![Some(
                // inputs_json
                r#"[{"namespace":"raw","name":"orders"}]"#,
            )])),
            Arc::new(StringArray::from(vec![Some(
                // outputs_json
                r#"[{"namespace":"marts","name":"daily_orders"}]"#,
            )])),
            Arc::new(StringArray::from(vec![None::<&str>])), // column_lineage_json
            Arc::new(StringArray::from(vec![None::<&str>])), // raw_json
        ],
    )
    .unwrap()
}

async fn seeded_store() -> (tempfile::TempDir, LineageStore) {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = local_config(tmp.path().to_str().unwrap());
    DeltaWriter::new(&cfg)
        .append(run_event_batch())
        .await
        .unwrap();
    let store = LineageStore::from_config(&cfg);
    (tmp, store)
}

#[tokio::test]
async fn namespaces_lists_job_and_dataset_namespaces() {
    let (_tmp, store) = seeded_store().await;
    let ns = store.namespaces().await.unwrap();
    let names: Vec<&str> = ns.namespaces.iter().map(|n| n.name.as_str()).collect();
    assert!(names.contains(&"etl"), "job namespace present: {names:?}");
    assert!(
        names.contains(&"raw"),
        "input ds namespace present: {names:?}"
    );
    assert!(
        names.contains(&"marts"),
        "output ds namespace present: {names:?}"
    );
}

#[tokio::test]
async fn jobs_returns_job_with_inputs_and_outputs() {
    let (_tmp, store) = seeded_store().await;
    let jobs = store.jobs(Some("etl"), 100, 0).await.unwrap();
    assert_eq!(jobs.total_count, 1);
    let job = &jobs.jobs[0];
    assert_eq!(job.name, "build_daily");
    assert_eq!(job.inputs.len(), 1);
    assert_eq!(job.inputs[0].namespace, "raw");
    assert_eq!(job.inputs[0].name, "orders");
    assert_eq!(job.outputs[0].name, "daily_orders");
    // The dashboard's JobRunItem reduces over latestRuns without an initial
    // value, so it must never be empty.
    assert_eq!(job.latest_runs.len(), 1, "exactly one synthetic run");
}

#[tokio::test]
async fn datasets_include_job_referenced_tables() {
    let (_tmp, store) = seeded_store().await;
    let raw = store.datasets(Some("raw"), 100, 0).await.unwrap();
    assert_eq!(raw.datasets.len(), 1);
    assert_eq!(raw.datasets[0].name, "orders");

    let marts = store.datasets(Some("marts"), 100, 0).await.unwrap();
    assert_eq!(marts.datasets[0].name, "daily_orders");
}

#[tokio::test]
async fn lineage_graph_connects_job_to_its_datasets() {
    let (_tmp, store) = seeded_store().await;
    let node = "job:etl:build_daily";
    let graph = store.lineage(node, 2).await.unwrap();

    // Seed job + one input + one output dataset reachable within 2 hops.
    let ids: Vec<&str> = graph.graph.iter().map(|n| n.id.as_str()).collect();
    assert!(ids.contains(&node), "seed job present: {ids:?}");
    assert!(
        ids.contains(&"dataset:raw:orders"),
        "input present: {ids:?}"
    );
    assert!(
        ids.contains(&"dataset:marts:daily_orders"),
        "output present: {ids:?}"
    );

    // The job node should have one in-edge (from input) and one out-edge (to output).
    let job = graph.graph.iter().find(|n| n.id == node).unwrap();
    assert_eq!(job.node_type, "JOB");
    assert_eq!(job.in_edges.len(), 1);
    assert_eq!(job.in_edges[0].origin, "dataset:raw:orders");
    assert_eq!(job.out_edges.len(), 1);
    assert_eq!(job.out_edges[0].destination, "dataset:marts:daily_orders");
}

#[tokio::test]
async fn search_matches_job_and_dataset_names() {
    let (_tmp, store) = seeded_store().await;
    let hits = store.search("orders", 100).await.unwrap();
    // Matches both "raw.orders" and "marts.daily_orders".
    assert!(hits.total_count >= 2, "got {} hits", hits.total_count);
    assert!(hits.results.iter().all(|r| r.result_type == "DATASET"));
}

#[tokio::test]
async fn empty_table_yields_empty_results() {
    // Point at a path with no table — should be empty, not an error.
    let tmp = tempfile::tempdir().unwrap();
    let cfg = local_config(tmp.path().join("nonexistent").to_str().unwrap());
    let store = LineageStore::from_config(&cfg);
    let ns = store.namespaces().await.unwrap();
    assert!(ns.namespaces.is_empty());
}

#[tokio::test]
async fn missing_job_is_not_found() {
    let (_tmp, store) = seeded_store().await;
    let err = store.job("etl", "nope").await.unwrap_err();
    assert!(matches!(err, lineage_service::read::ReadError::NotFound(_)));
}

// --- HTTP-level tests ---------------------------------------------------------
// These drive the read router the way the Marquez UI does, so they catch issues
// the direct store-call tests can't: route-pattern syntax (axum 0.7 uses
// `:param`, not `{param}`) and the camelCase JSON envelope keys.

use http_body_util::BodyExt;
use lineage_service::read::http::router as read_router;
use tower::ServiceExt; // for `oneshot`

async fn get(store: LineageStore, uri: &str) -> (axum::http::StatusCode, String) {
    let req = axum::http::Request::builder()
        .uri(uri)
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = read_router(store).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn http_namespaced_jobs_route_resolves() {
    // Regression guard for the axum-0.7 `:param` route syntax: a `{param}` route
    // would 404 here even though the namespace has jobs.
    let (_tmp, store) = seeded_store().await;
    let (status, body) = get(store, "/api/v1/namespaces/etl/jobs?limit=25&offset=0").await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    assert!(body.contains("build_daily"), "body: {body}");
    // Envelope key must be camelCase for the Marquez UI.
    assert!(body.contains("\"totalCount\""), "body: {body}");
}

#[tokio::test]
async fn http_global_jobs_route_resolves() {
    let (_tmp, store) = seeded_store().await;
    let (status, body) = get(store, "/api/v1/jobs?limit=25&offset=0").await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    assert!(body.contains("build_daily"), "body: {body}");
}

#[tokio::test]
async fn http_job_runs_returns_synthetic_run() {
    let (_tmp, store) = seeded_store().await;
    let (status, body) = get(
        store,
        "/api/v1/namespaces/etl/jobs/build_daily/runs?limit=14&offset=0",
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    assert!(body.contains("\"runs\""), "body: {body}");
    assert!(body.contains("\"totalCount\""), "body: {body}");
    assert!(body.contains("COMPLETED"), "body: {body}");
}

#[tokio::test]
async fn http_lineage_node_edges_are_camel_case() {
    // The UI's graph layout reads node.inEdges / node.outEdges; snake_case keys
    // make those `undefined` and crash `findDownstreamNodes`.
    let (_tmp, store) = seeded_store().await;
    let (status, body) = get(store, "/api/v1/lineage?nodeId=job:etl:build_daily&depth=2").await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    assert!(body.contains("\"outEdges\""), "body: {body}");
    assert!(body.contains("\"inEdges\""), "body: {body}");
    assert!(!body.contains("\"out_edges\""), "snake_case leaked: {body}");
}

#[tokio::test]
async fn http_search_envelope_is_camel_case() {
    let (_tmp, store) = seeded_store().await;
    let (status, body) = get(store, "/api/v1/search?q=orders&limit=100").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(body.contains("\"totalCount\""), "body: {body}");
    assert!(body.contains("\"nodeId\""), "body: {body}");
}
