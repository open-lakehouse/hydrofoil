use std::collections::HashMap;
use std::env;

/// Which lakehouse sink the service should fan a `WriteBatch` out to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkKind {
    Delta,
    /// Apache Iceberg. Only available when the crate is built with the
    /// `iceberg` cargo feature.
    Iceberg,
}

/// Error raised when the environment carries a value the service cannot run
/// with. Unset variables fall back to documented defaults and are *not* errors;
/// a variable that is *set but unparsable* is, so a misconfigured deployment
/// refuses to start instead of silently running on defaults.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("env var {key} has invalid value {value:?}: {reason}")]
    Invalid {
        key: &'static str,
        value: String,
        reason: String,
    },

    #[error("TABLE_SINKS contains unknown sink kind {0:?} (known: delta, iceberg)")]
    UnknownSink(String),

    #[cfg(not(feature = "iceberg"))]
    #[error(
        "TABLE_SINKS requests the iceberg sink, but this binary was built without the `iceberg` feature; rebuild with `--features iceberg`"
    )]
    IcebergNotCompiled,
}

#[derive(Debug, Clone)]
pub struct DeltaConfig {
    pub table_path: String,
    pub partition_cols: Vec<String>,
}

impl Default for DeltaConfig {
    fn default() -> Self {
        Self {
            table_path: "/data/events".into(),
            partition_cols: vec!["event_kind".into()],
        }
    }
}

#[cfg(feature = "iceberg")]
#[derive(Debug, Clone)]
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
    pub partition_cols: Vec<String>,
    /// Optional bearer token to attach to REST requests (Lakekeeper OIDC).
    /// Forwarded to the catalog as the REST `token` property by
    /// `iceberg::build_rest_props` (sourced from the `ICEBERG_TOKEN` env var).
    pub token: Option<String>,
}

/// Tuning for the asynchronous buffered writer that sits between HTTP
/// ingestion and the sinks. Defaults mirror the historical Go forwarder.
#[derive(Debug, Clone, Copy)]
pub struct WriterConfig {
    /// Flush once this many events are buffered (`BUFFER_SIZE`).
    pub buffer_size: usize,
    /// Flush at least this often, even below `buffer_size` (`FLUSH_INTERVAL_MS`).
    pub flush_interval_ms: u64,
    /// Bounded ingestion channel depth; `enqueue` applies backpressure once
    /// full (`CHANNEL_CAPACITY`).
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

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub sinks: Vec<SinkKind>,
    pub delta: DeltaConfig,
    #[cfg(feature = "iceberg")]
    pub iceberg: Option<IcebergConfig>,
    pub storage_options: HashMap<String, String>,
    pub writer: WriterConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: 8091,
            sinks: vec![SinkKind::Delta],
            delta: DeltaConfig::default(),
            #[cfg(feature = "iceberg")]
            iceberg: None,
            storage_options: HashMap::new(),
            writer: WriterConfig::default(),
        }
    }
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let port = env_parse("LINEAGE_SERVICE_PORT", 8091u16)?;

        let sinks = parse_sinks(&env::var("TABLE_SINKS").unwrap_or_else(|_| "delta".into()))?;

        let table_path = env::var("DELTA_TABLE_PATH").unwrap_or_else(|_| "/data/events".into());

        let partition_cols = env::var("DELTA_PARTITION_COLS")
            .unwrap_or_else(|_| "event_kind".into())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let mut storage_options = HashMap::new();
        Self::add_env(&mut storage_options, "AWS_REGION");
        Self::add_env(&mut storage_options, "AWS_ACCESS_KEY_ID");
        Self::add_env(&mut storage_options, "AWS_SECRET_ACCESS_KEY");
        Self::add_env(&mut storage_options, "AWS_ENDPOINT_URL");
        Self::add_env(&mut storage_options, "AWS_S3_ALLOW_UNSAFE_RENAME");
        Self::add_env(&mut storage_options, "UNITY_CATALOG_URL");
        Self::add_env(&mut storage_options, "UNITY_CATALOG_TOKEN");

        #[cfg(feature = "iceberg")]
        let iceberg = if sinks.contains(&SinkKind::Iceberg) {
            Some(iceberg_from_env())
        } else {
            None
        };

        let defaults = WriterConfig::default();
        let writer = WriterConfig {
            buffer_size: env_parse("BUFFER_SIZE", defaults.buffer_size)?,
            flush_interval_ms: env_parse("FLUSH_INTERVAL_MS", defaults.flush_interval_ms)?,
            channel_capacity: env_parse("CHANNEL_CAPACITY", defaults.channel_capacity)?,
        };

        Ok(Config {
            port,
            sinks,
            delta: DeltaConfig {
                table_path,
                partition_cols,
            },
            #[cfg(feature = "iceberg")]
            iceberg,
            storage_options,
            writer,
        })
    }

    fn add_env(opts: &mut HashMap<String, String>, key: &str) {
        if let Ok(val) = env::var(key) {
            opts.insert(key.to_lowercase(), val);
        }
    }
}

