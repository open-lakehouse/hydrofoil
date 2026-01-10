use std::sync::{Arc, LazyLock};

use arrow_flight::sql::{CommandStatementIngest, server::PeekableFlightDataStream};
use datafusion::{
    catalog::streaming::StreamingTable,
    datasource::provider_as_source,
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
        let schema = target.schema();

        let stream = FlightDataStream::new(stream.map_err(|e| e.into()), schema.clone());
        let source_provider = Arc::new(StreamingTable::try_new(schema, vec![Arc::new(stream)])?);
        let input = LogicalPlanBuilder::scan("input", provider_as_source(source_provider), None)?
            .build()?;

        LogicalPlanBuilder::insert_into(
            input,
            table_name,
            provider_as_source(target),
            InsertOp::Append,
        )?
        .build()
    }
}
