use arrow_array::RecordBatch;
use arrow_schema::{ArrowError, SchemaRef};
use arrow_select::concat::concat_batches;
use futures::Stream;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Default maximum batch size in bytes (3.5 MB to stay safely under 4MB with encoding overhead)
pub const DEFAULT_MAX_BATCH_SIZE: usize = 3_670_016; // 3.5 * 1024 * 1024

/// A stream transformer that ensures RecordBatches stay within size limits.
///
/// This wrapper:
/// - Splits large batches into smaller chunks
/// - Buffers small batches to combine them efficiently
/// - Returns an error if a single row exceeds the size limit
pub struct BatchSizer<S>
where
    S: Stream<Item = Result<RecordBatch, ArrowError>>,
{
    inner: Pin<Box<S>>,
    max_batch_size: usize,
    buffer: Vec<RecordBatch>,
    current_buffer_size: usize,
    schema: Option<SchemaRef>,
    split_queue: Vec<RecordBatch>,
    /// Track if we just split a batch, to avoid re-combining splits
    just_split: bool,
}

impl<S> BatchSizer<S>
where
    S: Stream<Item = Result<RecordBatch, ArrowError>>,
{
    /// Create a new BatchSizer with the default maximum batch size
    pub fn new(stream: S) -> Self {
        Self::with_max_size(stream, DEFAULT_MAX_BATCH_SIZE)
    }

    /// Create a new BatchSizer with a custom maximum batch size
    pub fn with_max_size(stream: S, max_batch_size: usize) -> Self {
        Self {
            inner: Box::pin(stream),
            max_batch_size,
            buffer: Vec::new(),
            current_buffer_size: 0,
            schema: None,
            split_queue: Vec::new(),
            just_split: false,
        }
    }

    /// Estimate the size of a RecordBatch in bytes
    ///
    /// Note: We calculate size based on the number of rows rather than using
    /// get_array_memory_size(), because sliced batches share underlying buffers
    /// and would report the full buffer size even for a slice.
    fn estimate_batch_size(batch: &RecordBatch) -> usize {
        if batch.num_rows() == 0 {
            return 0;
        }

        // For the first batch or full batch, get the actual memory size
        // Then estimate based on rows
        let full_size = batch.get_array_memory_size();
        let num_rows = batch.num_rows();

        // Calculate approximate bytes per row
        let bytes_per_row = (full_size as f64 / num_rows as f64).ceil() as usize;

        bytes_per_row * num_rows
    }

    /// Split a large RecordBatch into smaller chunks that fit within the size limit
    fn split_batch(&self, batch: RecordBatch) -> Result<Vec<RecordBatch>, ArrowError> {
        let num_rows = batch.num_rows();

        // Calculate bytes per row from the original batch
        // Note: get_array_memory_size() on sliced batches includes shared buffer overhead,
        // so we calculate this once from the original batch
        let total_size = batch.get_array_memory_size();

        let bytes_per_row = if num_rows > 0 {
            ((total_size as f64) / (num_rows as f64)).ceil() as usize
        } else {
            return Ok(vec![batch]);
        };

        let batch_size = bytes_per_row * num_rows;

        // If batch is within limit, return as-is
        if batch_size <= self.max_batch_size {
            return Ok(vec![batch]);
        }

        // Check if it's a single row that's too large
        if num_rows == 1 {
            return Err(ArrowError::InvalidArgumentError(format!(
                "Single row exceeds maximum batch size: {} bytes > {} bytes limit",
                batch_size, self.max_batch_size
            )));
        }

        // Start with an estimated number of rows per chunk
        let mut chunks = Vec::new();
        let mut offset = 0;

        while offset < num_rows {
            // Binary search for the maximum number of rows that fits in max_batch_size
            let remaining_rows = num_rows - offset;
            let mut low = 1;
            let mut high = remaining_rows;
            let mut best_length = 1;

            while low <= high {
                let mid = (low + high) / 2;
                // Estimate size based on bytes per row
                let test_size = bytes_per_row * mid;

                if test_size <= self.max_batch_size {
                    best_length = mid;
                    low = mid + 1;
                } else {
                    high = mid - 1;
                }
            }

            // Check if even a single row is too large
            if best_length == 1 && bytes_per_row > self.max_batch_size {
                return Err(ArrowError::InvalidArgumentError(format!(
                    "Single row exceeds maximum batch size: {} bytes > {} bytes limit",
                    bytes_per_row, self.max_batch_size
                )));
            }

            chunks.push(batch.slice(offset, best_length));
            offset += best_length;
        }

        Ok(chunks)
    }

    /// Try to flush the buffer by combining batches and returning a result
    fn try_flush_buffer(&mut self) -> Option<Result<RecordBatch, ArrowError>> {
        if self.buffer.is_empty() {
            return None;
        }

        // If we only have one batch, just return it
        if self.buffer.len() == 1 {
            let batch = self.buffer.remove(0);
            self.current_buffer_size = 0;
            return Some(Ok(batch));
        }

        // Combine multiple batches
        let schema = self.schema.as_ref().unwrap().clone();
        match concat_batches(&schema, &self.buffer) {
            Ok(combined) => {
                self.buffer.clear();
                self.current_buffer_size = 0;
                Some(Ok(combined))
            }
            Err(e) => {
                self.buffer.clear();
                self.current_buffer_size = 0;
                Some(Err(e))
            }
        }
    }

    /// Add a batch to the buffer
    fn add_to_buffer(&mut self, batch: RecordBatch) {
        let size = Self::estimate_batch_size(&batch);

        // Cache schema on first batch
        if self.schema.is_none() {
            self.schema = Some(batch.schema());
        }

        self.current_buffer_size += size;
        self.buffer.push(batch);
    }

    /// Check if buffer should be flushed
    fn should_flush_buffer(&self) -> bool {
        self.current_buffer_size >= self.max_batch_size
    }
}

