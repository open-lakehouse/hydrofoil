//! Server configuration.
//!
//! Mirrors the layered approach used by `lineage-service`: struct defaults are
//! overlaid with an optional config file and then `HYDROFOIL__*` environment
//! overrides, with secrets pulled from the environment so they never live in the
//! checked-in file. See [`Config::load`].
//!
//! The three integrations (OpenLineage, Cedar policy, Unity Catalog) are each
//! optional: a section that is absent (or whose required field is empty) leaves
//! that integration disabled, reproducing an empty config = fully-ungoverned,
//! standalone server.

use std::env;
use std::path::{Path, PathBuf};

use config::{Config as ConfigSource, Environment, File};
use serde::Deserialize;

/// Default Flight SQL listen address.
fn default_host() -> String {
    "0.0.0.0".into()
}

/// Default Flight SQL listen port.
fn default_port() -> u16 {
    50051
}

/// Default idle session TTL (30 minutes).
fn default_session_ttl_secs() -> u64 {
    1800
}

/// Default HTTP query-surface listen port. Matches the UC-quickstart query
/// sidecar's default (`QUERY_BIND` -> `…:9082`) so its callers point here
/// unchanged.
fn default_http_port() -> u16 {
    9082
}

/// The HTTP query surface is exposed by default (it replaces the query
/// sidecar). Set `http_enabled = false` to run Flight SQL only.
fn default_http_enabled() -> bool {
    true
}

/// Default hard cap on a query's row limit (sidecar parity: `QUERY_MAX_LIMIT`).
fn default_query_max_limit() -> u32 {
    10_000
}

/// Default row limit applied when a request omits one (sidecar parity:
/// `QUERY_DEFAULT_LIMIT`).
fn default_query_default_limit() -> u32 {
    1_000
}

/// OpenLineage integration. Enabled when [`url`](LineageConfig::url) is set.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LineageConfig {
    /// Base URL of an OpenLineage-compatible service, e.g.
    /// `http://marquez:9080`. Empty/absent disables lineage emission.
    pub url: Option<String>,
    /// Path appended to `url` to form the lineage endpoint. Defaults to
    /// `/api/v1/lineage` when unset.
    pub endpoint: Option<String>,
    /// Default OpenLineage job namespace (falls back to `default`).
    pub namespace: Option<String>,
    /// Optional bearer token. A secret — supply via the `OPENLINEAGE_API_KEY`
    /// env var rather than the config file (see [`SECRET_ENV_KEYS`]).
    pub api_key: Option<String>,
}

/// Cedar policy enforcement. Enabled when [`oci_ref`](PolicyConfig::oci_ref) is
/// set; otherwise the server runs allow-all (open, ungoverned).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PolicyConfig {
    /// OCI reference to a Cedar policy image, e.g.
    /// `localhost:10100/hydrofoil/plan-policy:latest`.
    pub oci_ref: Option<String>,
}

/// Unity Catalog integration. Enabled when [`endpoint`](UnityConfig::endpoint)
/// is set.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UnityConfig {
    /// Unity Catalog REST base URL, e.g.
    /// `http://unity-catalog:8081/api/2.1/unity-catalog/`.
    pub endpoint: Option<String>,
    /// Optional bearer token; when absent the factory runs unauthenticated
    /// (a local OSS server). A secret — supply via `UC_TOKEN` (see
    /// [`SECRET_ENV_KEYS`]).
    pub token: Option<String>,
    /// AWS region hint for the UC object-store factory. Falls back to
    /// `AWS_REGION` (see [`SECRET_ENV_KEYS`]).
    pub region: Option<String>,
}

/// A standalone Iceberg REST catalog (IRC) to register into every session, e.g.
/// a [Lakekeeper](https://github.com/lakekeeper/lakekeeper) deployment. Each
/// entry becomes a DataFusion catalog addressable as `<name>.<namespace>.<table>`.
///
/// Unlike Delta, Iceberg performs its own object-store I/O via `FileIO`; storage
/// credentials are vended by the REST catalog's `loadTable` response, so no
/// hydrofoil-side object-store registration is needed.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct IcebergRestCatalog {
    /// DataFusion catalog name to register this catalog under (the first part of
    /// a three-part `catalog.namespace.table` reference).
    pub name: String,
    /// Iceberg REST Catalog base URI, e.g. `http://lakekeeper:8181/catalog`.
    pub uri: String,
    /// Optional warehouse identifier passed to the REST catalog.
    pub warehouse: Option<String>,
    /// Optional bearer token (maps to the IRC `token` property). A secret —
    /// supply via `ICEBERG__<NAME>__TOKEN` env override rather than the file.
    pub token: Option<String>,
    /// Optional OAuth2 client credential `client_id:client_secret` (IRC
    /// `credential` property), used when `token` is absent.
    pub credential: Option<String>,
    /// Optional OAuth2 server URI for the client-credentials flow.
    pub oauth2_server_uri: Option<String>,
}

