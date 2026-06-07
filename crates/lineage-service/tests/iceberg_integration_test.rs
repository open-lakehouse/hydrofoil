//! End-to-end integration test for `IcebergSink` against a running Lakekeeper
//! REST catalog. These tests are `#[ignore]`d by default — they run only
//! when invoked explicitly with `cargo test -- --ignored` AND when the
//! `LAKEKEEPER_URL` environment variable is set.
//!
//! The expected stack is brought up by `just stack-up-iceberg` from the repo
//! root, which runs Lakekeeper + Postgres + MinIO via Compose and bootstraps
//! a `lineage` warehouse pointing at MinIO.
//!
//! Run with:
//!     just stack-up-iceberg
//!     just wait-healthy-iceberg
//!     just test-iceberg-integration

use std::sync::Arc;

use deltalake::arrow::array::{RecordBatch, StringArray, TimestampMicrosecondArray};
use table_service::config::IcebergConfig;
use table_service::writer::iceberg::IcebergSink;
use table_service::writer::schema::arrow_schema;
use table_service::writer::sink::TableSink;

/// Skip when Lakekeeper isn't reachable. Tests that depend on the live
/// catalog should call this at the top and `return` early on `None`.
fn lakekeeper_uri() -> Option<String> {
    std::env::var("LAKEKEEPER_URL")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Generate a unique namespace per test run so reruns don't collide on a
/// pre-existing table that the previous run committed.
fn unique_ns(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}_{nanos}")
}

fn one_row(kind: &str) -> RecordBatch {
    let schema = arrow_schema();
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![kind])),
            Arc::new(StringArray::from(vec![Some("START")])),
            Arc::new(
                TimestampMicrosecondArray::from(vec![1_700_000_000_000_000i64])
                    .with_timezone("UTC"),
            ),
            Arc::new(StringArray::from(vec!["test-producer"])),
            Arc::new(StringArray::from(vec![None::<&str>])),
            Arc::new(StringArray::from(vec![Some("run-1")])),
            Arc::new(StringArray::from(vec![Some("ns")])),
            Arc::new(StringArray::from(vec![Some("integration_job")])),
            Arc::new(StringArray::from(vec![None::<&str>])),
            Arc::new(StringArray::from(vec![None::<&str>])),
            Arc::new(StringArray::from(vec![None::<&str>])),
            Arc::new(StringArray::from(vec![None::<&str>])),
            Arc::new(StringArray::from(vec![None::<&str>])),
            Arc::new(StringArray::from(vec![None::<&str>])),
            Arc::new(StringArray::from(vec![Some("{}")])),
        ],
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "requires a live Lakekeeper at $LAKEKEEPER_URL (just stack-up-iceberg)"]
async fn integration_create_and_append_against_lakekeeper() {
    let Some(catalog_uri) = lakekeeper_uri() else {
        eprintln!("LAKEKEEPER_URL not set — skipping live integration test");
        return;
    };

    let namespace = unique_ns("it_create_append");
    let cfg = IcebergConfig {
        catalog_uri,
        warehouse: std::env::var("ICEBERG_WAREHOUSE").unwrap_or_else(|_| "lineage".into()),
        namespace: namespace.clone(),
        table: "events".into(),
        partition_cols: vec![],
        token: std::env::var("ICEBERG_TOKEN")
            .ok()
            .filter(|s| !s.is_empty()),
    };

    let sink = IcebergSink::from_config(&cfg)
        .await
        .expect("build IcebergSink against Lakekeeper");

    sink.append(one_row("run"))
        .await
        .expect("first append should create namespace + table and commit");
    sink.append(one_row("job"))
        .await
        .expect("second append should commit a new snapshot");
}

#[tokio::test]
#[ignore = "requires a live Lakekeeper at $LAKEKEEPER_URL (just stack-up-iceberg)"]
async fn integration_partitioned_table_against_lakekeeper() {
    let Some(catalog_uri) = lakekeeper_uri() else {
        eprintln!("LAKEKEEPER_URL not set — skipping live integration test");
        return;
    };

    let namespace = unique_ns("it_partitioned");
    let cfg = IcebergConfig {
        catalog_uri,
        warehouse: std::env::var("ICEBERG_WAREHOUSE").unwrap_or_else(|_| "lineage".into()),
        namespace,
        table: "events".into(),
        partition_cols: vec!["event_kind".into()],
        token: std::env::var("ICEBERG_TOKEN")
            .ok()
            .filter(|s| !s.is_empty()),
    };

    let sink = IcebergSink::from_config(&cfg)
        .await
        .expect("build partitioned IcebergSink against Lakekeeper");

    sink.append(one_row("run"))
        .await
        .expect("partitioned append should succeed");
    sink.append(one_row("job"))
        .await
        .expect("partitioned append for a different partition should succeed");
}

/// Exercises the bearer-token auth path end-to-end: requires both
/// `LAKEKEEPER_URL` and a non-empty `ICEBERG_TOKEN`. When the token is wrong
/// Lakekeeper rejects the request and `from_config`/`append` surfaces an error;
/// when correct the create-and-append path succeeds exactly as the unauth'd
/// case does. Skipped unless both env vars are present.
#[tokio::test]
#[ignore = "requires a live Lakekeeper at $LAKEKEEPER_URL + $ICEBERG_TOKEN (just stack-up-iceberg)"]
async fn auth_token_create_and_append_against_lakekeeper() {
    let Some(catalog_uri) = lakekeeper_uri() else {
        eprintln!("LAKEKEEPER_URL not set — skipping live auth integration test");
        return;
    };
    let Some(token) = std::env::var("ICEBERG_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
    else {
        eprintln!("ICEBERG_TOKEN not set — skipping live auth integration test");
        return;
    };

    let namespace = unique_ns("it_auth_create_append");
    let cfg = IcebergConfig {
        catalog_uri,
        warehouse: std::env::var("ICEBERG_WAREHOUSE").unwrap_or_else(|_| "lineage".into()),
        namespace,
        table: "events".into(),
        partition_cols: vec![],
        token: Some(token),
    };

    let sink = IcebergSink::from_config(&cfg)
        .await
        .expect("build IcebergSink against Lakekeeper with a bearer token");

    sink.append(one_row("run"))
        .await
        .expect("authenticated append should create namespace + table and commit");
    sink.append(one_row("job"))
        .await
        .expect("authenticated second append should commit a new snapshot");
}

#[tokio::test]
#[ignore = "requires a live Lakekeeper at $LAKEKEEPER_URL (just stack-up-iceberg)"]
async fn integration_empty_batch_is_a_noop_against_lakekeeper() {
    let Some(catalog_uri) = lakekeeper_uri() else {
        eprintln!("LAKEKEEPER_URL not set — skipping live integration test");
        return;
    };

    let namespace = unique_ns("it_empty");
    let cfg = IcebergConfig {
        catalog_uri,
        warehouse: std::env::var("ICEBERG_WAREHOUSE").unwrap_or_else(|_| "lineage".into()),
        namespace,
        table: "events".into(),
        partition_cols: vec![],
        token: std::env::var("ICEBERG_TOKEN")
            .ok()
            .filter(|s| !s.is_empty()),
    };

    let sink = IcebergSink::from_config(&cfg)
        .await
        .expect("build IcebergSink against Lakekeeper");

    let empty = RecordBatch::new_empty(arrow_schema());
    sink.append(empty)
        .await
        .expect("empty batch must not error");
}
