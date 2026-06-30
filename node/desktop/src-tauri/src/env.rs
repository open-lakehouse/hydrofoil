//! Environments: the registry model, on-disk persistence, and the active-environment
//! state the Tauri commands snapshot against.
//!
//! An environment is a named bundle of service configuration. Creating one writes
//! a registry entry + data dirs (no services); *starting* one (see
//! [`crate::commands::start_environment`]) spawns the UC sidecar, brings up the
//! selected modules, and binds the in-process executors via [`activate_endpoint`].

use std::sync::{Arc, RwLock};

use tauri::Manager;

use desktop_host::{HostConfig, Hosted};

use crate::paths::{app_data_dir, env_home_dir};

/// The services bound to the currently-active environment: the in-process
/// executors plus the resolved UC endpoint. `None` until an environment is
/// selected (the outer shell spawns services lazily on selection).
#[derive(Clone, Default)]
pub(crate) struct ActiveEnv {
    /// The active environment's id. `None` until one is started. Lets the shell
    /// highlight the running environment in the overview.
    pub(crate) id: Option<String>,
    pub(crate) hosted: Option<Arc<Hosted>>,
    /// Resolved Unity Catalog REST base (the spawned sidecar's dynamic endpoint),
    /// e.g. `http://127.0.0.1:PORT/api/2.1/unity-catalog/`. `None` when UC is
    /// disabled (files run in-memory); the proxy then errors.
    pub(crate) unity_endpoint: Option<String>,
    /// Resolved OpenLineage sink base (the Envoy gateway base on the host), e.g.
    /// `http://localhost:9080`. `Some` only when the environment carries the
    /// lineage capability; injected into the marimo child as `LINEAGE_URL` so
    /// notebook templates can wire OpenLineage. `None` when lineage isn't part of
    /// the environment.
    pub(crate) lineage_endpoint: Option<String>,
    /// Whether this environment serves a local `/home` volume. Surfaced to the
    /// UI as an environment capability.
    has_home: bool,
}

/// Managed state: the active environment behind interior mutability so the
/// `start_environment` command can swap services in after boot. Commands take a
/// snapshot (clone of the `Arc`s) under a short read lock, then drop it before
/// awaiting.
#[derive(Default)]
pub(crate) struct AppState {
    pub(crate) active: RwLock<ActiveEnv>,
}

impl AppState {
    /// Snapshot the active environment, erroring when none is selected yet.
    pub(crate) fn snapshot(&self) -> Result<ActiveEnv, String> {
        let active = self.active.read().unwrap();
        if active.hosted.is_none() {
            return Err("no environment selected".to_string());
        }
        Ok(active.clone())
    }

    /// Snapshot the active UC endpoint (the proxy needs only this).
    pub(crate) fn unity_endpoint(&self) -> Option<String> {
        self.active.read().unwrap().unity_endpoint.clone()
    }
}

impl ActiveEnv {
    /// Select the router that owns a service group: `"tags"` (portal Tags),
    /// `"query"` (hydrofoil QueryService), or `"ingest"` (hydrofoil IngestService).
    /// Files is not a router — it is served by the `files_*` commands directly.
    pub(crate) fn router(&self, service: &str) -> Result<&connectrpc::Router, String> {
        let hosted = self.hosted.as_ref().ok_or("no environment selected")?;
        match service {
            "tags" => Ok(&hosted.tags),
            "query" => Ok(&hosted.query),
            "ingest" => Ok(&hosted.ingest),
            other => Err(format!("unknown service group: {other}")),
        }
    }

    /// Clone the file-store handle so commands can drop the snapshot before
    /// awaiting the store call.
    pub(crate) fn files(&self) -> Result<Arc<dyn portal::store::FileStore>, String> {
        let hosted = self.hosted.as_ref().ok_or("no environment selected")?;
        Ok(Arc::clone(&hosted.files))
    }
}

/// One environment: a named bundle of service configuration. Carries an id +
/// display name (the UC config is derived from the id's directory), the baseline
/// catalog modules to run alongside UC (headwaters/mlflow/azurite — see
/// [`crate::topology::available_modules`]), and an observability opt-in. Modules map
/// directly to the shared topology catalog; observability is *not* a module — it's
/// an app-level opt-in that emits to the shared telemetry collector.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Environment {
    pub(crate) id: String,
    pub(crate) name: String,
    /// Selected catalog module ids (see [`crate::topology::available_modules`]).
    /// Empty = UC-only (no Docker). `#[serde(default)]` keeps module-less
    /// `environments.json` files parsing.
    #[serde(default)]
    pub(crate) modules: Vec<String>,
    /// Whether this environment opts in to emitting telemetry to the shared,
    /// app-level collector. Not a module: it brings up the shared collector, not a
    /// per-env service.
    #[serde(default)]
    pub(crate) observability: bool,
}

