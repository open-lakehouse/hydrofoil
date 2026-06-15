use std::sync::{Arc, LazyLock};

use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use arrow_flight::sql::{CommandStatementIngest, server::PeekableFlightDataStream};
use datafusion::{
    catalog::TableProvider,
    catalog::streaming::StreamingTable,
    datasource::{empty::EmptyTable, provider_as_source},
    error::Result,
    logical_expr::{LogicalPlan, LogicalPlanBuilder, dml::InsertOp},
    prelude::SessionContext,
};
use futures::TryStreamExt as _;
use tracing::instrument;

use crate::stream::FlightDataStream;

pub(crate) struct FlightPlanner;

impl FlightPlanner {
    pub fn new() -> Arc<Self> {
        static INSTANCE: LazyLock<Arc<FlightPlanner>> = LazyLock::new(|| Arc::new(FlightPlanner));
        INSTANCE.clone()
    }

    #[instrument(
        level = "info",
        skip_all,
        fields(
            table = command.table,
            schema = command.schema,
            catalog = command.catalog,
        )
    )]
    pub async fn plan_ingest(
        &self,
        session: &SessionContext,
        command: &CommandStatementIngest,
        stream: PeekableFlightDataStream,
    ) -> Result<LogicalPlan> {
        let table_name = &command.table;
        let target = session.table_provider(table_name).await?;
        Self::build_insert_plan(table_name, target, stream)
    }

    /// Build an `INSERT INTO … Append` logical plan that streams `stream` into the
    /// already-resolved `target` provider, coercing the wire batches to the
    /// table's schema.
    fn build_insert_plan(
        table_name: &str,
        target: Arc<dyn TableProvider>,
        stream: PeekableFlightDataStream,
    ) -> Result<LogicalPlan> {
        let schema = target.schema();
        let stream = FlightDataStream::new(stream.map_err(|e| e.into()), schema.clone());
        let source_provider = Arc::new(StreamingTable::try_new(schema, vec![Arc::new(stream)])?);
        let input = LogicalPlanBuilder::scan("input", provider_as_source(source_provider), None)?
            .build()?;
        insert_into_append(input, table_name, target)
    }

    /// Build the `INSERT INTO … Append` plan for *authorization only* — the same
    /// plan the external ingest path produces, but over an empty in-memory
    /// source rather than a live Flight stream. The managed ingest path uses
    /// this to run the Cedar gate (which inspects the write target and the
    /// optimized plan shape, not the data) before committing through the catalog
    /// committer.
    pub(crate) fn build_insert_plan_for_auth(
        table_name: &str,
        target: Arc<dyn TableProvider>,
    ) -> Result<LogicalPlan> {
        let schema = target.schema();
        let input = LogicalPlanBuilder::scan(
            "input",
            provider_as_source(Arc::new(EmptyTable::new(schema))),
            None,
        )?
        .build()?;
        insert_into_append(input, table_name, target)
    }
}

fn insert_into_append(
    input: LogicalPlan,
    table_name: &str,
    target: Arc<dyn TableProvider>,
) -> Result<LogicalPlan> {
    LogicalPlanBuilder::insert_into(
        input,
        table_name,
        provider_as_source(target),
        InsertOp::Append,
    )?
    .build()
}

/// Decode a Flight `do_put` data stream into [`RecordBatch`]es coerced to
/// `target_schema`, used by the managed-table ingest branch which writes the
/// batches through the Unity Catalog committer rather than a DataFusion plan.
///
/// Each decoded batch is cast column-by-column to `target_schema` (mirroring the
/// external sink's `cast_record_batch`), so ADBC type quirks (timestamp units,
/// dictionary-encoded strings) line up with the managed table's kernel schema.
pub(crate) async fn collect_coerced_batches(
    stream: PeekableFlightDataStream,
    target_schema: SchemaRef,
) -> Result<Vec<RecordBatch>> {
    use arrow::compute::cast;
    use arrow_flight::decode::FlightRecordBatchStream;
    use datafusion::common::exec_datafusion_err;

    let decoded = FlightRecordBatchStream::new_from_flight_data(stream.map_err(|e| e.into()));
    let raw: Vec<RecordBatch> = decoded
        .try_collect()
        .await
        .map_err(|e| exec_datafusion_err!("Failed to receive flight stream: {e}"))?;

    raw.into_iter()
        .map(|batch| {
            let columns = target_schema
                .fields()
                .iter()
                .map(|field| {
                    let col = batch.column_by_name(field.name()).ok_or_else(|| {
                        exec_datafusion_err!(
                            "ingest batch is missing column '{}' required by the target table",
                            field.name()
                        )
                    })?;
                    cast(col, field.data_type()).map_err(|e| {
                        exec_datafusion_err!(
                            "failed to coerce column '{}' to target type {:?}: {e}",
                            field.name(),
                            field.data_type()
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            RecordBatch::try_new(target_schema.clone(), columns)
                .map_err(|e| exec_datafusion_err!("failed to assemble coerced batch: {e}"))
        })
        .collect()
}
