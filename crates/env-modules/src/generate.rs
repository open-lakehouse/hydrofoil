//! Artifact generation: a [`ResolvedGraph`] → a runnable Docker Compose file.
//!
//! Kept separate from resolution ([`crate::resolve`]) so the analysis stays pure
//! and reusable. Generation is *also* pure: it takes a graph and a
//! [`LaunchContext`] (the host-side facts — the UC port, paths) and returns
//! string/struct artifacts. The desktop crate owns the side effects (writing the
//! file, running `docker compose`, spawning uvx).
//!
//! The generated compose file `include:`s the self-contained desktop fragments
//! under `environments/services/desktop/` — written specifically for this stack,
//! so there are no cross-stack defaults, profiles, or contradictory credentials
//! to override. The single boundary value the generator supplies is the **UC host
//! URL**, injected outward (Docker→UC) for any module that consumes it.

use std::collections::BTreeMap;

use crate::resolve::ResolvedGraph;

/// Host-side facts needed to turn a graph into runnable artifacts. The desktop
/// crate fills this in at start time (the UC sidecar's resolved port; the
/// resolved `environments/` paths).
#[derive(Clone, Debug)]
pub struct LaunchContext {
    /// The Unity Catalog host port the sidecar bound to. Injected outward so
    /// Docker services reach UC at `host.docker.internal:<uc_port>` and uvx
    /// sidecars at `localhost:<uc_port>`. `None` when UC is disabled.
    pub uc_port: Option<u16>,
    /// Absolute path to `environments/services/desktop/`, where the self-contained
    /// desktop fragments live. The generated compose `include:`s fragments here.
    pub fragments_dir: String,
    /// Absolute path to `environments/` — the `project_directory` the included
    /// fragments resolve their relative `./config`, `./docker` (read-only,
    /// repo-tracked) paths against.
    pub environments_dir: String,
    /// Absolute path to this environment's writable data root
    /// (`.open-lakehouse/envs/<id>/modules/data`), injected as `OL_ENV_DATA_DIR`
    /// so stateful fragments (Postgres, Azurite) bind their data dirs **per
    /// environment** and co-located with the rest of the env's state — so it
    /// survives restarts and the whole environment can be deleted as a unit.
    pub env_data_dir: String,
    /// The shared telemetry collector's OTLP/HTTP base URL (the host Jaeger, e.g.
    /// `http://host.docker.internal:4318`), set when this environment opted in to
    /// observability. Injected as `OTEL_COLLECTOR_HTTP` so the service fragments'
    /// OpenTelemetry exporters emit to it; `None` leaves it empty and the bundled
    /// instrumentation is a no-op.
    pub otel_collector_http: Option<String>,
}

/// The generated, runnable artifacts for an environment's Docker modules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposeArtifacts {
    /// The generated top-level compose file contents (`include:`s the desktop
    /// fragments). Empty when the graph has no Docker modules.
    pub compose_yaml: String,
    /// Environment variables to pass to `docker compose` (the values the fragments
    /// interpolate, including the UC host URL). Empty when no Docker modules.
    pub env: BTreeMap<String, String>,
}

/// The UC URL injected into Docker services (reached across the host boundary).
fn docker_uc_url(uc_port: u16) -> String {
    format!("http://host.docker.internal:{uc_port}/api/2.1/unity-catalog/")
}

