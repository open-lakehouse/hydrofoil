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