impl<S> Stream for BatchSizer<S>
where
    S: Stream<Item = Result<RecordBatch, ArrowError>>,
{
    type Item = Result<RecordBatch, ArrowError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // First, check if we have any pre-split batches queued
            if !self.split_queue.is_empty() {
                let batch = self.split_queue.remove(0);

                // If we just split a batch, yield the splits directly without buffering
                // to avoid re-combining them
                if self.just_split {
                    // Cache schema if needed
                    if self.schema.is_none() {
                        self.schema = Some(batch.schema());
                    }
                    return Poll::Ready(Some(Ok(batch)));
                } else {
                    // Normal buffering for batches that weren't split
                    self.add_to_buffer(batch);

                    if self.should_flush_buffer()
                        && let Some(result) = self.try_flush_buffer()
                    {
                        return Poll::Ready(Some(result));
                    }
                    continue;
                }
            }

            // Reset just_split flag when split_queue is empty
            if self.just_split && self.split_queue.is_empty() {
                self.just_split = false;
            }

            // Poll the inner stream for the next batch
            match self.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(batch))) => {
                    // Split the batch if necessary
                    match self.split_batch(batch) {
                        Ok(chunks) => {
                            // Check if we actually split the batch (more than 1 chunk)
                            if chunks.len() > 1 {
                                self.just_split = true;
                            }
                            // Add chunks to split queue for processing
                            self.split_queue.extend(chunks);
                            continue;
                        }
                        Err(e) => {
                            // Error splitting batch (likely single row too large)
                            return Poll::Ready(Some(Err(e)));
                        }
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    // Propagate error from inner stream
                    return Poll::Ready(Some(Err(e)));
                }
                Poll::Ready(None) => {
                    // Inner stream is exhausted, flush any remaining buffer
                    if let Some(result) = self.try_flush_buffer() {
                        return Poll::Ready(Some(result));
                    }
                    return Poll::Ready(None);
                }
                Poll::Pending => {
                    // If we have buffered data and the stream is pending,
                    // we could flush the buffer now, but let's wait for more data
                    // unless we're at the size limit
                    if self.should_flush_buffer()
                        && let Some(result) = self.try_flush_buffer()
                    {
                        return Poll::Ready(Some(result));
                    }
                    return Poll::Pending;
                }
            }
        }
    }
}

