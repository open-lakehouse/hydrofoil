//! Desktop-side orchestration of an environment's service modules.
//!
//! The pure resolution + artifact generation lives in the `env-modules` crate;
//! this module owns the side effects: resolving paths into a `LaunchContext`,
//! the Docker daemon preflight, writing the generated compose file, running
//! `docker compose up`/`down`. Everything started here
//! is tracked in the [`Supervisor`] so it is torn down with the environment.

use std::path::{Path, PathBuf};

use env_modules::{ComposeArtifacts, LaunchContext, ResolvedGraph};

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

/// The directory holding an environment's generated module artifacts
/// (`.open-lakehouse/envs/<id>/modules/`).
fn env_modules_dir(env_id: &str) -> PathBuf {
    crate::app_data_dir()
        .join("envs")
        .join(env_id)
        .join("modules")
}

/// Absolute path to the `environments/` directory (sibling of `node/`), the
/// project directory the included fragments resolve relative paths against.
fn environments_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../environments")
}

/// Absolute path to the self-contained desktop fragments directory.
fn fragments_dir() -> PathBuf {
    environments_dir().join("services/desktop")
}

/// Build the `LaunchContext` from the resolved UC port and host facts.
/// `observability` opts the env's services in to emitting to the shared host
/// Jaeger (reached from containers via host.docker.internal on its OTLP/HTTP port).
fn launch_context(uc_port: Option<u16>, observability: bool) -> LaunchContext {
    LaunchContext {
        uc_port,
        fragments_dir: fragments_dir().to_string_lossy().into_owned(),
        environments_dir: environments_dir().to_string_lossy().into_owned(),
        otel_collector_http: observability.then(|| {
            let port = std::env::var("JAEGER_OTLP_HTTP_PORT").unwrap_or_else(|_| "4318".into());
            format!("http://host.docker.internal:{port}")
        }),
    }
}

/// Start an environment's service modules after UC is up. `uc_port` is the
/// sidecar's bound port (for outward UC injection); `observability` opts the
/// services in to emitting traces to the shared collector. Tracks every started
/// process in the supervisor. Returns an error (after best-effort cleanup) if a
/// required Docker daemon is absent or `docker compose up` fails — so a failed
/// module start surfaces to the user rather than leaving a half-running env.
pub fn start_modules(
    env_id: &str,
    capabilities: &[env_modules::Capability],
    uc_port: Option<u16>,
    supervisor: &Supervisor,
) -> Result<ResolvedGraph, String> {
    let graph = env_modules::resolve_capabilities(capabilities)
        .map_err(|e| format!("resolving capabilities: {e}"))?;
    let observability = env_modules::Capability::wants_observability(capabilities);

    if graph.needs_docker() {
        if !docker_available() {
            return Err(
                "Docker is required for the selected services but the Docker daemon \
                 is not running. Start Docker and try again."
                    .into(),
            );
        }
        start_compose(env_id, &graph, uc_port, observability, supervisor)?;
    }

    Ok(graph)
}

/// The lineage sink endpoint to inject into the in-process engine, derived from
/// the resolved graph: `Some(base_url)` when the graph carries a lineage effect
/// (the Marquez sink is reached through the Envoy gateway on the host). `None`
/// when lineage isn't part of the environment.
pub fn lineage_endpoint(graph: &ResolvedGraph) -> Option<String> {
    graph
        .effect(env_modules::EffectKind::LineageEndpoint)
        .map(|_| gateway_base())
}

/// The host base URL of the Envoy gateway the Docker services are published on.
fn gateway_base() -> String {
    // Envoy publishes on ENVOY_PORT (default 9080); the desktop fragments use the
    // same default. Marquez's OpenLineage API lives under /api/v1 on the gateway.
    let port = std::env::var("ENVOY_PORT").unwrap_or_else(|_| "9080".to_string());
    format!("http://localhost:{port}")
}

