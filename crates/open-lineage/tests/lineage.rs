//! Integration tests for the OpenLineage DataFusion integration.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use datafusion::physical_plan::ExecutionPlanProperties;
use datafusion::prelude::SessionContext;
use datafusion_open_lineage::builder::{complete_event, fail_event, start_event};
use datafusion_open_lineage::config::OpenLineageConfig;
use datafusion_open_lineage::context::LineageContext;
use datafusion_open_lineage::event::{RunEvent, RunEventType};
use datafusion_open_lineage::extract::extract;
use datafusion_open_lineage::transport::{Transport, TransportError};
use datafusion_open_lineage::{
    instrument_session_state_simple, LineageContextProvider, OpenLineageClient,
};
use serde_json::Value;
use uuid::Uuid;

/// A transport that records every event it receives, for assertions.
#[derive(Debug, Default, Clone)]
struct RecordingTransport {
    events: Arc<Mutex<Vec<RunEvent>>>,
}

#[async_trait]
impl Transport for RecordingTransport {
    async fn emit(&self, event: &RunEvent) -> Result<(), TransportError> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
}

/// A transport that always errors — to prove failures are swallowed.
#[derive(Debug)]
struct FailingTransport;

#[async_trait]
impl Transport for FailingTransport {
    async fn emit(&self, _event: &RunEvent) -> Result<(), TransportError> {
        Err(TransportError::Other("boom".to_string()))
    }
}

