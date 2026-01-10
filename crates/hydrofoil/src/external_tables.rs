use std::sync::{Arc, LazyLock};

use datafusion::{
    catalog::{Session, TableProvider, TableProviderFactory},
    common::{plan_datafusion_err, plan_err},
    error::Result,
    logical_expr::CreateExternalTable,
};
use delta_kernel::engine::arrow_conversion::TryIntoKernel;
use deltalake_core::{StructField, operations::create::CreateBuilder, protocol::SaveMode};
use itertools::Itertools as _;
use tracing::{debug, instrument};
use url::Url;

use crate::session::SessionExt as _;

#[derive(Debug, Clone, Copy)]
pub struct DeltaTableFactory;

impl DeltaTableFactory {
    pub const FILE_FORMAT: &'static str = "DELTA";

    pub fn new() -> Arc<dyn TableProviderFactory> {
        static INSTANCE: LazyLock<Arc<DeltaTableFactory>> =
            LazyLock::new(|| Arc::new(DeltaTableFactory));
        INSTANCE.clone()
    }
}

#[async_trait::async_trait]
impl TableProviderFactory for DeltaTableFactory {
    #[instrument(
        skip_all,
        fields(
            session_id = ctx.session_id(),
            table_name = cmd.name.table(),
            schame_name = cmd.name.schema(),
            catalog_name = cmd.name.catalog(),
            location = cmd.location,
        )
    )]
    async fn create(
        &self,
        ctx: &dyn Session,
        cmd: &CreateExternalTable,
    ) -> Result<Arc<dyn TableProvider>> {
        if cmd.unbounded {
            return plan_err!("Creating unbounded Delta tables is not supported");
        }
        if cmd.temporary {
            return plan_err!("Creating temporary Delta tables is not supported");
        }

        let location = Url::parse(&cmd.location).map_err(|e| plan_datafusion_err!("{e}"))?;
        let log_store = ctx.delta_logstore_for(&location)?;

        let columns: Vec<StructField> = cmd
            .schema
            .fields()
            .iter()
            .map(|f| f.as_ref().try_into_kernel())
            .try_collect()?;

        let save_mode = if cmd.or_replace {
            SaveMode::Overwrite
        } else if cmd.if_not_exists {
            SaveMode::Ignore
        } else {
            SaveMode::ErrorIfExists
        };

        let mut builder = CreateBuilder::new()
            .with_log_store(log_store)
            .with_table_name(cmd.name.table())
            .with_columns(columns)
            .with_save_mode(save_mode)
            .with_configuration(cmd.options.iter().map(|(k, v)| (k, Some(v))));

        if !cmd.table_partition_cols.is_empty() {
            builder = builder.with_partition_columns(&cmd.table_partition_cols);
        }

        debug!("Creating Delta table at '{}'.", location.as_str());
        Ok(builder.await?.table_provider().await?)
    }
}
