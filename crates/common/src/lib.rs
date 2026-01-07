extern crate prost;

use std::sync::LazyLock;

use arrow_flight::sql::{Any, ProstMessageExt};
use prost::Name;

pub use crate::models::delta::connect::{
    AddFeatureSupport, CreateDeltaTable, DeltaCommand, VacuumTable,
    create_delta_table::Mode as CreateDeltaTableMode,
    delta_command::CommandType as DeltaCommandType,
};

mod models {
    pub mod spark {
        pub mod connect {
            include!("gen/spark/connect/spark.connect.rs");
        }
    }
    pub mod delta {
        pub mod connect {
            include!("gen/delta/connect/delta.connect.rs");
        }
    }
}

impl ProstMessageExt for DeltaCommand {
    fn type_url() -> &'static str {
        static TYPE_URL: LazyLock<String> = LazyLock::new(|| <DeltaCommand as Name>::type_url());
        TYPE_URL.as_str()
    }

    fn as_any(&self) -> Any {
        Any {
            type_url: <DeltaCommand as ProstMessageExt>::type_url().to_string(),
            value: ::prost::Message::encode_to_vec(self).into(),
        }
    }
}
