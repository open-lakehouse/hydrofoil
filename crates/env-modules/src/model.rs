//! The service-module model and the static registry.
//!
//! A *module* is one selectable capability a desktop environment can run
//! alongside its Unity Catalog server (e.g. MLflow, Marquez). Modules
//! declare their dependencies on other modules; the resolver ([`crate::resolve`])
//! closes over those edges. The registry here is the single source of truth for
//! what modules exist and how they relate — it mirrors the `depends_on:` edges in
//! `environments/services/*.yaml`, encoded once so both the resolver and (later)
//! the artifact generator and a graph visualization can reuse it.

use serde::{Deserialize, Serialize};

/// A stable module identifier, matching the service/fragment name used in
/// `environments/services/*.yaml` (e.g. `"mlflow"`, `"marquez"`, `"postgres"`).
pub type ModuleId = String;

/// How a module is launched at runtime. The resolver is agnostic to this; it's
/// carried through so the supervisor/generator can dispatch correctly and so the
/// UI can tell Docker-backed modules (which need the daemon) from native ones.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ModuleKind {
    /// A Docker Compose service from a self-contained desktop fragment under
    /// `environments/services/desktop/` (filename relative to that dir). The
    /// fragments are written specifically for the desktop stack — no cross-stack
    /// defaults, profiles, or contradictory creds to override — so the generator
    /// just `include:`s them and injects the UC host URL where needed.
    ///
    /// (Native host sidecars — e.g. marimo via `uvx` — are intentionally not
    /// modelled yet; that launch path is future work.)
    DockerService { fragment: String },
}

impl ModuleKind {
    /// Whether this module requires a running Docker daemon. Every module is
    /// Docker-backed today; this gates the UI's graceful degrade when the daemon
    /// is absent.
    pub fn needs_docker(&self) -> bool {
        matches!(self, ModuleKind::DockerService { .. })
    }
}

/// One service module: an identity, how it launches, and which other modules it
/// requires (its direct dependency edges).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    pub id: ModuleId,
    /// Human-readable label for the UI.
    pub name: String,
    pub kind: ModuleKind,
    /// Modules this one directly depends on. The resolver pulls these in
    /// transitively when the module is selected.
    #[serde(default)]
    pub requires: Vec<ModuleId>,
}

/// The static module registry. The single source of truth for available modules
/// and their dependency edges, mirroring `environments/services/*.yaml`.
///
/// Topology reminder: in the desktop context Unity Catalog runs on the host (not
/// in Docker), and dependencies flow one direction only — Docker→UC. UC is
/// therefore NOT a module here; it's always-present infrastructure that modules
/// consume via an injected host URL. The `envoy` gateway IS a module:
/// it's the single published entry point the app reaches Docker services through,
/// so it is pulled in whenever any Docker service is selected (see the registry
/// note below — every Docker service requires `envoy`).
pub fn registry() -> Vec<Module> {
    vec![
        // Shared Postgres backing MLflow + Marquez (own DBs/roles created by
        // docker/db/init/01-create-databases.sql). No dependencies.
        Module {
            id: "postgres".into(),
            name: "PostgreSQL".into(),
            kind: ModuleKind::DockerService {
                fragment: "postgres.yaml".into(),
            },
            requires: vec![],
        },
        // Azure Blob emulator (UC credential-vending target + MLflow artifacts).
        Module {
            id: "azurite".into(),
            name: "Azurite (Azure Blob)".into(),
            kind: ModuleKind::DockerService {
                fragment: "azurite.yaml".into(),
            },
            requires: vec![],
        },
        // Envoy gateway: the single host-published entry point for Docker
        // services. No deps of its own; every Docker service depends on it so it
        // is present whenever the app needs to reach a containerised service.
        Module {
            id: "envoy".into(),
            name: "Envoy gateway".into(),
            kind: ModuleKind::DockerService {
                fragment: "envoy.yaml".into(),
            },
            requires: vec![],
        },
        // MLflow: Postgres backend + Azurite artifacts, reached via Envoy. The
        // desktop fragment is self-contained (Azurite-backed, no AWS_* creds).
        Module {
            id: "mlflow".into(),
            name: "MLflow".into(),
            kind: ModuleKind::DockerService {
                fragment: "mlflow.yaml".into(),
            },
            requires: vec!["postgres".into(), "azurite".into(), "envoy".into()],
        },
        // Marquez (Java backend + web UI): Postgres backend, reached via Envoy.
        // The desktop fragment is profile-free (no compose profile to activate).
        Module {
            id: "marquez".into(),
            name: "Marquez (lineage)".into(),
            kind: ModuleKind::DockerService {
                fragment: "marquez.yaml".into(),
            },
            requires: vec!["postgres".into(), "envoy".into()],
        },
    ]
}
