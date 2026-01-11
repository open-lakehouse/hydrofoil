use datafusion::catalog::{AsyncCatalogProviderList, AsyncSchemaProvider};

pub use self::external_table::DeltaTableFactory;

mod external_table;