/// Iceberg integration. Each configured REST catalog is registered into every
/// session; an empty list leaves Iceberg disabled.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct IcebergConfig {
    /// Standalone Iceberg REST catalogs to register, e.g. Lakekeeper.
    pub rest_catalogs: Vec<IcebergRestCatalog>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Flight SQL listen host.
    pub host: String,
    /// Flight SQL listen port.
    pub port: u16,
    /// Whether to expose the HTTP query surface (`POST /query`) at all. When
    /// false, only the Flight SQL gRPC server runs. Defaults to true.
    pub http_enabled: bool,
    /// HTTP query-surface listen port (the catalog-native `POST /query`
    /// endpoint that replaces the UC-quickstart query sidecar). Shares
    /// [`host`](Self::host) with the Flight SQL listener; bound on its own port
    /// so Flight (ADBC) and HTTP clients coexist.
    pub http_port: u16,
    /// Hard cap on a query's row limit; a request asking for more is clamped to
    /// this. Mirrors the sidecar's `QUERY_MAX_LIMIT`.
    pub query_max_limit: u32,
    /// Row limit applied when a `POST /query` request omits one. Mirrors the
    /// sidecar's `QUERY_DEFAULT_LIMIT`.
    pub query_default_limit: u32,
    /// Idle TTL after which a session (and its statements) is swept.
    pub session_ttl_secs: u64,
    pub lineage: LineageConfig,
    pub policy: PolicyConfig,
    pub unity: UnityConfig,
    pub iceberg: IcebergConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            http_enabled: default_http_enabled(),
            http_port: default_http_port(),
            query_max_limit: default_query_max_limit(),
            query_default_limit: default_query_default_limit(),
            session_ttl_secs: default_session_ttl_secs(),
            lineage: LineageConfig::default(),
            policy: PolicyConfig::default(),
            unity: UnityConfig::default(),
            iceberg: IcebergConfig::default(),
        }
    }
}

/// Error raised while loading configuration. A missing file (when none was
/// explicitly requested) falls back to defaults and is *not* an error; a
/// malformed file or unparsable value is, so a misconfigured deployment refuses
/// to start instead of silently running on defaults.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to load configuration: {0}")]
    Source(#[from] config::ConfigError),
}

/// Environment variable holding the path to the config file. Also accepted as
/// the binary's first positional argument (see `main`).
pub const CONFIG_PATH_ENV: &str = "HYDROFOIL_CONFIG";

/// Prefix and separator for environment overrides of structured config keys,
/// e.g. `HYDROFOIL__PORT=9050` or `HYDROFOIL__UNITY__ENDPOINT=http://uc/`.
const ENV_PREFIX: &str = "HYDROFOIL";
const ENV_SEPARATOR: &str = "__";

/// Secrets sourced from the environment rather than the config file and overlaid
/// onto the corresponding config field at load time, keeping credentials out of
/// the checked-in file. Each entry maps an env var to where it lands.
const SECRET_ENV_KEYS: &[&str] = &[
    "OPENLINEAGE_API_KEY", // -> lineage.api_key
    "UC_TOKEN",            // -> unity.token
    "AWS_REGION",          // -> unity.region (fallback)
];

impl Config {
    /// Load configuration by layering, lowest precedence first:
    ///
    /// 1. struct defaults,
    /// 2. the config file (TOML/YAML/… — `path` if given, otherwise the
    ///    `HYDROFOIL_CONFIG` path if set; a missing file is only an error when
    ///    the path was explicitly requested),
    /// 3. `HYDROFOIL__*` environment overrides (e.g. `HYDROFOIL__PORT=9050`).
    ///
    /// Secrets (`OPENLINEAGE_API_KEY`, `UC_TOKEN`, `AWS_REGION`) are then overlaid
    /// from the environment so they never need to live in the checked-in file —
    /// but only when not already set by the file/env layers above.
    pub fn load(path: Option<impl AsRef<Path>>) -> Result<Self, ConfigError> {
        let path = path
            .map(|p| p.as_ref().to_path_buf())
            .or_else(|| env::var_os(CONFIG_PATH_ENV).map(PathBuf::from));

        let mut builder = ConfigSource::builder();
        if let Some(path) = path {
            // Explicitly requested -> the file must exist and parse.
            builder = builder.add_source(File::from(path).required(true));
        }
        builder = builder.add_source(
            Environment::with_prefix(ENV_PREFIX)
                .separator(ENV_SEPARATOR)
                .try_parsing(true),
        );

        let mut cfg: Config = builder.build()?.try_deserialize()?;
        cfg.overlay_secrets();
        Ok(cfg)
    }

