use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use config::{Config as ConfigSource, Environment, File};
use serde::Deserialize;

/// Which lakehouse sink the service should fan a `WriteBatch` out to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SinkKind {
    Delta,
    /// Apache Iceberg. Only available when the crate is built with the
    /// `iceberg` cargo feature.
    Iceberg,
}

/// Error raised while loading configuration. A missing file (when none was
/// explicitly requested) and unset variables both fall back to documented
/// defaults and are *not* errors; a malformed file, an unparsable value, or an
/// unsupported sink is, so a misconfigured deployment refuses to start instead
/// of silently running on defaults.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to load configuration: {0}")]
    Source(#[from] config::ConfigError),

    #[error(
        "config selects the iceberg sink, but this binary was built without the `iceberg` feature; rebuild with `--features iceberg`"
    )]
    #[cfg(not(feature = "iceberg"))]
    IcebergNotCompiled,
}

fn default_table_path() -> String {
    "/data/events".into()
}

fn default_partition_cols() -> Vec<String> {
    vec!["event_kind".into()]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DeltaConfig {
    pub table_path: String,
    pub partition_cols: Vec<String>,
}

impl Default for DeltaConfig {
    fn default() -> Self {
        Self {
            table_path: default_table_path(),
            partition_cols: default_partition_cols(),
        }
    }
}

#[cfg(feature = "iceberg")]
#[derive(Debug, Clone, Deserialize)]
pub struct IcebergConfig {
    /// Iceberg REST catalog URI. For Lakekeeper this looks like
    /// `http://lakekeeper:8181/catalog`.
    pub catalog_uri: String,
    /// REST `warehouse` property — for Lakekeeper this is the warehouse name
    /// (e.g. `lineage`), not an S3 path. For other servers it may be an
    /// `s3://bucket/prefix` URI.
    pub warehouse: String,
    pub namespace: String,
    pub table: String,
    /// Identity-transform partition columns. Empty means an unpartitioned
    /// table.
    #[serde(default)]
    pub partition_cols: Vec<String>,
    /// Optional bearer token to attach to REST requests (Lakekeeper OIDC).
    /// Forwarded to the catalog as the REST `token` property by
    /// `iceberg::build_rest_props`. A secret — supply via the `ICEBERG_TOKEN`
    /// env var rather than the config file.
    #[serde(default)]
    pub token: Option<String>,
}

/// Tuning for the asynchronous buffered writer that sits between HTTP
/// ingestion and the sinks. Defaults mirror the historical Go forwarder.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default)]
pub struct WriterConfig {
    /// Flush once this many events are buffered.
    pub buffer_size: usize,
    /// Flush at least this often, even below `buffer_size`.
    pub flush_interval_ms: u64,
    /// Bounded ingestion channel depth; `enqueue` applies backpressure once
    /// full.
    pub channel_capacity: usize,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            buffer_size: 100,
            flush_interval_ms: 500,
            channel_capacity: 1000,
        }
    }
}

fn default_port() -> u16 {
    8091
}

fn default_sinks() -> Vec<SinkKind> {
    vec![SinkKind::Delta]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub port: u16,
    pub sinks: Vec<SinkKind>,
    pub delta: DeltaConfig,
    #[cfg(feature = "iceberg")]
    pub iceberg: Option<IcebergConfig>,
    /// Object-store options (region, endpoint, …) forwarded to the writer.
    /// Secrets (`AWS_*`, tokens) are layered in from the environment at load
    /// time rather than read from the config file — see [`Config::load`].
    pub storage_options: HashMap<String, String>,
    pub writer: WriterConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            sinks: default_sinks(),
            delta: DeltaConfig::default(),
            #[cfg(feature = "iceberg")]
            iceberg: None,
            storage_options: HashMap::new(),
            writer: WriterConfig::default(),
        }
    }
}

/// Secrets and object-store options sourced from the environment rather than the
/// config file, and layered into [`Config::storage_options`] at load time. Keeps
/// credentials out of the checked-in file. The map key is the lowercased env-var
/// name, matching what the Delta/object-store layer expects.
const STORAGE_OPTION_ENV_KEYS: &[&str] = &[
    "AWS_REGION",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_ENDPOINT_URL",
    "AWS_S3_ALLOW_UNSAFE_RENAME",
    "UNITY_CATALOG_URL",
    "UNITY_CATALOG_TOKEN",
];

/// Environment variable holding the path to the config file. Also accepted as
/// the binary's first positional argument (see `main`).
pub const CONFIG_PATH_ENV: &str = "LINEAGE_CONFIG";

/// Prefix and separator for environment overrides of structured config keys,
/// e.g. `LINEAGE__PORT=9000` or `LINEAGE__WRITER__BUFFER_SIZE=200`.
const ENV_PREFIX: &str = "LINEAGE";
const ENV_SEPARATOR: &str = "__";