/// Generate + write the compose file and bring the project up, tracking it for
/// teardown. Reconciles any stale project from a prior crash first.
fn start_compose(
    env_id: &str,
    graph: &ResolvedGraph,
    uc_port: Option<u16>,
    observability: bool,
    supervisor: &Supervisor,
) -> Result<(), String> {
    let project = compose_project(env_id);
    // Reconcile a stale project from a prior force-quit before bringing ours up.
    compose_down(&project);

    let ctx = launch_context(uc_port, observability);
    let ComposeArtifacts { compose_yaml, env } = env_modules::generate_compose(graph, &ctx);

    let dir = env_modules_dir(env_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {dir:?}: {e}"))?;
    let compose_path = dir.join("compose.yaml");
    std::fs::write(&compose_path, &compose_yaml)
        .map_err(|e| format!("writing {compose_path:?}: {e}"))?;

    // Track BEFORE `up` so a partial/failed bring-up is still cleaned up on the
    // next stop / app exit (compose down is idempotent).
    supervisor.track(ManagedProcess::Compose {
        project: project.clone(),
    });

    let mut cmd = std::process::Command::new("docker");
    cmd.args([
        "compose",
        "-p",
        &project,
        "-f",
        &compose_path.to_string_lossy(),
        "up",
        "-d",
        "--wait",
    ])
    .env("COMPOSE_PROJECT_NAME", &project);
    for (k, v) in &env {
        cmd.env(k, v);
    }

    let output = cmd
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
/// capabilities — for the read-only viewer. The generated compose is produced
/// **on demand** (illustrative `LaunchContext`, no side effects), so it's
/// viewable before the environment has ever started; the static fragments + the
/// gateway/collector configs are read from the repo `environments/` tree. All
/// file reads are confined to that tree (no arbitrary paths cross from the UI).
pub fn config_artifacts(
    capabilities: &[env_modules::Capability],
) -> Result<Vec<ConfigArtifact>, String> {
    let graph = env_modules::resolve_capabilities(capabilities)
        .map_err(|e| format!("resolving capabilities: {e}"))?;

    let mut artifacts = Vec::new();

    // 1. The generated compose for this capability set — the centerpiece. Use an
    //    illustrative context (placeholder UC port; collector shown when
    //    observability is opted in) so it renders the real shape pre-start.
    let observability = env_modules::Capability::wants_observability(capabilities);
    let ctx = launch_context(None, observability);
    let ComposeArtifacts { compose_yaml, .. } = env_modules::generate_compose(&graph, &ctx);
    if !compose_yaml.is_empty() {
        artifacts.push(ConfigArtifact {
            id: "generated-compose".into(),
            label: "Generated compose".into(),
            description: "The Docker Compose file env-modules generates for the \
                          selected capabilities (services + their dependencies)."
                .into(),
            language: "yaml".into(),
            content: compose_yaml,
        });
    }

    // 2. Each Docker module's self-contained fragment, in startup order.
    let fragments = fragments_dir();
    for module in graph.docker_modules() {
        #[allow(irrefutable_let_patterns)]
        let env_modules::ModuleKind::DockerService { fragment } = &module.kind
        else {
            continue;
        };
        let path = fragments.join(fragment);
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {path:?}: {e}"))?;
        artifacts.push(ConfigArtifact {
            id: format!("fragment-{}", module.id),
            label: format!("{} ({fragment})", module.name),
            description: format!("Service fragment for the {} module.", module.name),
            language: "yaml".into(),
            content,
        });
    }

    // 3. The desktop Envoy gateway config (present whenever the gateway runs).
    if graph.docker_modules().iter().any(|m| m.id == "envoy") {
        let path = environments_dir().join("config/tauri/envoy.yaml");
        if let Ok(content) = std::fs::read_to_string(&path) {
            artifacts.push(ConfigArtifact {
                id: "envoy-config".into(),
                label: "Envoy gateway (envoy.yaml)".into(),
                description: "The gateway config: routes to the containerised \
                              services and (when observability is on) exports its \
                              own traces."
                    .into(),
                language: "yaml".into(),
                content,
            });
        }
    }

    // 4. The shared telemetry collector fragment, when observability is opted in.
    if observability {
        let path = fragments.join("jaeger.yaml");
        if let Ok(content) = std::fs::read_to_string(&path) {
            artifacts.push(ConfigArtifact {
                id: "jaeger".into(),
                label: "Shared collector (jaeger.yaml)".into(),
                description: "The app-level Jaeger collector all environments' \
                              traces are sent to."
                    .into(),
                language: "yaml".into(),
                content,
            });
        }
    }

    Ok(artifacts)
}

