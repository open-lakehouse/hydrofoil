//! Physical-plan wrapper that emits COMPLETE / FAIL at end of execution.
//!
//! The [`crate::planner::OpenLineageQueryPlanner`] emits START at plan time and
//! wraps the root physical plan in an [`OpenLineageExec`]. This node observes
//! the result streams and emits exactly one terminal event once every output
//! partition has finished:
//!
//! - COMPLETE when all partitions drain successfully;
//! - FAIL (with an `errorMessage` run facet) if any partition yields an error
//!   or is dropped before its stream is exhausted (e.g. a cancelled query).
//!
//! Completion is tracked with a `Drop`-based guard so cancellation is handled
//! without special-casing, and the terminal event fires under the *same*
//! `runId` the START used.

use std::any::Any;
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll};

use datafusion::arrow::array::RecordBatch;
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::error::Result;
use datafusion::execution::{RecordBatchStream, SendableRecordBatchStream, TaskContext};
use datafusion::physical_plan::{
    DisplayAs, DisplayFormatType, ExecutionPlan, PlanProperties,
};
use datafusion::physical_plan::metrics::MetricsSet;
use futures::Stream;

use crate::client::OpenLineageClient;
use crate::event::{RunEvent, RunEventType};
use crate::facets::{BaseFacet, ErrorMessageRunFacet};

const ERROR_FACET: &str = "1-0-0/ErrorMessageRunFacet.json";

/// Shared completion state across the partitions of one query run.
#[derive(Debug)]
struct RunState {
    client: OpenLineageClient,
    /// COMPLETE event template (cloned and mutated into FAIL on error).
    complete: RunEvent,
    producer: String,
    /// Outstanding partitions yet to finish.
    remaining: AtomicUsize,
    /// Set if any partition observed an error or was dropped early.
    failed: AtomicBool,
    /// First error message observed, for the FAIL facet.
    error: std::sync::Mutex<Option<String>>,
    /// Guards against emitting more than once (e.g. zero-partition plans).
    emitted: AtomicBool,
}

impl RunState {
    fn record_error(&self, message: String) {
        self.failed.store(true, Ordering::SeqCst);
        let mut slot = self.error.lock().unwrap();
        if slot.is_none() {
            *slot = Some(message);
        }
    }

    /// Mark one partition finished; emit the terminal event when the last one
    /// completes. Safe to call once per partition (including from `Drop`).
    fn partition_finished(&self) {
        // `remaining` starts at the partition count; the partition that brings
        // it to zero emits.
        if self.remaining.fetch_sub(1, Ordering::SeqCst) != 1 {
            return;
        }
        self.emit_terminal();
    }

    fn emit_terminal(&self) {
        if self.emitted.swap(true, Ordering::SeqCst) {
            return;
        }
        if self.failed.load(Ordering::SeqCst) {
            let mut event = self.complete.clone();
            event.event_type = RunEventType::Fail;
            let message = self
                .error
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| "query failed".to_string());
            event.run.facets.error_message = Some(ErrorMessageRunFacet {
                base: BaseFacet::new(&self.producer, ERROR_FACET),
                message,
                programming_language: "Rust".to_string(),
                stack_trace: None,
            });
            self.client.emit(event);
        } else {
            self.client.emit(self.complete.clone());
        }
    }
}

/// Wraps the root physical plan, emitting a terminal lineage event when
/// execution finishes.
pub struct OpenLineageExec {
    inner: Arc<dyn ExecutionPlan>,
    state: Arc<RunState>,
}

impl OpenLineageExec {
    /// Wrap `inner`, emitting COMPLETE (or FAIL on error) once all partitions
    /// finish. `complete` is the pre-built COMPLETE event (sharing the run id
    /// used by START); `producer` builds the error facet on failure.
    pub fn new(
        inner: Arc<dyn ExecutionPlan>,
        client: OpenLineageClient,
        complete: RunEvent,
        producer: String,
    ) -> Arc<Self> {
        let partitions = inner.properties().output_partitioning().partition_count();
        let state = Arc::new(RunState {
            client,
            complete,
            producer,
            // A plan may have zero partitions; guard so we still emit once.
            remaining: AtomicUsize::new(partitions.max(1)),
            failed: AtomicBool::new(false),
            error: std::sync::Mutex::new(None),
            emitted: AtomicBool::new(false),
        });
        Arc::new(Self { inner, state })
    }

    fn with_new_inner(&self, inner: Arc<dyn ExecutionPlan>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            state: self.state.clone(),
        })
    }
}

impl fmt::Debug for OpenLineageExec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenLineageExec").finish_non_exhaustive()
    }
}

impl DisplayAs for OpenLineageExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => {
                write!(f, "OpenLineageExec")
            }
            DisplayFormatType::TreeRender => write!(f, "OpenLineageExec"),
        }
    }
}

impl ExecutionPlan for OpenLineageExec {
    fn name(&self) -> &str {
        "OpenLineageExec"
    }

    fn as_any(&self) -> &dyn Any {
        // Transparent downcasting: callers see the inner plan's type.
        self.inner.as_any()
    }

    fn properties(&self) -> &Arc<PlanProperties> {
        self.inner.properties()
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![&self.inner]
    }

    fn with_new_children(
        self: Arc<Self>,
        mut children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        // We wrap a single root; rewrap whatever child we're given so the node
        // stays installed across optimizer/child rewrites.
        let child = children.pop().unwrap_or_else(|| self.inner.clone());
        Ok(self.with_new_inner(child))
    }

    fn metrics(&self) -> Option<MetricsSet> {
        self.inner.metrics()
    }

    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        let inner = self.inner.execute(partition, context)?;
        Ok(Box::pin(TrackedStream {
            schema: inner.schema(),
            inner,
            state: self.state.clone(),
            done: false,
        }))
    }
}

/// Wraps a partition's stream, recording errors and signalling completion on
/// terminal (exhaustion, error, or drop).
struct TrackedStream {
    schema: SchemaRef,
    inner: SendableRecordBatchStream,
    state: Arc<RunState>,
    done: bool,
}

impl TrackedStream {
    fn finish(&mut self) {
        if !self.done {
            self.done = true;
            self.state.partition_finished();
        }
    }
}

impl Stream for TrackedStream {
    type Item = Result<RecordBatch>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(batch))) => Poll::Ready(Some(Ok(batch))),
            Poll::Ready(Some(Err(e))) => {
                self.state.record_error(e.to_string());
                self.finish();
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                self.finish();
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl RecordBatchStream for TrackedStream {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }
}

impl Drop for TrackedStream {
    fn drop(&mut self) {
        // A stream dropped before exhaustion means the partition was cancelled
        // or abandoned: count it as a failure for the run.
        if !self.done {
            self.state
                .record_error("query stream dropped before completion".to_string());
            self.finish();
        }
    }
}
