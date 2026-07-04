pub use self::external_table::DeltaTableFactory;
#[cfg(feature = "governance")]
pub use self::functions::UnityFunctionResolver;
pub use self::schema::LakehouseSchemaProvider;
pub use self::tags::{CatalogFactSinkExt, UnityGovernedTagProvider};
pub use self::unity::LakehouseTableProviderBuilder;

mod external_table;
#[cfg(feature = "governance")]
mod functions;
mod schema;
mod tags;
mod unity;
