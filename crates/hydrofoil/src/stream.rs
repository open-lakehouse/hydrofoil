// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use arrow::datatypes::SchemaRef;
use arrow_flight::FlightData;
use arrow_flight::decode::FlightRecordBatchStream;
use arrow_flight::encode::FlightDataEncoderBuilder;
use arrow_flight::error::FlightError;

use datafusion::catalog::Session;
use datafusion::common::runtime::JoinSet;
use datafusion::common::{exec_datafusion_err, exec_err};
use datafusion::error::DataFusionError;
use datafusion::execution::{SendableRecordBatchStream, TaskContext};

use datafusion::logical_expr::LogicalPlan;
use datafusion::physical_plan::displayable;
use datafusion::physical_plan::execute_stream;
use datafusion::physical_plan::stream::RecordBatchStreamAdapter;
use datafusion::physical_plan::streaming::PartitionStream;
use futures::TryStreamExt;
use futures::stream::BoxStream;
use futures::{Future, Stream, StreamExt};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{Instrument, Span, debug, dispatcher, instrument};

type FlightResult<T> = Result<T, FlightError>;
type FlightStream = Pin<Box<dyn Stream<Item = Result<FlightData, FlightError>> + Send + 'static>>;

fn to_flight_err(error: DataFusionError) -> FlightError {
    match error {
        DataFusionError::ArrowError(arrow_err, _msg) => FlightError::Arrow(*arrow_err),
        _ => FlightError::from_external_error(Box::new(error)),
    }
}

/// Creates a stream from a collection of producing tasks, routing panics to the stream.
///
/// Note that this is similar to  [`ReceiverStream` from tokio-stream], with the differences being:
///
/// 1. Methods to bound and "detach"  tasks (`spawn()` and `spawn_blocking()`).
///
/// 2. Propagates panics, whereas the `tokio` version doesn't propagate panics to the receiver.
///
/// 3. Automatically cancels any outstanding tasks when the receiver stream is dropped.
///
/// [`ReceiverStream` from tokio-stream]: https://docs.rs/tokio-stream/latest/tokio_stream/wrappers/struct.ReceiverStream.html
pub(crate) struct ReceiverStreamBuilder<O> {
    tx: Sender<Result<O, FlightError>>,
    rx: Receiver<Result<O, FlightError>>,
    join_set: JoinSet<FlightResult<()>>,
}

