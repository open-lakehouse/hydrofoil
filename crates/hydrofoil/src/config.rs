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
///
/// **Deprecated** in favor of the [`[governance]`](GovernanceConfig) section; a
/// bare `[policy] oci_ref` is still accepted for one release as an alias that
/// maps to `posture = "enforcing", sources = ["cedar-oci"]` (with a warning at
/// startup). See [`GovernanceConfig::resolve`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PolicyConfig {
    /// OCI reference to a Cedar policy image, e.g.
    /// `localhost:10100/hydrofoil/plan-policy:latest`.
    pub oci_ref: Option<String>,
}

/// The runtime enforcement posture, chosen explicitly (never the accidental
/// result of a forgotten wire-up). Maps 1:1 onto
/// [`datafusion_policy::Posture`]. Defaults to [`Disabled`](Posture::Disabled) —
/// an ungoverned server must be a deliberate, logged choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Posture {
    /// No enforcement installed at all (the only ungoverned mode).
    #[default]
    Disabled,
    /// Evaluate + structured-log what *would* be enforced, enforce nothing
    /// (the rollout / dry-run stance).
    Permissive,
    /// Fail-closed enforcement (the production stance).
    Enforcing,
}

impl From<Posture> for datafusion_policy::Posture {
    fn from(p: Posture) -> Self {
        match p {
            Posture::Disabled => datafusion_policy::Posture::Disabled,
            Posture::Permissive => datafusion_policy::Posture::Permissive,
            Posture::Enforcing => datafusion_policy::Posture::Enforcing,
        }
    }
}

/// A policy engine source composed into the runtime [`Governance`] profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicySource {
    /// The UC-ABAC engine over `/policies`-sourced bindings (no Cedar/OCI).
    UnityAbac,
    /// The Cedar engine loaded from an OCI policy image.
    CedarOci,
}

/// The identity provider (principal enrichment PIP) selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityProvider {
    /// No enrichment (pass-through). The default.
    #[default]
    None,
    /// The built-in demo directory map — a fixed set of users/groups for local
    /// experimentation. Only constructed when explicitly chosen.
    Demo,
}

/// The Cedar-source sub-config, read only when `sources` includes `cedar-oci`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CedarConfig {
    /// OCI reference to a Cedar policy image, e.g.
    /// `localhost:10100/hydrofoil/plan-policy:latest`.
    pub oci_ref: Option<String>,
}

/// The identity sub-config.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct IdentityConfig {
    /// Which identity provider to install. Defaults to `none` (pass-through).
    pub provider: IdentityProvider,
}

/// Governance (policy enforcement) configuration — the single, explicit source
/// of the runtime [`Posture`], the composed engine sources, and the identity
/// provider. Replaces the fail-*open* `[policy]` section: an unconfigured server
/// is `disabled` (loudly logged), never a silent allow-all.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GovernanceConfig {
    /// The enforcement posture. Defaults to [`Posture::Disabled`].
    pub posture: Posture,
    /// The engine sources composed into the profile. Empty under `disabled`;
    /// required (non-empty) otherwise.
    pub sources: Vec<PolicySource>,
    /// Policy-binding cache TTL (seconds) for the UC `/policies` provider.
    pub policy_cache_ttl_secs: Option<u64>,
    /// Cedar-source settings (read only when `sources` includes `cedar-oci`).
    pub cedar: CedarConfig,
    /// Identity provider settings.
    pub identity: IdentityConfig,
}

/// Error raised when the `[governance]` configuration cannot enforce what its
/// posture promises — surfaced at startup so a misconfigured deployment refuses
/// to start rather than silently under-enforcing.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GovernanceConfigError {
    #[error(
        "governance posture is {posture:?} but no sources are configured; \
         set governance.sources (e.g. [\"unity-abac\"]) or posture = \"disabled\""
    )]
    NoSources { posture: Posture },
    #[error(
        "governance sources include \"unity-abac\" but unity.endpoint is not set; \
         UC-ABAC policies are fetched from Unity Catalog"
    )]
    UnityAbacWithoutEndpoint,
    #[error("governance sources include \"cedar-oci\" but governance.cedar.oci_ref is not set")]
    CedarOciWithoutRef,
}

