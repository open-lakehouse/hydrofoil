//! Layered configuration for the portal server.
//!
//! Mirrors the pattern used by the other workspace services (hydrofoil,
//! lineage-service): struct defaults are overlaid by an optional config file,
//! then by `PORTAL__*` environment overrides, and finally secrets / the Unity
//! endpoint are pulled from the environment so they never need to live in the
//! checked-in file.

use std::env;
use std::path::{Path, PathBuf};

use config::{Config as ConfigSource, Environment, File};
use serde::Deserialize;

/// Error raised while loading configuration. A missing file (when none was
/// explicitly requested) and unset variables both fall back to documented
/// defaults and are *not* errors; a malformed file, an unparsable value, or an
/// unknown files backend is, so a misconfigured deployment refuses to start
/// instead of silently running on defaults.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to load configuration: {0}")]
    Source(#[from] config::ConfigError),

    #[error("unknown files.backend {0:?} (known: memory, unity)")]
    UnknownBackend(String),

    #[error("files.backend = \"unity\" requires files.endpoint or the UNITY_ENDPOINT env var")]
    UnityMissingEndpoint,
}

fn default_port() -> u16 {
    8080
}

fn default_backend() -> String {
    "memory".into()
}

/// Files backend configuration.
///
/// A flat struct with a `backend` discriminator (not a serde-tagged enum) so
/// every field stays overridable by `PORTAL__FILES__*` env vars — the `config`
/// crate's env source merges key-by-key and cannot drive a tagged enum.
/// [`FilesConfig::resolve`] converts it into the validated [`FilesBackend`].
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FilesConfig {
    /// `"memory"` (default, process-local) or `"unity"` (Unity Catalog volumes).
    pub backend: String,
    /// Unity Catalog REST base URL, e.g.
    /// `http://unity-catalog:8081/api/2.1/unity-catalog/`. Falls back to the
    /// `UNITY_ENDPOINT` env var (overlaid at load time). Use `https://` when a
    /// token is set — an `http://` endpoint 301-redirects and drops the bearer.
    pub endpoint: Option<String>,
    /// Optional bearer token for the UC server. Sourced from the `UNITY_TOKEN`
    /// env var (a secret — never read from the file); omit for an
    /// unauthenticated OSS server.
    pub token: Option<String>,
    /// AWS region hint for vended credentials. Falls back to `UNITY_REGION`,
    /// then `AWS_REGION`.
    pub region: Option<String>,
}

impl Default for FilesConfig {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            endpoint: None,
            token: None,
            region: None,
        }
    }
}

/// The validated, resolved files backend produced by [`FilesConfig::resolve`].
#[derive(Debug, Clone)]
pub enum FilesBackend {
    /// In-process store; state is not durable.
    Memory,
    /// Unity Catalog volumes, accessed through the UC REST API.
    Unity {
        endpoint: String,
        token: Option<String>,
        region: Option<String>,
    },
}

