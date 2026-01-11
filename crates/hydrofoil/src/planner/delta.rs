use std::sync::{Arc, LazyLock};

use arrow::datatypes::Schema;
use datafusion::{
    catalog::Session,
    common::{DFSchema, plan_err},
    error::Result,
    logical_expr::{CreateExternalTable, DdlStatement, LogicalPlan},
    sql::TableReference,
};
use hydrofoil_common::{
    CreateDeltaTable, CreateDeltaTableMode, DeltaCommand, DeltaCommandType,
    conversion::{ConversionOptions, column_to_arrow},
};
use itertools::Itertools as _;

use crate::catalog::DeltaTableFactory;

#[derive(Debug, Clone, Copy)]
pub(crate) struct DeltaPlanner;

impl DeltaPlanner {
    pub fn new() -> Arc<Self> {
        static INSTANCE: LazyLock<Arc<DeltaPlanner>> = LazyLock::new(|| Arc::new(DeltaPlanner));
        INSTANCE.clone()
    }

    pub fn plan_delta_connect(
        &self,
        _session: &dyn Session,
        command: &DeltaCommand,
    ) -> Result<LogicalPlan> {
        let Some(command_type) = &command.command_type else {
            return plan_err!("DeltaCommand has no command_type");
        };
        match command_type {
            DeltaCommandType::CreateDeltaTable(cmd) => plan_create_delta_table(cmd),
            _ => plan_err!("Unsupported command in DeltaPlanner: {command:?}"),
        }
    }
}

fn plan_create_delta_table(command: &CreateDeltaTable) -> Result<LogicalPlan> {
    let Some(location) = &command.location else {
        return plan_err!(
            "CreateDeltaTable command requires a location. Registration to catalogs not yet implemented."
        );
    };

    let fields: Vec<_> = command
        .columns
        .iter()
        .map(|c| column_to_arrow(c, &ConversionOptions::default()))
        .try_collect()?;
    let schema = Schema::new(fields);
    let table_ref = TableReference::parse_str(command.table_name());
    let schema = Arc::new(DFSchema::try_from(schema)?);

    let cmd = CreateExternalTable {
        schema,
        name: table_ref,
        location: location.clone(),
        file_type: DeltaTableFactory::FILE_FORMAT.to_string(),
        table_partition_cols: command.partitioning_columns.clone(),
        if_not_exists: matches!(command.mode(), CreateDeltaTableMode::CreateIfNotExists),
        or_replace: matches!(command.mode(), CreateDeltaTableMode::CreateOrReplace),
        options: command.properties.clone(),
        unbounded: false,
        temporary: false,
        definition: None,
        order_exprs: Default::default(),
        constraints: Default::default(),
        column_defaults: Default::default(),
    };

    Ok(LogicalPlan::Ddl(DdlStatement::CreateExternalTable(cmd)))
}