impl Config {
    /// Load configuration by layering, lowest precedence first:
    ///
    /// 1. struct defaults,
    /// 2. the config file (TOML/YAML/… — `path` if given, otherwise the
    ///    `LINEAGE_CONFIG` path if set; a missing file is only an error when the
    ///    path was explicitly requested),
    /// 3. `LINEAGE__*` environment overrides (e.g. `LINEAGE__PORT=9000`).
    ///
    /// Secrets and object-store options (`AWS_*`, `UNITY_CATALOG_*`) are then
    /// overlaid into `storage_options` from the environment so they never need
    /// to live in the checked-in file.
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
                .list_separator(",")
                .with_list_parse_key("sinks")
                .with_list_parse_key("delta.partition_cols")
                .try_parsing(true),
        );

        let mut cfg: Config = builder.build()?.try_deserialize()?;

        for key in STORAGE_OPTION_ENV_KEYS {
            if let Ok(val) = env::var(key) {
                cfg.storage_options.insert(key.to_lowercase(), val);
            }
        }

        cfg.validate()?;
        Ok(cfg)
    }

    /// Validate cross-cutting invariants that serde can't express on its own.
    fn validate(&self) -> Result<(), ConfigError> {
        #[cfg(not(feature = "iceberg"))]
        if self.sinks.contains(&SinkKind::Iceberg) {
            return Err(ConfigError::IcebergNotCompiled);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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
        assert_eq!(cfg.port, 8091);
        assert_eq!(cfg.delta.table_path, "/data/events");
        assert_eq!(cfg.delta.partition_cols, vec!["event_kind"]);
        assert_eq!(cfg.sinks, vec![SinkKind::Delta]);
    }

    #[test]
    fn test_empty_file_is_all_defaults() {
        let cfg = from_toml("").unwrap();
        assert_eq!(cfg.port, 8091);
        assert_eq!(cfg.sinks, vec![SinkKind::Delta]);
        assert_eq!(cfg.delta.table_path, "/data/events");
        assert_eq!(cfg.writer.buffer_size, 100);
    }

    #[test]
    fn test_partial_file_overrides_only_named_fields() {
        let cfg = from_toml(
            r#"
            port = 9000
            [delta]
            table_path = "s3://bucket/events"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.port, 9000);
        assert_eq!(cfg.delta.table_path, "s3://bucket/events");
        // Untouched fields keep their defaults.
        assert_eq!(cfg.delta.partition_cols, vec!["event_kind"]);
        assert_eq!(cfg.writer.flush_interval_ms, 500);
        assert_eq!(cfg.sinks, vec![SinkKind::Delta]);
    }

    #[test]
    fn test_sinks_and_partition_cols_parse_from_file() {
        let cfg = from_toml(
            r#"
            sinks = ["delta"]
            [delta]
            partition_cols = ["event_kind", "event_type"]
            [writer]
            buffer_size = 250
            "#,
        )
        .unwrap();
        assert_eq!(cfg.sinks, vec![SinkKind::Delta]);
        assert_eq!(cfg.delta.partition_cols, vec!["event_kind", "event_type"]);
        assert_eq!(cfg.writer.buffer_size, 250);
    }

    #[test]
    fn test_unknown_sink_is_error() {
        let err = from_toml(r#"sinks = ["hudi"]"#).unwrap_err();
        assert!(
            err.to_string().contains("hudi") || err.to_string().contains("enum"),
            "unknown sinks fail to deserialize: {err}",
        );
    }

    #[test]
    fn test_malformed_value_is_error() {
        assert!(from_toml("port = \"not-a-port\"").is_err());
    }

    #[cfg(not(feature = "iceberg"))]
    #[test]
    fn test_iceberg_sink_without_feature_fails_validation() {
        // `iceberg` deserializes fine (the variant always exists), but
        // validate() rejects it when the feature is off.
        let cfg = Config {
            sinks: vec![SinkKind::Iceberg],
            ..Config::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(ConfigError::IcebergNotCompiled)
        ));
    }

    #[test]
    fn test_load_from_file_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lineage-service.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "port = 8095\n[delta]\ntable_path = \"/tmp/ev\"\n").unwrap();
        let cfg = Config::load(Some(&path)).unwrap();
        assert_eq!(cfg.port, 8095);
        assert_eq!(cfg.delta.table_path, "/tmp/ev");
    }

    #[test]
    fn test_load_missing_explicit_path_is_error() {
        assert!(Config::load(Some("/nonexistent/lineage-service.toml")).is_err());
    }

    #[test]
    fn test_writer_defaults() {
        let w = WriterConfig::default();
        assert_eq!(w.buffer_size, 100);
        assert_eq!(w.flush_interval_ms, 500);
        assert_eq!(w.channel_capacity, 1000);
        assert_eq!(Config::default().writer.buffer_size, 100);
    }
}
