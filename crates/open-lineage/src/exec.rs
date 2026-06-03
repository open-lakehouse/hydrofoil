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
use crate::facets::{BaseFacet, ErrorMessageRunFacet, OutputStatisticsOutputDatasetFacet};

const ERROR_FACET: &str = "1-0-0/ErrorMessageRunFacet.json";
const OUTPUT_STATS_FACET: &str = "1-0-2/OutputStatisticsOutputDatasetFacet.json";

/// Shared completion state across the partitions of one query run.
struct RunState {
    client: OpenLineageClient,
    /// COMPLETE event template (cloned and mutated into FAIL on error).
    complete: RunEvent,
    producer: String,
    /// The wrapped plan, read for native runtime metrics on completion. Tracked
    /// through `with_new_children` rewrites so metrics come from the node that
    /// actually executed.
    inner: std::sync::Mutex<Arc<dyn ExecutionPlan>>,
    /// Outstanding partitions yet to finish.
    remaining: AtomicUsize,
    /// Set if any partition observed an error or was dropped early.
    failed: AtomicBool,
    /// First error message observed, for the FAIL facet.
    error: std::sync::Mutex<Option<String>>,
    /// Rows written, summed from DataFusion's write-result `count` batches
    /// (`Some` once any write count is observed).
    rows_written: std::sync::Mutex<Option<u64>>,
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

    /// Accumulate rows written, observed from a write-result `count` batch.
    fn record_rows_written(&self, rows: u64) {
        let mut slot = self.rows_written.lock().unwrap();
        *slot = Some(slot.unwrap_or(0) + rows);
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
            let mut event = self.complete.clone();
            self.attach_output_statistics(&mut event);
            self.client.emit(event);
        }
    }

    /// Attach an `outputStatistics` facet to each output dataset of the COMPLETE
    /// event.
    ///
    /// The row count comes from DataFusion's write-result `count` batch (the
    /// authoritative rows-written signal, captured as the stream drained). Size
    /// is taken from a `bytes_scanned`-style plan metric when available. Reads
    /// (SELECT) have no output dataset, so this is a no-op there.
    fn attach_output_statistics(&self, event: &mut RunEvent) {
        if event.outputs.is_empty() {
            return;
        }

        let row_count = self.rows_written.lock().unwrap().map(|n| n as i64);

        // `bytes_scanned` is the closest widely-emitted size metric; absent for
        // many plans, in which case `size` is simply omitted.
        let size = self
            .inner
            .lock()
            .unwrap()
            .metrics()
            .and_then(|m| m.aggregate_by_name().sum_by_name("bytes_scanned"))
            .map(|v| v.as_usize() as i64);

        if row_count.is_none() && size.is_none() {
            return;
        }

        let stats = OutputStatisticsOutputDatasetFacet {
            base: BaseFacet::new(&self.producer, OUTPUT_STATS_FACET),
            row_count,
            size,
            file_count: None,
        };
        for output in &mut event.outputs {
            let facets = output.output_facets.get_or_insert_with(Default::default);
            facets.output_statistics = Some(stats.clone());
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
            inner: std::sync::Mutex::new(inner.clone()),
            // A plan may have zero partitions; guard so we still emit once.
            remaining: AtomicUsize::new(partitions.max(1)),
            failed: AtomicBool::new(false),
            error: std::sync::Mutex::new(None),
            rows_written: std::sync::Mutex::new(None),
            emitted: AtomicBool::new(false),
        });
        Arc::new(Self { inner, state })
    }

    fn with_new_inner(&self, inner: Arc<dyn ExecutionPlan>) -> Arc<Self> {
        // Keep the shared run state pointed at the node that will execute, so
        // metrics are harvested from the right plan on completion.
        *self.state.inner.lock().unwrap() = inner.clone();
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
            Poll::Ready(Some(Ok(batch))) => {
                if let Some(rows) = write_count(&batch) {
                    self.state.record_rows_written(rows);
                }
                Poll::Ready(Some(Ok(batch)))
            }
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

/// Recognize DataFusion's write-result batch — a single `count` UInt64 column
/// whose value is the number of rows written — and return that count.
fn write_count(batch: &RecordBatch) -> Option<u64> {
    use datafusion::arrow::array::{Array, UInt64Array};

    if batch.num_columns() != 1 || batch.schema().field(0).name() != "count" {
        return None;
    }
    let counts = batch.column(0).as_any().downcast_ref::<UInt64Array>()?;
    Some(
        (0..counts.len())
            .filter(|i| counts.is_valid(*i))
            .map(|i| counts.value(i))
            .sum(),
    )
}