/// Generate the Docker Compose artifacts for a graph. Returns empty artifacts
/// when the graph has no Docker modules.
pub fn generate_compose(graph: &ResolvedGraph, ctx: &LaunchContext) -> ComposeArtifacts {
    let docker = graph.docker_modules();
    if docker.is_empty() {
        return ComposeArtifacts {
            compose_yaml: String::new(),
            env: BTreeMap::new(),
        };
    }

    // The one boundary value: the UC host URL, injected outward. The desktop
    // fragments otherwise carry their own (desktop-specific) configuration, so
    // there is nothing else to override here.
    let mut env = BTreeMap::new();
    if let Some(port) = ctx.uc_port {
        env.insert("UC_HOST_URL".to_string(), docker_uc_url(port));
    }
    // The per-environment writable data root. Stateful fragments bind their data
    // dirs under this (e.g. `${OL_ENV_DATA_DIR}/db`), so state is isolated per
    // environment and persists across restarts.
    env.insert("OL_ENV_DATA_DIR".to_string(), ctx.env_data_dir.clone());
    // When the env opted in to observability, point the service fragments' OTLP
    // exporters at the shared host collector; otherwise leave it unset so the
    // baked-in instrumentation is a no-op.
    if let Some(collector) = &ctx.otel_collector_http {
        env.insert("OTEL_COLLECTOR_HTTP".to_string(), collector.clone());
    }

    // Generated top-level file: just `include:`s each module's self-contained
    // desktop fragment, all sharing the `environments/` project directory so
    // their relative paths resolve. Hand-built YAML (a tiny, fixed shape) rather
    // than pulling in a YAML serializer dependency.
    let mut yaml = String::from("# Generated by env-modules — do not edit.\n");
    yaml.push_str("name: ${COMPOSE_PROJECT_NAME}\n\n");
    yaml.push_str("include:\n");
    for module in &docker {
        // `docker_modules()` already filtered to Docker services; destructure to
        // get the fragment. `ModuleKind` has one variant today (hence the allow);
        // `let-else` keeps this correct if more kinds are added later.
        #[allow(irrefutable_let_patterns)]
        let crate::model::ModuleKind::DockerService { fragment } = &module.kind else {
            continue;
        };
        yaml.push_str(&format!("  - path: {}/{}\n", ctx.fragments_dir, fragment));
        yaml.push_str(&format!(
            "    project_directory: {}\n",
            ctx.environments_dir
        ));
    }

    ComposeArtifacts {
        compose_yaml: yaml,
        env,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::resolve;

    fn ctx() -> LaunchContext {
        LaunchContext {
            uc_port: Some(54321),
            fragments_dir: "/repo/environments/services/desktop".into(),
            environments_dir: "/repo/environments".into(),
            env_data_dir: "/data/.open-lakehouse/envs/dev/modules/data".into(),
            otel_collector_http: None,
        }
    }

    #[test]
    fn empty_graph_generates_no_compose() {
        let graph = resolve(&[]).unwrap();
        let artifacts = generate_compose(&graph, &ctx());
        assert!(artifacts.compose_yaml.is_empty());
        assert!(artifacts.env.is_empty());
    }

    #[test]
    fn mlflow_includes_fragments_and_injects_uc_url() {
        let graph = resolve(&["mlflow".into()]).unwrap();
        let artifacts = generate_compose(&graph, &ctx());
        // Includes each docker fragment from the desktop dir.
        for fragment in ["mlflow.yaml", "postgres.yaml", "azurite.yaml", "envoy.yaml"] {
            assert!(
                artifacts
                    .compose_yaml
                    .contains(&format!("services/desktop/{fragment}")),
                "expected include of desktop/{fragment} in:\n{}",
                artifacts.compose_yaml
            );
        }
        // All includes share the environments/ project directory.
        assert!(
            artifacts
                .compose_yaml
                .contains("project_directory: /repo/environments\n")
        );
        // Injects the UC host URL pointing at host.docker.internal.
        assert_eq!(
            artifacts.env.get("UC_HOST_URL").map(String::as_str),
            Some("http://host.docker.internal:54321/api/2.1/unity-catalog/")
        );
        // Injects the per-env data root for stateful fragments to bind against.
        assert_eq!(
            artifacts.env.get("OL_ENV_DATA_DIR").map(String::as_str),
            Some("/data/.open-lakehouse/envs/dev/modules/data")
        );
        // No collector by default → no OTEL_COLLECTOR_HTTP (instrumentation no-op).
        assert!(!artifacts.env.contains_key("OTEL_COLLECTOR_HTTP"));
    }

    #[test]
    fn collector_injected_when_observability_opted_in() {
        let graph = resolve(&["mlflow".into()]).unwrap();
        let mut c = ctx();
        c.otel_collector_http = Some("http://host.docker.internal:4318".into());
        let artifacts = generate_compose(&graph, &c);
        assert_eq!(
            artifacts.env.get("OTEL_COLLECTOR_HTTP").map(String::as_str),
            Some("http://host.docker.internal:4318")
        );
    }
}