pin_project! {
    /// Stream that buffers and chunks RecordBatches to respect size limits
    pub(crate) struct BatchChunkStream<S> {
        max_message_size: usize,

        buffer: Option<(RecordBatch, usize)>,

        // The schema of the output stream
        schema: SchemaRef,

        #[pin]
        // The input stream
        stream: S,
    }
}

impl<S> BatchChunkStream<S>
where
    S: Stream<Item = Result<RecordBatch, ArrowError>>,
{
    /// Create a new BatchChunkStream with the specified maximum batch size
    pub fn new(stream: S, schema: SchemaRef) -> Self {
        Self {
            max_message_size: DEFAULT_MAX_BATCH_SIZE,
            buffer: None,
            schema,
            stream,
        }
    }

    pub fn with_max_message_size(mut self, max_batch_size: impl Into<Option<usize>>) -> Self {
        self.max_message_size = max_batch_size.into().unwrap_or(DEFAULT_MAX_BATCH_SIZE);
        self
    }
}

impl<S> Stream for BatchChunkStream<S>
where
    S: Stream<Item = Result<RecordBatch, ArrowError>>,
{
    type Item = Result<RecordBatch, ArrowError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        if let Some((batch, start)) = this.buffer.take() {
            let batch_size = BatchSizer::<S>::estimate_batch_size(&batch);
            let bytes_per_row = if batch.num_rows() > 0 {
                (batch_size as f64 / batch.num_rows() as f64).ceil() as usize
            } else {
                0
            };

            let mut end = batch.num_rows();

            if bytes_per_row > 0 {
                let max_rows = *this.max_message_size / bytes_per_row;
                end = std::cmp::min(start + max_rows, batch.num_rows());
            }

            let chunk = batch.slice(start, end - start);

            if end < batch.num_rows() {
                // More rows remain, store in buffer
                *this.buffer = Some((batch, end));
            }

            return Poll::Ready(Some(Ok(chunk)));
        }

        match this.stream.poll_next(cx) {
            Poll::Ready(Some(Ok(batch))) => {
                if batch.get_array_memory_size() < *this.max_message_size {
                    return Poll::Ready(Some(Ok(batch)));
                }
                Poll::Ready(Some(Ok(batch)))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(Err(err))) => Poll::Ready(Some(Err(err))),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{Int32Array, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use futures::{StreamExt, stream};
    use std::sync::Arc;

    fn create_test_schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, false),
        ]))
    }

    fn create_test_batch(schema: SchemaRef, num_rows: usize) -> RecordBatch {
        let ids: Int32Array = (0..num_rows as i32).collect();
        let names: StringArray =
            StringArray::from_iter_values((0..num_rows).map(|i| format!("name_{}", i)));

        RecordBatch::try_new(schema, vec![Arc::new(ids), Arc::new(names)]).unwrap()
    }

    #[tokio::test]
    async fn test_small_batches_are_combined() {
        let schema = create_test_schema();

        // Create several small batches (10 rows each)
        let batches = vec![
            Ok(create_test_batch(schema.clone(), 10)),
            Ok(create_test_batch(schema.clone(), 10)),
            Ok(create_test_batch(schema.clone(), 10)),
        ];

        let stream = stream::iter(batches);
        let sizer = BatchSizer::with_max_size(stream, 10_000); // Small limit to force combining

        let results: Vec<_> = sizer.collect().await;

        // Should combine into fewer batches
        assert!(!results.is_empty());
        assert!(results.len() < 3); // Should be combined

        // Verify total row count
        let total_rows: usize = results
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .map(|b| b.num_rows())
            .sum();
        assert_eq!(total_rows, 30);
    }

    #[tokio::test]
    async fn test_large_batch_is_split() {
        let schema = create_test_schema();

        // Create a batch with many rows and larger strings to ensure it's big enough
        let num_rows = 10000;
        let ids: Int32Array = (0..num_rows as i32).collect();
        // Create longer strings to increase batch size
        let names: StringArray = StringArray::from_iter_values(
            (0..num_rows)
                .map(|i| format!("this_is_a_longer_name_value_for_row_{}_with_more_data", i)),
        );
        let large_batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(ids), Arc::new(names)]).unwrap();

        let batch_size = BatchSizer::<
            stream::Iter<std::vec::IntoIter<Result<RecordBatch, ArrowError>>>,
        >::estimate_batch_size(&large_batch);
        println!("Batch size: {} bytes", batch_size);
        println!("Limit: {} bytes", batch_size / 4);

        let batches = vec![Ok(large_batch)];
        let stream = stream::iter(batches);

        // Set limit smaller than the batch size
        let sizer = BatchSizer::with_max_size(stream, batch_size / 4);

        let results: Vec<_> = sizer.collect().await;

        println!("Number of result batches: {}", results.len());

        // Should be split into multiple batches
        assert!(
            results.len() > 1,
            "Expected multiple batches but got {}",
            results.len()
        );

        // Verify total row count
        let total_rows: usize = results
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .map(|b| b.num_rows())
            .sum();
        assert_eq!(total_rows, num_rows);

        // Verify each batch has a reasonable number of rows
        // Since we're using bytes_per_row estimation, we can't verify exact sizes
        // on sliced batches (they share buffers), but we can verify row counts
        let bytes_per_row = batch_size / num_rows;
        let expected_rows_per_chunk = (batch_size / 4) / bytes_per_row;

        for (i, result) in results.iter().enumerate() {
            let batch = result.as_ref().unwrap();
            println!("Batch {} rows: {}", i, batch.num_rows());
            // Each chunk should have roughly the expected number of rows (within 2x for safety)
            assert!(
                batch.num_rows() <= expected_rows_per_chunk * 2,
                "Batch {} has {} rows, expected around {}",
                i,
                batch.num_rows(),
                expected_rows_per_chunk
            );
        }
    }

    #[tokio::test]
    async fn test_single_large_row_returns_error() {
        let schema = create_test_schema();

        // Create a batch with a single row
        let single_row_batch = create_test_batch(schema.clone(), 1);
        let batch_size = BatchSizer::<
            stream::Iter<std::vec::IntoIter<Result<RecordBatch, ArrowError>>>,
        >::estimate_batch_size(&single_row_batch);

        let batches = vec![Ok(single_row_batch)];
        let stream = stream::iter(batches);

        // Set limit smaller than the single row
        let sizer = BatchSizer::with_max_size(stream, batch_size / 2);

        let results: Vec<_> = sizer.collect().await;

        // Should return an error
        assert_eq!(results.len(), 1);
        assert!(results[0].is_err());

        let err = results[0].as_ref().unwrap_err();
        assert!(
            err.to_string()
                .contains("Single row exceeds maximum batch size")
        );
    }

    #[tokio::test]
    async fn test_empty_stream() {
        let batches: Vec<Result<RecordBatch, ArrowError>> = vec![];
        let stream = stream::iter(batches);
        let sizer = BatchSizer::new(stream);

        let results: Vec<_> = sizer.collect().await;

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_error_propagation() {
        let batches = vec![Err(ArrowError::InvalidArgumentError(
            "test error".to_string(),
        ))];
        let stream = stream::iter(batches);
        let sizer = BatchSizer::new(stream);

        let results: Vec<_> = sizer.collect().await;

        assert_eq!(results.len(), 1);
        assert!(results[0].is_err());
        assert!(
            results[0]
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("test error")
        );
    }
}