    /// Overlay secret env vars onto their config fields, without clobbering a
    /// value already provided by the file or `HYDROFOIL__*` layers.
    fn overlay_secrets(&mut self) {
        for key in SECRET_ENV_KEYS {
            let Ok(val) = env::var(key) else { continue };
            if val.is_empty() {
                continue;
            }
            match *key {
                "OPENLINEAGE_API_KEY" => self.lineage.api_key.get_or_insert(val),
                "UC_TOKEN" => self.unity.token.get_or_insert(val),
                "AWS_REGION" => self.unity.region.get_or_insert(val),
                _ => continue,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deserialize a TOML body into `Config` the way `load` does (defaults +
    /// file), without touching the process environment.
    fn from_toml(body: &str) -> Result<Config, config::ConfigError> {
        ConfigSource::builder()
            .add_source(File::from_str(body, config::FileFormat::Toml))
            .build()?
            .try_deserialize()
    }

    #[test]
    fn test_defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.port, 50051);
        assert!(cfg.http_enabled);
        assert_eq!(cfg.http_port, 9082);
        assert_eq!(cfg.query_max_limit, 10_000);
        assert_eq!(cfg.query_default_limit, 1_000);
        assert_eq!(cfg.session_ttl_secs, 1800);
        assert!(cfg.lineage.url.is_none());
        assert!(cfg.policy.oci_ref.is_none());
        assert!(cfg.unity.endpoint.is_none());
    }

    #[test]
    fn test_empty_file_is_all_defaults_and_disabled() {
        let cfg = from_toml("").unwrap();
        assert_eq!(cfg.port, 50051);
        // Every integration stays disabled (its enabling field is absent).
        assert!(cfg.lineage.url.is_none());
        assert!(cfg.policy.oci_ref.is_none());
        assert!(cfg.unity.endpoint.is_none());
    }

    #[test]
    fn test_partial_file_overrides_only_named_fields() {
        let cfg = from_toml(
            r#"
            port = 9050
            [unity]
            endpoint = "http://uc:8081/api/2.1/unity-catalog/"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.port, 9050);
        assert_eq!(
            cfg.unity.endpoint.as_deref(),
            Some("http://uc:8081/api/2.1/unity-catalog/")
        );
        // Untouched fields keep their defaults.
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.session_ttl_secs, 1800);
        assert!(cfg.lineage.url.is_none());
    }

    #[test]
    fn test_integrations_parse_from_file() {
        let cfg = from_toml(
            r#"
            [lineage]
            url = "http://marquez:9080"
            namespace = "hydrofoil-live"
            [policy]
            oci_ref = "localhost:10100/hydrofoil/plan-policy:latest"
            [unity]
            endpoint = "http://uc:8081/"
            region = "eu-central-1"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.lineage.url.as_deref(), Some("http://marquez:9080"));
        assert_eq!(cfg.lineage.namespace.as_deref(), Some("hydrofoil-live"));
        assert_eq!(
            cfg.policy.oci_ref.as_deref(),
            Some("localhost:10100/hydrofoil/plan-policy:latest")
        );
        assert_eq!(cfg.unity.endpoint.as_deref(), Some("http://uc:8081/"));
        assert_eq!(cfg.unity.region.as_deref(), Some("eu-central-1"));
    }

    #[test]
    fn test_iceberg_rest_catalogs_parse_from_file() {
        let cfg = from_toml(
            r#"
            [[iceberg.rest_catalogs]]
            name = "lakekeeper"
            uri = "http://lakekeeper:8181/catalog"
            warehouse = "demo"
            token = "irc-token"

            [[iceberg.rest_catalogs]]
            name = "nessie"
            uri = "http://nessie:19120/iceberg"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.iceberg.rest_catalogs.len(), 2);
        let first = &cfg.iceberg.rest_catalogs[0];
        assert_eq!(first.name, "lakekeeper");
        assert_eq!(first.uri, "http://lakekeeper:8181/catalog");
        assert_eq!(first.warehouse.as_deref(), Some("demo"));
        assert_eq!(first.token.as_deref(), Some("irc-token"));
        let second = &cfg.iceberg.rest_catalogs[1];
        assert_eq!(second.name, "nessie");
        assert!(second.warehouse.is_none());
    }

    #[test]
    fn test_iceberg_disabled_by_default() {
        let cfg = from_toml("").unwrap();
        assert!(cfg.iceberg.rest_catalogs.is_empty());
    }

    #[test]
    fn test_http_surface_fields_parse_from_file() {
        let cfg = from_toml(
            r#"
            http_enabled = false
            http_port = 8088
            query_max_limit = 5000
            query_default_limit = 250
            "#,
        )
        .unwrap();
        assert!(!cfg.http_enabled);
        assert_eq!(cfg.http_port, 8088);
        assert_eq!(cfg.query_max_limit, 5000);
        assert_eq!(cfg.query_default_limit, 250);
        // Untouched fields keep their defaults.
        assert_eq!(cfg.port, 50051);
    }

    #[test]
    fn test_malformed_value_is_error() {
        assert!(from_toml("port = \"not-a-port\"").is_err());
    }

    #[test]
    fn test_load_from_file_path() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hydrofoil.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "port = 9055\nsession_ttl_secs = 600\n").unwrap();
        let cfg = Config::load(Some(&path)).unwrap();
        assert_eq!(cfg.port, 9055);
        assert_eq!(cfg.session_ttl_secs, 600);
    }

    #[test]
    fn test_load_missing_explicit_path_is_error() {
        assert!(Config::load(Some("/nonexistent/hydrofoil.toml")).is_err());
    }
}