impl FilesConfig {
    /// Convert the flat config into a validated [`FilesBackend`].
    ///
    /// Setting `endpoint` (e.g. via `UNITY_ENDPOINT`) implies the `unity`
    /// backend even when `backend` is left at its `memory` default — this
    /// preserves the historical "endpoint set ⇒ use volumes" behavior.
    pub fn resolve(&self) -> Result<FilesBackend, ConfigError> {
        let wants_unity =
            self.backend == "unity" || (self.backend == "memory" && self.endpoint.is_some());
        match (self.backend.as_str(), wants_unity) {
            ("memory", false) => Ok(FilesBackend::Memory),
            ("memory" | "unity", true) => {
                let endpoint = self
                    .endpoint
                    .clone()
                    .ok_or(ConfigError::UnityMissingEndpoint)?;
                Ok(FilesBackend::Unity {
                    endpoint,
                    token: self.token.clone(),
                    region: self.region.clone(),
                })
            }
            (other, _) => Err(ConfigError::UnknownBackend(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub port: u16,
    pub files: FilesConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            files: FilesConfig::default(),
        }
    }
}

/// Environment variable holding the path to the config file. Also accepted as
/// the binary's first positional argument (see `main`).
pub const CONFIG_PATH_ENV: &str = "PORTAL_CONFIG";

/// Prefix and separator for environment overrides of structured config keys,
/// e.g. `PORTAL__PORT=9000` or `PORTAL__FILES__BACKEND=unity`.
const ENV_PREFIX: &str = "PORTAL";
const ENV_SEPARATOR: &str = "__";

impl Config {
    /// Load configuration by layering, lowest precedence first:
    ///
    /// 1. struct defaults,
    /// 2. the config file (TOML/YAML/… — `path` if given, otherwise the
    ///    `PORTAL_CONFIG` path if set; a missing file is only an error when the
    ///    path was explicitly requested),
    /// 3. `PORTAL__*` environment overrides (e.g. `PORTAL__PORT=9000`).
    ///
    /// The Unity endpoint/token/region are then overlaid from the bare
    /// `UNITY_ENDPOINT` / `UNITY_TOKEN` / `UNITY_REGION` (or `AWS_REGION`) env
    /// vars so the token (a secret) never needs to live in the checked-in file
    /// and the historical env-var names keep working.
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
        cfg.overlay_env();
        cfg.validate()?;
        Ok(cfg)
    }

    /// Overlay the Unity endpoint/token/region from the bare environment vars,
    /// keeping credentials out of the checked-in file and preserving the
    /// historical `UNITY_*` / `AWS_REGION` names.
    fn overlay_env(&mut self) {
        if let Some(v) = non_empty_env("UNITY_ENDPOINT") {
            self.files.endpoint = Some(v);
        }
        if let Some(v) = non_empty_env("UNITY_TOKEN") {
            self.files.token = Some(v);
        }
        if let Some(v) = non_empty_env("UNITY_REGION").or_else(|| non_empty_env("AWS_REGION")) {
            self.files.region = Some(v);
        }
    }

    /// Validate cross-cutting invariants that serde can't express on its own —
    /// resolve the files backend so a bad backend / missing endpoint fails at
    /// startup rather than on the first request.
    fn validate(&self) -> Result<(), ConfigError> {
        self.files.resolve().map(|_| ())
    }

    /// The resolved files backend.
    pub fn files_backend(&self) -> Result<FilesBackend, ConfigError> {
        self.files.resolve()
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.is_empty())
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
        assert_eq!(cfg.port, 8080);
        assert_eq!(cfg.files.backend, "memory");
        assert!(cfg.files.endpoint.is_none());
        assert!(matches!(cfg.files.resolve().unwrap(), FilesBackend::Memory));
    }

    #[test]
    fn test_empty_file_is_all_defaults() {
        let cfg = from_toml("").unwrap();
        assert_eq!(cfg.port, 8080);
        assert_eq!(cfg.files.backend, "memory");
    }

    #[test]
    fn test_partial_file_overrides_only_named_fields() {
        let cfg = from_toml(
            r#"
            port = 9000
            [files]
            backend = "unity"
            endpoint = "http://uc:8081/api/2.1/unity-catalog/"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.port, 9000);
        assert_eq!(cfg.files.backend, "unity");
        assert_eq!(
            cfg.files.endpoint.as_deref(),
            Some("http://uc:8081/api/2.1/unity-catalog/")
        );
        // Untouched field keeps its default.
        assert!(cfg.files.token.is_none());
    }

    #[test]
    fn test_malformed_value_is_error() {
        assert!(from_toml("port = \"not-a-port\"").is_err());
    }

    #[test]
    fn test_load_from_file_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("portal.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "port = 8095").unwrap();
        let cfg = Config::load(Some(&path)).unwrap();
        assert_eq!(cfg.port, 8095);
    }

    #[test]
    fn test_load_missing_explicit_path_is_error() {
        assert!(Config::load(Some("/nonexistent/portal.toml")).is_err());
    }

    // --- files backend resolution ---

    #[test]
    fn test_default_backend_resolves_memory() {
        assert!(matches!(
            FilesConfig::default().resolve().unwrap(),
            FilesBackend::Memory
        ));
    }

    #[test]
    fn test_endpoint_implies_unity_even_with_memory_backend() {
        // "endpoint set ⇒ use volumes" — mirrors the historical UNITY_ENDPOINT
        // behavior even when the backend is left at its memory default.
        let cfg = FilesConfig {
            endpoint: Some("http://uc:8081/api/2.1/unity-catalog/".into()),
            ..FilesConfig::default()
        };
        match cfg.resolve().unwrap() {
            FilesBackend::Unity { endpoint, .. } => {
                assert_eq!(endpoint, "http://uc:8081/api/2.1/unity-catalog/");
            }
            other => panic!("expected Unity, got {other:?}"),
        }
    }

    #[test]
    fn test_unity_backend_missing_endpoint_errors() {
        let cfg = FilesConfig {
            backend: "unity".into(),
            ..FilesConfig::default()
        };
        assert!(matches!(
            cfg.resolve(),
            Err(ConfigError::UnityMissingEndpoint)
        ));
    }

    #[test]
    fn test_unknown_backend_errors() {
        let cfg = FilesConfig {
            backend: "s3fs".into(),
            ..FilesConfig::default()
        };
        assert!(matches!(
            cfg.resolve(),
            Err(ConfigError::UnknownBackend(ref b)) if b == "s3fs"
        ));
    }
}