#[allow(unused)]
impl<O: Send + 'static> ReceiverStreamBuilder<O> {
    /// Create new channels with the specified buffer size
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(capacity);

        Self {
            tx,
            rx,
            join_set: JoinSet::new(),
        }
    }

    /// Get a handle for sending data to the output
    pub fn tx(&self) -> Sender<FlightResult<O>> {
        self.tx.clone()
    }

    /// Spawn task that will be aborted if this builder (or the stream
    /// built from it) are dropped
    pub fn spawn<F>(&mut self, task: F)
    where
        F: Future<Output = FlightResult<()>>,
        F: Send + 'static,
    {
        let current_span = tracing::Span::current();
        self.join_set
            .spawn(async move { task.instrument(current_span).await });
    }

    /// Same as [`Self::spawn`] but it spawns the task on the provided runtime
    pub fn spawn_on<F>(&mut self, task: F, handle: &Handle)
    where
        F: Future<Output = FlightResult<()>>,
        F: Send + 'static,
    {
        let current_span = Span::current();
        self.join_set
            .spawn_on(async move { task.instrument(current_span).await }, handle);
    }

    /// Spawn a blocking task that will be aborted if this builder (or the stream
    /// built from it) are dropped.
    ///
    /// This is often used to spawn tasks that write to the sender
    /// retrieved from `Self::tx`.
    pub fn spawn_blocking<F>(&mut self, f: F)
    where
        F: FnOnce() -> Result<(), FlightError>,
        F: Send + 'static,
    {
        let dispatch = dispatcher::get_default(|d| d.clone());
        let span = tracing::Span::current();

        self.join_set.spawn_blocking(move || {
            dispatcher::with_default(&dispatch, || {
                let _enter = span.enter();
                f()
            })
        });
    }

    /// Same as [`Self::spawn_blocking`] but it spawns the blocking task on the provided runtime
    pub fn spawn_blocking_on<F>(&mut self, f: F, handle: &Handle)
    where
        F: FnOnce() -> Result<(), FlightError>,
        F: Send + 'static,
    {
        let dispatch = dispatcher::get_default(|d| d.clone());
        let span = tracing::Span::current();

        self.join_set.spawn_blocking_on(
            move || {
                dispatcher::with_default(&dispatch, || {
                    let _enter = span.enter();
                    f()
                })
            },
            handle,
        );
    }

    /// Create a stream of all data written to `tx`
    pub fn build(self) -> BoxStream<'static, Result<O, FlightError>> {
        let Self {
            tx,
            rx,
            mut join_set,
        } = self;

        // Doesn't need tx
        drop(tx);

        // future that checks the result of the join set, and propagates panic if seen
        let check = async move {
            while let Some(result) = join_set.join_next().await {
                match result {
                    Ok(task_result) => {
                        match task_result {
                            // Nothing to report
                            Ok(_) => continue,
                            // This means a blocking task error
                            Err(error) => return Some(Err(error)),
                        }
                    }
                    // This means a tokio task error, likely a panic
                    Err(e) => {
                        if e.is_panic() {
                            // resume on the main thread
                            std::panic::resume_unwind(e.into_panic());
                        } else {
                            // This should only occur if the task is
                            // cancelled, which would only occur if
                            // the JoinSet were aborted, which in turn
                            // would imply that the receiver has been
                            // dropped and this code is not running
                            return Some(Err(FlightError::from_external_error(Box::new(e))));
                        }
                    }
                }
            }
            None
        };

        let check_stream = futures::stream::once(check)
            // unwrap Option / only return the error
            .filter_map(|item| async move { item });

        // Convert the receiver into a stream
        let rx_stream = futures::stream::unfold(rx, |mut rx| async move {
            let next_item = rx.recv().await;
            next_item.map(|next_item| (next_item, rx))
        });

        // Merge the streams together so whichever is ready first
        // produces the batch
        futures::stream::select(rx_stream, check_stream).boxed()
    }
}

/// Builder for `RecordBatchReceiverStream` that propagates errors
/// and panic's correctly.
///
/// [`RecordBatchReceiverStreamBuilder`] is used to spawn one or more tasks
/// that produce [`RecordBatch`]es and send them to a single
/// `Receiver` which can improve parallelism.
///
/// This also handles propagating panic`s and canceling the tasks.
///
/// # Example
///
/// The following example spawns 2 tasks that will write [`RecordBatch`]es to
/// the `tx` end of the builder, after building the stream, we can receive
/// those batches with calling `.next()`
///
/// ```
/// # use std::sync::Arc;
/// # use datafusion_common::arrow::datatypes::{Schema, Field, DataType};
/// # use datafusion_common::arrow::array::RecordBatch;
/// # use datafusion_physical_plan::stream::RecordBatchReceiverStreamBuilder;
/// # use futures::stream::StreamExt;
/// # use tokio::runtime::Builder;
/// # let rt = Builder::new_current_thread().build().unwrap();
/// #
/// # rt.block_on(async {
/// let schema = Arc::new(Schema::new(vec![Field::new("foo", DataType::Int8, false)]));
/// let mut builder = RecordBatchReceiverStreamBuilder::new(Arc::clone(&schema), 10);
///
/// // task 1
/// let tx_1 = builder.tx();
/// let schema_1 = Arc::clone(&schema);
/// builder.spawn(async move {
///     // Your task needs to send batches to the tx
///     tx_1.send(Ok(RecordBatch::new_empty(schema_1)))
///         .await
///         .unwrap();
///
///     Ok(())
/// });
///
/// // task 2
/// let tx_2 = builder.tx();
/// let schema_2 = Arc::clone(&schema);
/// builder.spawn(async move {
///     // Your task needs to send batches to the tx
///     tx_2.send(Ok(RecordBatch::new_empty(schema_2)))
///         .await
///         .unwrap();
///
///     Ok(())
/// });
///
/// let mut stream = builder.build();
/// while let Some(res_batch) = stream.next().await {
///     // `res_batch` can either from task 1 or 2
///
///     // do something with `res_batch`
/// }
/// # });
/// ```
pub struct FlightDataReceiverStreamBuilder {
    inner: ReceiverStreamBuilder<FlightData>,
}

