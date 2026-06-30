//! Desktop-side orchestration of an environment's service modules.
//!
//! The pure topology model — the catalog, the plan, and the rendered compose
//! artifacts — lives in `olai-stack-topology` (consumed via `env_modules::topology`).
//! This module owns the side effects: persisting the environment manifest, writing the
//! rendered project tree, the Docker daemon preflight, running `docker compose
//! up`/`down`, and resolving the in-process engine's service URLs from the plan.
//! Everything started here is tracked in the [`Supervisor`] so it is torn down with the
//! environment.

use std::path::PathBuf;

use env_modules::topology::{self, Manifest, Plan, ServiceRole};

use crate::supervisor::{ManagedProcess, Supervisor, compose_down};

/// Whether the Docker daemon is reachable (`docker info` succeeds). Drives both
/// the start-time preflight and the UI availability banner.
pub fn docker_available() -> bool {
    std::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The deterministic Compose project name for an environment. Deterministic so a
/// stale project from a prior force-quit is reconciled (torn down) on next start.
pub fn compose_project(env_id: &str) -> String {
    format!("ol-{env_id}")
}

/// The directory holding an environment's rendered topology project
/// (`.open-lakehouse/envs/<id>/modules/`): the top-level `compose.yaml`, `.env`, the
/// Envoy bootstrap, and each module's fragment + config files.
fn env_modules_dir(env_id: &str) -> PathBuf {
    crate::app_data_dir()
        .join("envs")
        .join(env_id)
        .join("modules")
}

/// The persisted environment manifest path (`.open-lakehouse/envs/<id>/env.toml`):
/// the re-plannable record of the environment's selection + context.
fn env_manifest_path(env_id: &str) -> PathBuf {
    crate::app_data_dir()
        .join("envs")
        .join(env_id)
        .join("env.toml")
}

/// The data root for an environment's stateful services, injected into the rendered
/// compose as `DATA_ROOT`. Per-environment so service state is isolated and co-located
/// with the env's UC/home/notebook state — persists across restarts, removed with the
/// environment. The topology templates mount `${DATA_ROOT}/<module>` by convention.
fn env_data_dir(env_id: &str) -> PathBuf {
    env_modules_dir(env_id).join("data")
}

/// The Envoy gateway's host-published port. Envoy publishes on `ENVOY_PORT`
/// (default 9080); this is also the plan's `gateway_host_port`, so every in-process →
/// gatewayed-service URL the plan resolves agrees with where Envoy actually binds.
fn gateway_host_port() -> u16 {
    std::env::var("ENVOY_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9080)
}

/// Build the environment manifest from the selected capabilities and host facts.
/// The manifest is the single source of truth for the plan; persisting it makes the
/// environment reproducible and editable.
fn manifest(env_id: &str, capabilities: &[env_modules::Capability]) -> Manifest {
    topology::manifest(
        capabilities,
        compose_project(env_id),
        env_data_dir(env_id).to_string_lossy().into_owned(),
        gateway_host_port(),
    )
}

/// Start an environment's service modules after UC is up. Resolves the topology plan
/// from the (persisted) manifest, renders + writes the compose project, and brings it
/// up. Returns the resolved [`Plan`] so the caller can resolve in-process service URLs
/// (e.g. the lineage sink) via [`lineage_endpoint`]. Tracks every started process in
/// the supervisor. Returns an error (after best-effort cleanup) if a required Docker
/// daemon is absent or `docker compose up` fails — so a failed module start surfaces to
/// the user rather than leaving a half-running env.
///
/// `observability` is intentionally not threaded here: the topology templates carry
/// their own OTLP configuration, and the shared collector is an app-level concern wired
/// outside the compose project.
pub fn start_modules(
    env_id: &str,
    capabilities: &[env_modules::Capability],
    supervisor: &Supervisor,
) -> Result<Plan, String> {
    let manifest = manifest(env_id, capabilities);
    let plan = manifest
        .plan(&topology::catalog())
        .map_err(|e| format!("planning environment topology: {e}"))?;

    // No containerized modules (e.g. observability-only) → nothing to bring up.
    let needs_docker = !plan.graph.nodes.is_empty();
    if needs_docker {
        if !docker_available() {
            return Err(
                "Docker is required for the selected services but the Docker daemon \
                 is not running. Start Docker and try again."
                    .into(),
            );
        }
        start_compose(env_id, &manifest, &plan, supervisor)?;
    }

    Ok(plan)
}

/// The lineage sink endpoint to configure the in-process engine with, resolved from the
/// plan: `Some(url)` when the environment runs a `lineage`-role service (headwaters),
/// reached from the host through the Envoy gateway; `None` when lineage isn't part of
/// the environment. The in-process engine sits on the host, so it addresses the
/// gatewayed service at the host vantage.
pub fn lineage_endpoint(plan: &Plan) -> Option<String> {
    let lineage = plan.service_by_role(&ServiceRole::lineage()).ok()?;
    lineage
        .address(topology::IN_PROCESS_VANTAGE, topology::LINEAGE_ENDPOINT_ID)
        .ok()
        .map(|url| url.to_string())
}

/// Render + write the compose project, persist the manifest, and bring the project up,
/// tracking it for teardown. Reconciles any stale project from a prior crash first.
fn start_compose(
    env_id: &str,
    manifest: &Manifest,
    plan: &Plan,
    supervisor: &Supervisor,
) -> Result<(), String> {
    let project = compose_project(env_id);
    // Reconcile a stale project from a prior force-quit before bringing ours up.
    compose_down(&project);

    let dir = env_modules_dir(env_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {dir:?}: {e}"))?;
    // Create the per-env data root up front so the bind mounts resolve to a host dir we
    // own (Docker would otherwise create it root-owned on first up).
    let data_dir = env_data_dir(env_id);
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("creating {data_dir:?}: {e}"))?;

    // Persist the manifest so the environment is reproducible and editable across
    // restarts (re-plan from it rather than recompute from capabilities).
    manifest
        .write_to(&env_manifest_path(env_id))
        .map_err(|e| format!("writing environment manifest: {e}"))?;

    // Render the full project tree (top-level compose, .env, Envoy bootstrap, per-module
    // fragments + config files) and write it under the env's modules dir.
    plan.materialize()
        .write_to(&dir)
        .map_err(|e| format!("writing rendered compose project to {dir:?}: {e}"))?;

    // Track BEFORE `up` so a partial/failed bring-up is still cleaned up on the next
    // stop / app exit (compose down is idempotent).
    supervisor.track(ManagedProcess::Compose {
        project: project.clone(),
    });

    let compose_path = dir.join("compose.yaml");
    // The rendered project is self-contained: `.env` sits next to compose.yaml and is
    // auto-loaded, so no env injection is needed here.
    let output = std::process::Command::new("docker")
        .args([
            "compose",
            "-p",
            &project,
            "-f",
            &compose_path.to_string_lossy(),
            "up",
            "-d",
            "--wait",
        ])
        .env("COMPOSE_PROJECT_NAME", &project)
        .output()
        .map_err(|e| format!("running docker compose up: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "docker compose up failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// Live status of one container in an environment's compose project.
#[derive(serde::Serialize)]
pub struct ServiceStatus {
    /// The compose service name (e.g. `mlflow`, `db`, `jaeger`).
    pub service: String,
    /// Compose state: `running`, `exited`, `restarting`, etc.
    pub state: String,
    /// Health when the service declares a healthcheck: `healthy`, `starting`,
    /// `unhealthy`, or empty when it has none.
    pub health: String,
    /// Whether this is the shared, app-level telemetry collector (rendered as
    /// "shared" in the UI) rather than a per-environment service.
    pub shared: bool,
}

/// Live per-service status for a running environment, for the UI's Services panel.
/// Reads `docker compose ps` for the env's project plus the shared telemetry
/// project. Best-effort: returns an empty list if Docker is unavailable or the
/// projects aren't up (the UI then shows nothing rather than erroring).
pub fn service_status(env_id: &str) -> Vec<ServiceStatus> {
    let mut all = compose_ps(&compose_project(env_id), false);
    // The shared collector is its own app-level project; surface it too so the
    // user sees the full picture of what their environment talks to.
    all.extend(compose_ps(crate::telemetry::TELEMETRY_PROJECT, true));
    all
}

/// Whether the shared app-level telemetry collector (Jaeger) is currently up.
/// Drives the Telemetry entry's status dot and its embedded UI gate, independent
/// of any environment.
pub fn telemetry_running() -> bool {
    compose_ps(crate::telemetry::TELEMETRY_PROJECT, true)
        .iter()
        .any(|s| s.state == "running")
}

/// Run `docker compose -p <project> ps --format json` and parse per-service
/// status. Compose emits either a JSON array or newline-delimited JSON objects
/// depending on version; handle both. Best-effort — any failure yields an empty
/// list.
fn compose_ps(project: &str, shared: bool) -> Vec<ServiceStatus> {
    let Ok(output) = std::process::Command::new("docker")
        .args(["compose", "-p", project, "ps", "--format", "json", "--all"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);

    // Each line is a JSON object (newline-delimited); some compose versions emit
    // a single array. Parse line-by-line, falling back to array parsing.
    let mut rows: Vec<serde_json::Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if rows.is_empty() {
        if let Ok(serde_json::Value::Array(arr)) = serde_json::from_str(text.trim()) {
            rows = arr;
        }
    }

    rows.into_iter()
        .filter_map(|row| {
            let service = row.get("Service")?.as_str()?.to_string();
            let state = row
                .get("State")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // Compose reports health inside the `Health` field (or empty).
            let health = row
                .get("Health")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(ServiceStatus {
                service,
                state,
                health,
                shared,
            })
        })
        .collect()
}

/// A read-only config artifact for the teaching/inspection viewer.
#[derive(serde::Serialize)]
pub struct ConfigArtifact {
    /// Stable id (used as the Monaco model key / picker value).
    pub id: String,
    /// Human-readable label for the picker.
    pub label: String,
    /// Short one-line description of what the artifact is / does.
    pub description: String,
    /// Editor language id (`yaml` for everything we surface today).
    pub language: String,
    /// The file contents (generated or read from disk).
    pub content: String,
}

/// Build the curated list of config artifacts for an environment's selected
/// capabilities — for the read-only viewer. The artifacts are the **fully rendered**
/// topology project (top-level compose, `.env`, the `LAYOUT.md` gateway summary, the
/// Envoy bootstrap, and each module's fragment + config files), produced on demand from
/// the plan with no side effects — so it shows the real, exact shape the environment
/// will run, viewable before it has ever started.
pub fn config_artifacts(
    capabilities: &[env_modules::Capability],
) -> Result<Vec<ConfigArtifact>, String> {
    // Illustrative facts: the viewer renders pre-start, so use a placeholder env id for
    // the project name / data-root path shown in the rendered artifacts.
    let manifest = topology::manifest(
        capabilities,
        "<env>",
        env_data_dir("<env>").to_string_lossy().into_owned(),
        gateway_host_port(),
    );
    let plan = manifest
        .plan(&topology::catalog())
        .map_err(|e| format!("planning environment topology: {e}"))?;

    let artifacts = plan
        .materialize()
        .files
        .into_iter()
        .map(|file| {
            let language = artifact_language(&file.path);
            ConfigArtifact {
                id: file.path.clone(),
                label: artifact_label(&file.path),
                description: artifact_description(&file.path),
                language,
                content: file.contents,
            }
        })
        .collect();

    Ok(artifacts)
}

/// The Monaco editor language id for a rendered artifact path.
fn artifact_language(path: &str) -> String {
    match path.rsplit('.').next() {
        Some("toml") => "toml",
        Some("md") => "markdown",
        Some("env") => "ini",
        // compose.yaml, envoy.yaml, and the `.env` top-level file default to yaml/ini;
        // the top-level `.env` has no extension after the dot, handled above.
        _ if path == ".env" => "ini",
        _ => "yaml",
    }
    .to_string()
}

/// A human-readable picker label for a rendered artifact path.
fn artifact_label(path: &str) -> String {
    match path {
        "compose.yaml" => "Top-level compose".to_string(),
        ".env" => "Environment (.env)".to_string(),
        "LAYOUT.md" => "Gateway layout (LAYOUT.md)".to_string(),
        "modules/envoy/envoy.yaml" => "Envoy gateway (envoy.yaml)".to_string(),
        // `modules/<id>/<file>` — label by module + file.
        _ => path
            .strip_prefix("modules/")
            .map(|rest| rest.replace('/', " / "))
            .unwrap_or_else(|| path.to_string()),
    }
}

/// A one-line description for a rendered artifact path.
fn artifact_description(path: &str) -> String {
    match path {
        "compose.yaml" => {
            "The top-level Docker Compose file: includes each module's fragment.".to_string()
        }
        ".env" => "Plan-injected environment values the compose file substitutes.".to_string(),
        "LAYOUT.md" => "Human-readable summary of the gateway routes → services.".to_string(),
        "modules/envoy/envoy.yaml" => {
            "The gateway bootstrap: routes to the containerised services.".to_string()
        }
        _ => format!("Rendered artifact at {path}."),
    }
}