/// Read the environments registry (`environments.json`). Returns an empty list
/// when the file is absent (fresh install) so the shell shows the create flow.
pub(crate) fn read_environments() -> Result<Vec<Environment>, String> {
    let path = app_data_dir().join("environments.json");
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("parsing {path:?}: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("reading {path:?}: {e}")),
    }
}

/// Persist the environments registry, creating the app data dir if needed.
pub(crate) fn write_environments(envs: &[Environment]) -> Result<(), String> {
    let dir = app_data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {dir:?}: {e}"))?;
    let json = serde_json::to_vec_pretty(envs).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("environments.json"), json).map_err(|e| e.to_string())
}

/// Derive a stable, filesystem-safe id from a display name, disambiguating
/// against existing ids with a numeric suffix. Avoids needing a random/uuid
/// source: the suffix is deterministic from the current registry.
pub(crate) fn allocate_env_id(name: &str, existing: &[Environment]) -> String {
    let slug: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    let base = if slug.is_empty() {
        "env".to_string()
    } else {
        slug
    };
    let taken = |candidate: &str| existing.iter().any(|e| e.id == candidate);
    if !taken(&base) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Seed a fresh home volume with a starter `queries/` dir + a README so the editor
/// is never empty on first open. Idempotent: skips when the dir already has any
/// contents (so it never clobbers user files across restarts). Best-effort —
/// failures are logged, not fatal.
pub(crate) fn seed_home_dir(home: &std::path::Path) {
    let non_empty = std::fs::read_dir(home)
        .map(|mut d| d.next().is_some())
        .unwrap_or(false);
    if non_empty {
        return;
    }
    let write = |rel: &str, body: &str| {
        let path = home.join(rel);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&path, body) {
            eprintln!("[home] seed {path:?} failed: {e}");
        }
    };
    write(
        "queries/example.sql",
        "SELECT * FROM main.default.users\nORDER BY events DESC\nLIMIT 10;\n",
    );
    write(
        "README.md",
        "# Home\n\nLocal scratch space for SQL and notes.\n",
    );
}

/// Bring an environment online: build (and store) the in-process executors for
/// the given UC endpoint. `unity_endpoint` is `None` when UC is disabled (files
/// run in-memory). Called by `start_environment`.
pub(crate) async fn activate_endpoint(
    app: &tauri::AppHandle,
    id: Option<String>,
    unity_endpoint: Option<String>,
    lineage_endpoint: Option<String>,
) -> Result<(), String> {
    // An environment gets a local home volume under its data dir.
    let home_root = match id.as_deref() {
        Some(env_id) => {
            let home = env_home_dir(env_id);
            std::fs::create_dir_all(&home).map_err(|e| format!("creating {home:?}: {e}"))?;
            seed_home_dir(&home);
            Some(home)
        }
        None => None,
    };

    let has_home = home_root.is_some();
    let cfg = HostConfig {
        unity_endpoint: unity_endpoint.clone(),
        lineage_endpoint: lineage_endpoint.clone(),
        home_root,
        ..Default::default()
    };
    let hosted = desktop_host::build(cfg)
        .await
        .map_err(|e| format!("failed to build in-process services: {e}"))?;

    let state = app.state::<AppState>();
    let mut active = state.active.write().unwrap();
    *active = ActiveEnv {
        id,
        hosted: Some(Arc::new(hosted)),
        unity_endpoint,
        lineage_endpoint,
        has_home,
    };
    Ok(())
}

/// Build the `ActiveEnvironment` descriptor the UI consumes (see
/// node/ui/src/lib/client/environments.ts): id, display name, and capabilities.
/// The UI derives built-in volumes from `hasHome`. Returns `null` when nothing is
/// active. `name` falls back to the id if the env has no registry entry.
pub(crate) fn active_environment_descriptor(state: &AppState) -> Option<serde_json::Value> {
    let active = state.active.read().unwrap();
    let id = active.id.clone()?;
    let name = read_environments()
        .ok()
        .and_then(|envs| envs.into_iter().find(|e| e.id == id).map(|e| e.name))
        .unwrap_or_else(|| id.clone());
    Some(serde_json::json!({
        "id": id,
        "name": name,
        "capabilities": { "hasHome": active.has_home },
    }))
}