/// Read `key` and parse it as `T`. An unset variable yields `default`; a value
/// that is set but fails to parse is a hard [`ConfigError::Invalid`] so a
/// typo'd deployment fails fast instead of silently running on `default`.
fn env_parse<T: std::str::FromStr>(key: &'static str, default: T) -> Result<T, ConfigError>
where
    T::Err: std::fmt::Display,
{
    match env::var(key) {
        Ok(v) => v.parse().map_err(|e: T::Err| ConfigError::Invalid {
            key,
            value: v,
            reason: e.to_string(),
        }),
        Err(_) => Ok(default),
    }
}

fn parse_sinks(raw: &str) -> Result<Vec<SinkKind>, ConfigError> {
    let mut parsed = Vec::new();
    for token in raw.split(',').map(|s| s.trim().to_ascii_lowercase()) {
        if token.is_empty() {
            continue;
        }
        match token.as_str() {
            "delta" => parsed.push(SinkKind::Delta),
            "iceberg" => {
                #[cfg(not(feature = "iceberg"))]
                return Err(ConfigError::IcebergNotCompiled);
                #[cfg(feature = "iceberg")]
                parsed.push(SinkKind::Iceberg);
            }
            other => return Err(ConfigError::UnknownSink(other.to_string())),
        }
    }
    if parsed.is_empty() {
        Ok(vec![SinkKind::Delta])
    } else {
        Ok(parsed)
    }
}

#[cfg(feature = "iceberg")]
fn iceberg_from_env() -> IcebergConfig {
    let catalog_uri =
        env::var("ICEBERG_CATALOG_URI").unwrap_or_else(|_| "http://localhost:8181/catalog".into());
    let warehouse = env::var("ICEBERG_WAREHOUSE").unwrap_or_else(|_| "lineage".into());
    let namespace = env::var("ICEBERG_NAMESPACE").unwrap_or_else(|_| "lineage".into());
    let table = env::var("ICEBERG_TABLE").unwrap_or_else(|_| "events".into());
    let partition_cols = env::var("ICEBERG_PARTITION_COLS")
        .unwrap_or_else(|_| "event_kind".into())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let token = env::var("ICEBERG_TOKEN").ok().filter(|s| !s.is_empty());
    IcebergConfig {
        catalog_uri,
        warehouse,
        namespace,
        table,
        partition_cols,
        token,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.port, 8091);
        assert_eq!(cfg.delta.table_path, "/data/events");
        assert_eq!(cfg.delta.partition_cols, vec!["event_kind"]);
        assert_eq!(cfg.sinks, vec![SinkKind::Delta]);
    }

    #[test]
    fn test_parse_sinks_default_when_empty() {
        assert_eq!(parse_sinks("").unwrap(), vec![SinkKind::Delta]);
    }

    #[test]
    fn test_parse_sinks_delta() {
        assert_eq!(parse_sinks(" Delta ").unwrap(), vec![SinkKind::Delta]);
    }

    #[test]
    fn test_parse_sinks_unknown_is_error() {
        let err = parse_sinks("hudi,delta").unwrap_err();
        assert!(
            matches!(err, ConfigError::UnknownSink(ref s) if s == "hudi"),
            "unknown sinks fail loudly so a misconfigured deploy refuses to start: {err}",
        );
    }

    #[cfg(feature = "iceberg")]
    #[test]
    fn test_parse_sinks_dual_write_order_preserved() {
        assert_eq!(
            parse_sinks(" Delta , iceberg ").unwrap(),
            vec![SinkKind::Delta, SinkKind::Iceberg]
        );
    }

    #[cfg(not(feature = "iceberg"))]
    #[test]
    fn test_parse_sinks_iceberg_without_feature_is_error() {
        assert!(matches!(
            parse_sinks("iceberg").unwrap_err(),
            ConfigError::IcebergNotCompiled
        ));
    }

    #[test]
    fn test_env_parse_set_but_unparsable_is_error() {
        // SAFETY: single-threaded test; restore the var after.
        unsafe { env::set_var("LINEAGE_SERVICE_PORT", "not-a-port") };
        let got = env_parse("LINEAGE_SERVICE_PORT", 8091u16);
        unsafe { env::remove_var("LINEAGE_SERVICE_PORT") };
        assert!(matches!(got, Err(ConfigError::Invalid { key, .. }) if key == "LINEAGE_SERVICE_PORT"));
    }

    #[test]
    fn test_writer_defaults() {
        let w = WriterConfig::default();
        assert_eq!(w.buffer_size, 100);
        assert_eq!(w.flush_interval_ms, 500);
        assert_eq!(w.channel_capacity, 1000);
        // The full Config default carries the same writer defaults.
        assert_eq!(Config::default().writer.buffer_size, 100);
    }
}