fn config() -> OpenLineageConfig {
    OpenLineageConfig {
        job_namespace: "test-ns".to_string(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// 1. Spec-conformant JSON
// ---------------------------------------------------------------------------

#[tokio::test]
async fn start_event_serializes_to_spec_shape() {
    let ctx = SessionContext::new();
    ctx.sql("CREATE TABLE t (a INT, b INT) AS VALUES (1, 2)")
        .await
        .unwrap();
    let plan = ctx
        .state()
        .create_logical_plan("SELECT a, b FROM t")
        .await
        .unwrap();
    let optimized = ctx.state().optimize(&plan).unwrap();

    let lineage = extract(&optimized, &config());
    let event = start_event(Uuid::now_v7(), &lineage, &LineageContext::default(), &config());

    let json: Value = serde_json::to_value(&event).unwrap();
    // Envelope.
    assert_eq!(json["eventType"], "START");
    assert!(json["producer"].is_string());
    assert!(json["schemaURL"].is_string());
    // Every facet carries _producer + _schemaURL.
    let pe = &json["run"]["facets"]["processing_engine"];
    assert_eq!(pe["name"], "DataFusion");
    assert!(pe["_producer"].is_string());
    assert!(pe["_schemaURL"].is_string());
    // Job type facet.
    assert_eq!(json["job"]["facets"]["jobType"]["integration"], "DATAFUSION");
}

// ---------------------------------------------------------------------------
// 2. Extraction: inputs, schema, column lineage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn extract_simple_projection() {
    let ctx = SessionContext::new();
    ctx.sql("CREATE TABLE t (a INT, b INT) AS VALUES (1, 2)")
        .await
        .unwrap();
    let plan = ctx
        .state()
        .create_logical_plan("SELECT a, b FROM t")
        .await
        .unwrap();
    let optimized = ctx.state().optimize(&plan).unwrap();

    let lineage = extract(&optimized, &config());
    assert_eq!(lineage.inputs.len(), 1, "one input table");
    assert_eq!(lineage.inputs[0].fields.len(), 2, "two schema fields");
    // Identity columns map back to the source.
    assert!(lineage.column_lineage.contains_key("a"));
    let a = &lineage.column_lineage["a"];
    assert_eq!(a[0].field.as_deref(), Some("a"));
}

#[tokio::test]
async fn extract_insert_has_output() {
    let ctx = SessionContext::new();
    ctx.sql("CREATE TABLE src (a INT) AS VALUES (1)")
        .await
        .unwrap();
    ctx.sql("CREATE TABLE dst (a INT) AS VALUES (0)")
        .await
        .unwrap();
    let plan = ctx
        .state()
        .create_logical_plan("INSERT INTO dst SELECT a FROM src")
        .await
        .unwrap();
    let optimized = ctx.state().optimize(&plan).unwrap();

    let lineage = extract(&optimized, &config());
    assert!(!lineage.inputs.is_empty(), "src is an input");
    assert_eq!(lineage.outputs.len(), 1, "dst is an output");
    assert!(lineage.outputs[0].name.name.contains("dst"));
}

// ---------------------------------------------------------------------------
// 3. Transport / client: START + COMPLETE share run id, failures swallowed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn start_and_complete_share_run_id() {
    let lineage = Default::default();
    let cx = LineageContext::default();
    let cfg = config();
    let run_id = Uuid::now_v7();

    let start = start_event(run_id, &lineage, &cx, &cfg);
    let complete = complete_event(run_id, &lineage, &cx, &cfg);
    let fail = fail_event(run_id, &lineage, &cx, &cfg, "kaboom");

    assert_eq!(start.run.run_id, complete.run.run_id);
    assert_eq!(start.run.run_id, fail.run.run_id);
    assert_eq!(start.event_type, RunEventType::Start);
    assert_eq!(complete.event_type, RunEventType::Complete);
    assert_eq!(fail.event_type, RunEventType::Fail);
    // Error facet present on FAIL only.
    let fail_json = serde_json::to_value(&fail).unwrap();
    assert_eq!(fail_json["run"]["facets"]["errorMessage"]["message"], "kaboom");
}

#[tokio::test]
async fn failing_transport_does_not_panic() {
    let client = OpenLineageClient::new(Arc::new(FailingTransport));
    client.emit(start_event(
        Uuid::now_v7(),
        &Default::default(),
        &LineageContext::default(),
        &config(),
    ));
    // Give the worker a moment to process; the failing emit must be swallowed.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}

#[tokio::test]
async fn end_to_end_emits_start_and_complete() {
    let transport = RecordingTransport::default();
    let client = OpenLineageClient::new(Arc::new(transport.clone()));

    // Build an instrumented context from a fresh state, then create the table
    // inside it so there is exactly one catalog.
    let base = SessionContext::new();
    let state = instrument_session_state_simple(base.state(), client, config());
    let instrumented = SessionContext::new_with_state(state);
    instrumented
        .sql("CREATE TABLE t (a INT, b INT) AS VALUES (1, 2)")
        .await
        .unwrap();

    let df = instrumented.sql("SELECT a FROM t").await.unwrap();
    let _ = df.collect().await.unwrap();

    // Allow the background emit worker to drain.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let events = transport.events.lock().unwrap();
    let types: Vec<RunEventType> = events.iter().map(|e| e.event_type).collect();
    assert!(types.contains(&RunEventType::Start), "got START: {types:?}");
    assert!(
        types.contains(&RunEventType::Complete),
        "got COMPLETE: {types:?}"
    );
}

#[tokio::test]
async fn complete_fires_only_after_execution() {
    let transport = RecordingTransport::default();
    let client = OpenLineageClient::new(Arc::new(transport.clone()));

    let base = SessionContext::new();
    let state = instrument_session_state_simple(base.state(), client, config());
    let instrumented = SessionContext::new_with_state(state);
    instrumented
        .sql("CREATE TABLE t (a INT) AS VALUES (1)")
        .await
        .unwrap();

    // Plan + physical plan, but do NOT consume the stream yet.
    let df = instrumented.sql("SELECT a FROM t").await.unwrap();
    let plan = df.create_physical_plan().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Identify the run for THIS query by its START (setup CREATE TABLE produced
    // its own events). The select's run must have START but not yet COMPLETE.
    let run_id = {
        let events = transport.events.lock().unwrap();
        let start = events
            .iter()
            .rev()
            .find(|e| e.event_type == RunEventType::Start)
            .expect("a START for the select");
        let id = start.run.run_id;
        let completes = events
            .iter()
            .filter(|e| e.run.run_id == id && e.event_type == RunEventType::Complete)
            .count();
        assert_eq!(completes, 0, "COMPLETE must NOT fire before execution");
        id
    };

    // Now execute (drain all partitions) -> exactly one COMPLETE for this run.
    let task_ctx = instrumented.task_ctx();
    let partitions = plan.output_partitioning().partition_count();
    for p in 0..partitions {
        let mut stream = plan.execute(p, task_ctx.clone()).unwrap();
        use futures::StreamExt;
        while stream.next().await.is_some() {}
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let events = transport.events.lock().unwrap();
    let completes = events
        .iter()
        .filter(|e| e.run.run_id == run_id && e.event_type == RunEventType::Complete)
        .count();
    assert_eq!(completes, 1, "exactly one COMPLETE across partitions");
}

#[tokio::test]
async fn runtime_error_emits_fail() {
    let transport = RecordingTransport::default();
    let client = OpenLineageClient::new(Arc::new(transport.clone()));

    let base = SessionContext::new();
    let state = instrument_session_state_simple(base.state(), client, config());
    let instrumented = SessionContext::new_with_state(state);
    instrumented
        .sql("CREATE TABLE t (a INT) AS VALUES (1), (2), (0)")
        .await
        .unwrap();

    // Division by zero surfaces as an error during stream execution, not at
    // planning time -> the run must report FAIL, not COMPLETE.
    let df = instrumented.sql("SELECT 10 / a AS r FROM t").await.unwrap();
    let result = df.collect().await;
    assert!(result.is_err(), "query should fail at runtime");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let events = transport.events.lock().unwrap();
    // Correlate by the failing run (setup CREATE TABLE has its own run id).
    let fail = events
        .iter()
        .find(|e| e.event_type == RunEventType::Fail)
        .expect("a FAIL event");
    let run_id = fail.run.run_id;
    let for_run: Vec<RunEventType> = events
        .iter()
        .filter(|e| e.run.run_id == run_id)
        .map(|e| e.event_type)
        .collect();
    assert!(for_run.contains(&RunEventType::Start), "START: {for_run:?}");
    assert!(
        !for_run.contains(&RunEventType::Complete),
        "no COMPLETE on runtime failure: {for_run:?}"
    );
    // The FAIL event carries an errorMessage facet.
    let json = serde_json::to_value(fail).unwrap();
    assert!(json["run"]["facets"]["errorMessage"]["message"].is_string());
}

// ---------------------------------------------------------------------------
// 4. Context provider injects orchestration metadata
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct FixedContextProvider(LineageContext);

#[async_trait]
impl LineageContextProvider for FixedContextProvider {
    async fn context(
        &self,
        _state: &datafusion::execution::context::SessionState,
    ) -> LineageContext {
        self.0.clone()
    }
}

#[tokio::test]
async fn context_run_id_flows_to_event() {
    let fixed_run = Uuid::now_v7();
    let cx = LineageContext {
        run_id: Some(fixed_run),
        job_name: Some("airflow.my_dag.my_task".to_string()),
        ..Default::default()
    };

    let event = start_event(
        cx.run_id.unwrap(),
        &Default::default(),
        &cx,
        &config(),
    );
    assert_eq!(event.run.run_id, fixed_run);
    assert_eq!(event.job.name, "airflow.my_dag.my_task");
    // Provider trait is object-safe and usable.
    let provider: Arc<dyn LineageContextProvider> = Arc::new(FixedContextProvider(cx));
    let _ = format!("{provider:?}");
}
