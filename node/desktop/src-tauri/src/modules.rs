//! Desktop-side orchestration of an environment's service modules.
//!
//! The pure resolution + artifact generation lives in the `env-modules` crate;
//! this module owns the side effects: resolving paths into a `LaunchContext`,
//! the Docker daemon preflight, writing the generated compose file, running
//! `docker compose up`/`down`, and spawning uvx sidecars. Everything started here
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
fn launch_context(uc_port: Option<u16>) -> LaunchContext {
    LaunchContext {
        uc_port,
        fragments_dir: fragments_dir().to_string_lossy().into_owned(),
        environments_dir: environments_dir().to_string_lossy().into_owned(),
    }
}

/// Start an environment's service modules after UC is up. `uc_port` is the
/// sidecar's bound port (for outward UC injection). Tracks every started process
/// in the supervisor. Returns an error (after best-effort cleanup) if a required
/// Docker daemon is absent or `docker compose up` fails — so a failed module
/// start surfaces to the user rather than leaving a half-running environment.
pub async fn start_modules(
    app: &tauri::AppHandle,
    env_id: &str,
    modules: &[String],
    uc_port: Option<u16>,
    supervisor: &Supervisor,
) -> Result<(), String> {
    let graph = env_modules::resolve(modules).map_err(|e| format!("resolving modules: {e}"))?;

    if graph.needs_docker() {
        if !docker_available() {
            return Err(
                "Docker is required for the selected services but the Docker daemon \
                 is not running. Start Docker and try again."
                    .into(),
            );
        }
        start_compose(env_id, &graph, uc_port, supervisor)?;
    }

    start_uvx_sidecars(app, &graph, uc_port, supervisor).await?;
    Ok(())
}

/// Generate + write the compose file and bring the project up, tracking it for
/// teardown. Reconciles any stale project from a prior crash first.
fn start_compose(
    env_id: &str,
    graph: &ResolvedGraph,
    uc_port: Option<u16>,
    supervisor: &Supervisor,
) -> Result<(), String> {
    let project = compose_project(env_id);
    // Reconcile a stale project from a prior force-quit before bringing ours up.
    compose_down(&project);

    let ctx = launch_context(uc_port);
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

/// Spawn each uvx sidecar module (e.g. marimo) as a tracked Tauri child, with the
/// host-local UC URI injected.
///
/// Phase 5 wires the concrete marimo launch recipe (`marimo edit --headless …`)
/// and the matching `shell:allow-spawn` scope entry. Until then, selecting a uvx
/// module fails fast rather than spawning a command the shell scope would reject
/// at runtime — keeping the Docker path fully functional and the error honest.
async fn start_uvx_sidecars(
    _app: &tauri::AppHandle,
    graph: &ResolvedGraph,
    _uc_port: Option<u16>,
    _supervisor: &Supervisor,
) -> Result<(), String> {
    if let Some(module) = graph.uvx_modules().first() {
        return Err(format!(
            "service module '{}' (uvx sidecar) is not yet supported",
            module.id
        ));
    }
    Ok(())
}