impl FlightDataReceiverStreamBuilder {
    /// Create new channels with the specified buffer size
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: ReceiverStreamBuilder::new(capacity),
        }
    }

    /// Get a handle for sending [`RecordBatch`] to the output
    ///
    /// If the stream is dropped / canceled, the sender will be closed and
    /// calling `tx().send()` will return an error. Producers should stop
    /// producing in this case and return control.
    pub fn tx(&self) -> Sender<Result<FlightData, FlightError>> {
        self.inner.tx()
    }

    /// Same as [`Self::spawn`] but it spawns the task on the provided runtime.
    pub fn spawn_on<F>(&mut self, task: F, handle: &Handle)
    where
        F: Future<Output = FlightResult<()>>,
        F: Send + 'static,
    {
        self.inner.spawn_on(task, handle)
    }

    #[instrument(skip_all, level = "info", fields(plan = plan.display_indent().to_string()))]
    pub fn execute_logical_plan(
        &mut self,
        ctx: Arc<dyn Session>,
        plan: LogicalPlan,
        handle: &Handle,
    ) {
        let tx = self.tx();

        let driver_task = async move {
            let exec = match ctx.create_physical_plan(&plan).await {
                Err(e) => {
                    // If send fails, the plan being torn down, there
                    // is no place to send the error and no reason to continue.
                    tx.send(Err(to_flight_err(e))).await.ok();
                    debug!("Stopping execution: error creating physical plan: {plan:?}",);
                    return Ok(());
                }
                Ok(exec) => exec,
            };

            let schema = exec.schema().clone();

            let stream = match execute_stream(exec.clone(), ctx.task_ctx()) {
                Err(e) => {
                    tx.send(Err(to_flight_err(e))).await.ok();
                    debug!(
                        "Stopping execution: error executing input: {}",
                        displayable(exec.as_ref()).one_line()
                    );
                    return Ok(());
                }
                Ok(stream) => stream,
            }
            .map_err(|e| FlightError::from_external_error(Box::new(e)));

            let mut stream = FlightDataEncoderBuilder::new()
                .with_schema(schema)
                .build(stream);

            while let Some(data) = stream.next().await {
                let is_err = data.is_err();

                if tx.send(data).await.is_err() {
                    debug!(
                        "Stopping execution: output is gone, plan cancelling: {}",
                        displayable(exec.as_ref()).one_line()
                    );
                    return Ok(());
                }

                // Stop after the first error is encountered (Don't drive all streams to completion)
                if is_err {
                    debug!(
                        "Stopping execution: plan returned error: {}",
                        displayable(exec.as_ref()).one_line()
                    );
                    return Ok(());
                }
            }

            Ok(())
        };

        self.spawn_on(driver_task, handle);
    }

    /// Create a stream of all [`FlightData`] written to `tx`
    pub fn build(self) -> FlightStream {
        Box::pin(self.inner.build())
    }
}

#[derive(Debug)]
pub struct FlightDataStream {
    inner: Mutex<Option<FlightRecordBatchStream>>,
    schema: SchemaRef,
}

impl FlightDataStream {
    pub fn new<S>(inner: S, schema: SchemaRef) -> Self
    where
        S: Stream<Item = Result<FlightData, FlightError>> + Send + 'static,
    {
        Self {
            inner: Mutex::new(Some(FlightRecordBatchStream::new_from_flight_data(inner))),
            schema,
        }
    }
}

impl PartitionStream for FlightDataStream {
    fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    fn execute(&self, _context: Arc<TaskContext>) -> SendableRecordBatchStream {
        if let Ok(mut guard) = self.inner.lock()
            && let Some(stream) = guard.take()
        {
            return Box::pin(RecordBatchStreamAdapter::new(
                self.schema().clone(),
                stream.map_err(|e| exec_datafusion_err!("Failed to receive flight stream: {}", e)),
            ));
        };
        Box::pin(RecordBatchStreamAdapter::new(
            self.schema().clone(),
            futures::stream::once(async {
                exec_err!(
                    "FlightDataStream has already been executed once and cannot be executed again"
                )
            }),
        ))
    }
}
