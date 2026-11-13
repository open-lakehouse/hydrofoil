//! Guards the kernel declarative-plan path that hydrofoil gets from delta-rs's
//! DataFusion executor (`deltalake-kernel-executor`, re-exported as
//! `deltalake_core::delta_datafusion::engine::DataFusionEngine`).
//!
//! Every snapshot hydrofoil loads through `DataFusionEngine` runs kernel's
//! protocol + metadata replay as a plan on DataFusion, because the engine
//! exposes a `PlanExecutor`. These tests build a local table that crosses a
//! checkpoint and an overwrite, then check that:
//!
//! - the engine exposes a plan executor;
//! - a snapshot loads through it (the P&M plan path);
//! - kernel's declarative metadata scan plan runs on it and yields the same
//!   live files as the classic `scan_metadata` replay.
//!
//! See `docs/adr/0018-kernel-plan-executor-from-delta-rs.md`.

#![cfg(feature = "delta")]

use std::sync::Arc;

use datafusion::arrow::array::{Int32Array, RecordBatch, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::prelude::SessionContext;
use delta_kernel::Snapshot;
use delta_kernel::plans::{Operation, PlanResult};
use deltalake_core::DeltaTableBuilder;
use deltalake_core::delta_datafusion::engine::DataFusionEngine;
use deltalake_core::protocol::checkpoints::create_checkpoint;
use url::Url;

fn batch(ids: &[i32], vals: &[&str]) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("val", DataType::Utf8, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int32Array::from(ids.to_vec())),
            Arc::new(StringArray::from(vals.to_vec())),
        ],
    )
    .unwrap()
}

/// A table under `dir`: v0 and v1 appends, a checkpoint at v1, and a v2
/// overwrite that tombstones the earlier files. Returns its `file://` root.
async fn build_table(dir: &std::path::Path) -> Url {
    let url = Url::from_directory_path(dir).unwrap();
    let table = DeltaTableBuilder::from_url(url.clone())
        .unwrap()
        .build()
        .unwrap();
    let table = table
        .write(vec![batch(&[1, 2], &["a", "b"])])
        .await
        .unwrap();
    let table = table.write(vec![batch(&[3], &["c"])]).await.unwrap();
    create_checkpoint(&table).await.unwrap();
    table
        .write(vec![batch(&[4, 5], &["d", "e"])])
        .with_save_mode(deltalake_core::protocol::SaveMode::Overwrite)
        .await
        .unwrap();
    url
}

fn engine() -> Arc<DataFusionEngine> {
    DataFusionEngine::new_from_context(SessionContext::new().task_ctx())
}

#[tokio::test(flavor = "multi_thread")]
async fn datafusion_engine_exposes_a_plan_executor() {
    use delta_kernel::Engine as _;
    assert!(
        engine().plan_executor().is_some(),
        "DataFusionEngine must hand kernel its PlanExecutor, or snapshot loads fall back to \
         the classic handlers"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_loads_through_the_plan_executor() {
    let dir = tempfile::tempdir().unwrap();
    let url = build_table(dir.path()).await;
    let engine = engine();

    let snapshot = Snapshot::builder_for(url.as_str())
        .build(engine.as_ref())
        .unwrap();
    assert_eq!(snapshot.version(), 2);
    let schema = snapshot.schema();
    let names: Vec<_> = schema.fields().map(|f| f.name().as_str()).collect();
    assert_eq!(names, ["id", "val"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn declarative_scan_plan_matches_classic_replay() {
    use delta_kernel::Engine as _;

    let dir = tempfile::tempdir().unwrap();
    let url = build_table(dir.path()).await;
    let engine = engine();
    let snapshot = Snapshot::builder_for(url.as_str())
        .build(engine.as_ref())
        .unwrap();
    let scan = snapshot.scan_builder().build().unwrap();

    // Classic replay: count the selected (live) files.
    let classic: usize = scan
        .scan_metadata(engine.as_ref())
        .unwrap()
        .map(|metadata| {
            let metadata = metadata.unwrap();
            let (data, selection) = metadata.scan_files.into_parts();
            (0..data.len())
                .filter(|&row| selection.get(row).copied().unwrap_or(true))
                .count()
        })
        .sum();
    assert!(classic > 0, "the overwrite leaves live files");

    // Declarative: the plan's rows are the live add actions.
    let plan = scan
        .declarative_metadata_scan_plan(engine.as_ref())
        .unwrap()
        .expect("a table with live files has a metadata plan");
    let executor = engine.plan_executor().unwrap();
    let PlanResult::Data(data) = executor.execute_op(Operation::QueryPlan(plan)).unwrap() else {
        panic!("a query plan returns data");
    };
    let declarative: usize = data.map(|batch| batch.unwrap().len()).sum();
    assert_eq!(declarative, classic);
}