/// The validated, normalized governance profile the server wiring consumes — the
/// output of [`Config::resolve_governance`] after applying the deprecated
/// `[policy]` alias and the startup validation. All invariants the enforcement
/// layer relies on hold here (an enforcing posture has ≥1 source; each source's
/// required config is present), so `main.rs` can build the `Governance` without
/// re-checking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedGovernance {
    /// The runtime posture.
    pub posture: Posture,
    /// The engine sources (empty only under `disabled`).
    pub sources: Vec<PolicySource>,
    /// The Cedar OCI reference (present iff `sources` includes `cedar-oci`).
    pub cedar_oci_ref: Option<String>,
    /// The policy-binding cache TTL for the UC provider.
    pub policy_cache_ttl_secs: Option<u64>,
    /// The identity provider selection.
    pub identity_provider: IdentityProvider,
}

impl ResolvedGovernance {
    /// Whether the UC-ABAC engine is one of the composed sources.
    pub fn has_unity_abac(&self) -> bool {
        self.sources.contains(&PolicySource::UnityAbac)
    }
    /// Whether the Cedar/OCI engine is one of the composed sources.
    pub fn has_cedar_oci(&self) -> bool {
        self.sources.contains(&PolicySource::CedarOci)
    }
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

/// Default dev object store: the seaweedfs S3 gateway the UC quickstart runs.
/// The values are the historical hardcoded defaults, now overridable.
fn default_object_store_endpoint() -> String {
    "http://localhost:8333/".into()
}
fn default_object_store_bucket() -> String {
    "open-lakehouse".into()
}
fn default_object_store_access_key_id() -> String {
    "seaweed-key-id".into()
}
fn default_object_store_secret_access_key() -> String {
    "seaweed-access-key".into()
}

/// The S3-compatible object store registered on every session's runtime. The
/// defaults reproduce the historical hardcoded seaweedfs dev gateway (the UC
/// quickstart's S3 endpoint); a deployment overrides them via `[object-store]`.
/// Credentials are dev defaults, overridable via env secrets.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ObjectStoreConfig {
    /// S3 endpoint URL (e.g. `http://localhost:8333/` for the seaweedfs gateway).
    pub endpoint: String,
    /// Bucket the lakehouse data lives in.
    pub bucket: String,
    /// S3 access key id. A dev default; override via `AWS_ACCESS_KEY_ID`.
    pub access_key_id: String,
    /// S3 secret access key. A dev default; override via `AWS_SECRET_ACCESS_KEY`.
    pub secret_access_key: String,
    /// Whether to allow plain HTTP (true for the local dev gateway).
    pub allow_http: bool,
}

impl Default for ObjectStoreConfig {
    fn default() -> Self {
        Self {
            endpoint: default_object_store_endpoint(),
            bucket: default_object_store_bucket(),
            access_key_id: default_object_store_access_key_id(),
            secret_access_key: default_object_store_secret_access_key(),
            allow_http: true,
        }
    }
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
    /// Deprecated: superseded by [`governance`](Self::governance). A bare
    /// `[policy] oci_ref` is accepted as an alias for one release.
    pub policy: PolicyConfig,
    pub unity: UnityConfig,
    pub governance: GovernanceConfig,
    #[serde(rename = "object-store")]
    pub object_store: ObjectStoreConfig,
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
            governance: GovernanceConfig::default(),
            object_store: ObjectStoreConfig::default(),
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

