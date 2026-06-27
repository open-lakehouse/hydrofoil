//! Standalone Iceberg REST catalog (IRC) wiring.
//!
//! Registers configured Iceberg REST catalogs (e.g.
//! [Lakekeeper](https://github.com/lakekeeper/lakekeeper)) into a session's
//! [`SessionContext`] as DataFusion catalogs, addressable as
//! `<name>.<namespace>.<table>`.
//!
//! Unlike the Delta path, Iceberg performs its own object-store I/O through
//! `FileIO`: storage credentials are vended by the REST catalog's `loadTable`
//! response and threaded into `FileIO` by iceberg-rust, so no hydrofoil-side
//! object-store registration on the runtime is required.
//!
//! This is independent of Unity Catalog. UC-exposed Iceberg tables (routed via
//! `data_source_format`) are a separate, deferred path — UC's Iceberg REST
//! endpoint is not yet wired into the UC client.

use std::collections::HashMap;
use std::sync::Arc;

use datafusion::error::{DataFusionError, Result};
use datafusion::prelude::SessionContext;
use iceberg::CatalogBuilder;
use iceberg_catalog_rest::RestCatalogBuilder;
use iceberg_datafusion::IcebergCatalogProvider;

use crate::config::IcebergRestCatalog;

/// IRC property keys (see the Iceberg REST spec / `iceberg-catalog-rest`).
const PROP_URI: &str = "uri";
const PROP_WAREHOUSE: &str = "warehouse";
const PROP_TOKEN: &str = "token";
const PROP_CREDENTIAL: &str = "credential";
const PROP_OAUTH2_SERVER_URI: &str = "oauth2-server-uri";

/// Build the IRC property map for a configured catalog.
fn props_for(cat: &IcebergRestCatalog) -> HashMap<String, String> {
    let mut props = HashMap::new();
    props.insert(PROP_URI.to_string(), cat.uri.clone());
    if let Some(warehouse) = &cat.warehouse {
        props.insert(PROP_WAREHOUSE.to_string(), warehouse.clone());
    }
    if let Some(token) = &cat.token {
        props.insert(PROP_TOKEN.to_string(), token.clone());
    }
    if let Some(credential) = &cat.credential {
        props.insert(PROP_CREDENTIAL.to_string(), credential.clone());
    }
    if let Some(uri) = &cat.oauth2_server_uri {
        props.insert(PROP_OAUTH2_SERVER_URI.to_string(), uri.clone());
    }
    props
}

/// Build and register a single Iceberg REST catalog into `ctx`.
///
/// Loading the catalog lists its namespaces over the network (eager, as
/// [`IcebergCatalogProvider::try_new`] requires), so namespaces created after
/// session start are not auto-discovered — matching hydrofoil's per-session
/// model, where a fresh session re-lists. Table data still refreshes per scan.
pub async fn register_rest_catalog(ctx: &SessionContext, cat: &IcebergRestCatalog) -> Result<()> {
    let catalog = RestCatalogBuilder::default()
        .load(cat.name.clone(), props_for(cat))
        .await
        .map_err(|e| {
            DataFusionError::External(Box::new(std::io::Error::other(format!(
                "failed to load Iceberg REST catalog '{}' at '{}': {e}",
                cat.name, cat.uri
            ))))
        })?;

    let provider = IcebergCatalogProvider::try_new(Arc::new(catalog))
        .await
        .map_err(|e| {
            DataFusionError::External(Box::new(std::io::Error::other(format!(
                "failed to build provider for Iceberg REST catalog '{}': {e}",
                cat.name
            ))))
        })?;

    ctx.register_catalog(&cat.name, Arc::new(provider));
    Ok(())
}

/// Register every configured Iceberg REST catalog into `ctx`. A catalog that
/// fails to load (unreachable endpoint, bad auth) is logged and skipped rather
/// than failing the whole session, so one misconfigured catalog does not take
/// down a session that also serves Delta/UC tables.
pub async fn register_rest_catalogs(ctx: &SessionContext, catalogs: &[IcebergRestCatalog]) {
    for cat in catalogs {
        if let Err(e) = register_rest_catalog(ctx, cat).await {
            tracing::warn!(
                catalog = %cat.name, uri = %cat.uri, error = %e,
                "failed to register Iceberg REST catalog; skipping"
            );
        }
    }
}