    /// Resolve and validate the runtime governance profile from the
    /// `[governance]` section, applying the deprecated `[policy]` alias, and
    /// **refusing to start** (returning `Err`) when the configuration cannot
    /// enforce what its posture promises. This replaces the previous silent
    /// fail-*open* (a missing `policy.oci_ref` yielding allow-all).
    ///
    /// Precedence: an explicit `[governance]` section (posture other than the
    /// default `disabled`, or any `sources`) wins. Otherwise a bare `[policy]
    /// oci_ref` is honored as the deprecated alias — mapping to
    /// `posture = enforcing, sources = [cedar-oci]`, with a warning logged.
    ///
    /// Validation (all fail closed at startup):
    /// - a non-`disabled` posture requires ≥1 source;
    /// - `unity-abac` requires `unity.endpoint`;
    /// - `cedar-oci` requires `governance.cedar.oci_ref`.
    pub fn resolve_governance(&self) -> Result<ResolvedGovernance, GovernanceConfigError> {
        let gov = &self.governance;
        let governance_configured = gov.posture != Posture::Disabled || !gov.sources.is_empty();

        // The deprecated `[policy] oci_ref` alias, honored only when no explicit
        // `[governance]` section is present.
        let legacy_oci = self.policy.oci_ref.as_deref().filter(|r| !r.is_empty());

        let (posture, sources, cedar_oci_ref) = if governance_configured {
            let cedar_ref = gov.cedar.oci_ref.clone().filter(|r| !r.is_empty());
            (gov.posture, gov.sources.clone(), cedar_ref)
        } else if let Some(reference) = legacy_oci {
            tracing::warn!(
                target: "hydrofoil::config",
                "[policy] oci_ref is deprecated; use [governance] posture=\"enforcing\" \
                 sources=[\"cedar-oci\"] with [governance.cedar] oci_ref=... — mapping the \
                 legacy value for now"
            );
            (
                Posture::Enforcing,
                vec![PolicySource::CedarOci],
                Some(reference.to_string()),
            )
        } else {
            // Nothing configured: disabled, and say so loudly.
            tracing::warn!(
                target: "hydrofoil::config",
                "governance posture is DISABLED (no [governance] section and no [policy] \
                 oci_ref); the server runs UNGOVERNED — set [governance] to enable enforcement"
            );
            (Posture::Disabled, Vec::new(), None)
        };

        // Validation — fail closed at startup.
        if posture != Posture::Disabled && sources.is_empty() {
            return Err(GovernanceConfigError::NoSources { posture });
        }
        if sources.contains(&PolicySource::UnityAbac)
            && self
                .unity
                .endpoint
                .as_deref()
                .filter(|e| !e.is_empty())
                .is_none()
        {
            return Err(GovernanceConfigError::UnityAbacWithoutEndpoint);
        }
        if sources.contains(&PolicySource::CedarOci) && cedar_oci_ref.is_none() {
            return Err(GovernanceConfigError::CedarOciWithoutRef);
        }

        Ok(ResolvedGovernance {
            posture,
            sources,
            cedar_oci_ref,
            policy_cache_ttl_secs: gov.policy_cache_ttl_secs,
            identity_provider: gov.identity.provider,
        })
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

    // --- [governance] + [object-store] (H6) ---

    #[test]
    fn governance_defaults_to_disabled() {
        let cfg = from_toml("").unwrap();
        assert_eq!(cfg.governance.posture, Posture::Disabled);
        assert!(cfg.governance.sources.is_empty());
        assert_eq!(cfg.governance.identity.provider, IdentityProvider::None);
        // Resolving an empty config yields a disabled profile (no error).
        let resolved = cfg.resolve_governance().unwrap();
        assert_eq!(resolved.posture, Posture::Disabled);
        assert!(resolved.sources.is_empty());
    }

    #[test]
    fn object_store_defaults_are_the_seaweedfs_dev_gateway() {
        let cfg = from_toml("").unwrap();
        assert_eq!(cfg.object_store.endpoint, "http://localhost:8333/");
        assert_eq!(cfg.object_store.bucket, "open-lakehouse");
        assert_eq!(cfg.object_store.access_key_id, "seaweed-key-id");
        assert_eq!(cfg.object_store.secret_access_key, "seaweed-access-key");
        assert!(cfg.object_store.allow_http);
    }

    #[test]
    fn object_store_section_overrides_defaults() {
        let cfg = from_toml(
            r#"
            [object-store]
            endpoint = "http://minio:9000/"
            bucket = "lake"
            allow_http = false
            "#,
        )
        .unwrap();
        assert_eq!(cfg.object_store.endpoint, "http://minio:9000/");
        assert_eq!(cfg.object_store.bucket, "lake");
        assert!(!cfg.object_store.allow_http);
        // Untouched credentials keep their dev defaults.
        assert_eq!(cfg.object_store.access_key_id, "seaweed-key-id");
    }

    #[test]
    fn governance_unity_abac_parses_and_resolves() {
        let cfg = from_toml(
            r#"
            [unity]
            endpoint = "http://uc:8081/"
            [governance]
            posture = "enforcing"
            sources = ["unity-abac"]
            policy_cache_ttl_secs = 15
            [governance.identity]
            provider = "demo"
            "#,
        )
        .unwrap();
        let resolved = cfg.resolve_governance().unwrap();
        assert_eq!(resolved.posture, Posture::Enforcing);
        assert!(resolved.has_unity_abac());
        assert!(!resolved.has_cedar_oci());
        assert_eq!(resolved.policy_cache_ttl_secs, Some(15));
        assert_eq!(resolved.identity_provider, IdentityProvider::Demo);
    }

    #[test]
    fn governance_enforcing_unity_abac_without_endpoint_is_rejected() {
        let cfg = from_toml(
            r#"
            [governance]
            posture = "enforcing"
            sources = ["unity-abac"]
            "#,
        )
        .unwrap();
        assert_eq!(
            cfg.resolve_governance(),
            Err(GovernanceConfigError::UnityAbacWithoutEndpoint)
        );
    }

    #[test]
    fn governance_cedar_oci_without_ref_is_rejected() {
        let cfg = from_toml(
            r#"
            [governance]
            posture = "enforcing"
            sources = ["cedar-oci"]
            "#,
        )
        .unwrap();
        assert_eq!(
            cfg.resolve_governance(),
            Err(GovernanceConfigError::CedarOciWithoutRef)
        );
    }

    #[test]
    fn governance_cedar_oci_with_ref_resolves() {
        let cfg = from_toml(
            r#"
            [governance]
            posture = "permissive"
            sources = ["cedar-oci"]
            [governance.cedar]
            oci_ref = "localhost:10100/hydrofoil/plan-policy:latest"
            "#,
        )
        .unwrap();
        let resolved = cfg.resolve_governance().unwrap();
        assert_eq!(resolved.posture, Posture::Permissive);
        assert!(resolved.has_cedar_oci());
        assert_eq!(
            resolved.cedar_oci_ref.as_deref(),
            Some("localhost:10100/hydrofoil/plan-policy:latest")
        );
    }

    #[test]
    fn governance_non_disabled_without_sources_is_rejected() {
        let cfg = from_toml(
            r#"
            [governance]
            posture = "enforcing"
            "#,
        )
        .unwrap();
        assert_eq!(
            cfg.resolve_governance(),
            Err(GovernanceConfigError::NoSources {
                posture: Posture::Enforcing
            })
        );
    }

    #[test]
    fn legacy_policy_oci_ref_maps_to_cedar_enforcing() {
        // The deprecated `[policy] oci_ref` alias (no `[governance]` section).
        let cfg = from_toml(
            r#"
            [policy]
            oci_ref = "localhost:10100/hydrofoil/plan-policy:latest"
            "#,
        )
        .unwrap();
        let resolved = cfg.resolve_governance().unwrap();
        assert_eq!(resolved.posture, Posture::Enforcing);
        assert!(resolved.has_cedar_oci());
        assert_eq!(
            resolved.cedar_oci_ref.as_deref(),
            Some("localhost:10100/hydrofoil/plan-policy:latest")
        );
    }

    #[test]
    fn explicit_governance_wins_over_legacy_policy_alias() {
        // Both present: the explicit `[governance]` section takes precedence, and
        // the legacy `[policy] oci_ref` is ignored.
        let cfg = from_toml(
            r#"
            [policy]
            oci_ref = "localhost:10100/legacy:latest"
            [unity]
            endpoint = "http://uc:8081/"
            [governance]
            posture = "enforcing"
            sources = ["unity-abac"]
            "#,
        )
        .unwrap();
        let resolved = cfg.resolve_governance().unwrap();
        assert!(resolved.has_unity_abac());
        assert!(!resolved.has_cedar_oci());
        assert_eq!(resolved.cedar_oci_ref, None);
    }
}
