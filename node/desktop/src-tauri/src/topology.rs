//! Hydrofoil's policy over the shared [`olai-stack-topology`](olai_stack_topology)
//! model.
//!
//! The topology crate speaks [`Selection`] + [`PlanCtx`] against a [`Catalog`];
//! this module wires hydrofoil's fixed choices on top: which baseline modules are
//! user-selectable, the headwaters API-only knob, and the Azurite object-store
//! preference. The result is packaged as an [`EnvManifest`] — the persisted record
//! of *what* an environment is. The desktop crate owns the side effects (planning
//! is pure; writing the manifest and the rendered compose, running Docker, are not).
//!
//! Environments select baseline catalog **module ids** directly (`headwaters`,
//! `mlflow`, `azurite`); the catalog is the crate's embedded [`baseline_catalog`],
//! the source of truth for the service definitions (Envoy, Postgres, Azurite,
//! MLflow, headwaters lineage, …), so hydrofoil carries no compose fragments of its
//! own. Lineage is served by **headwaters** (the `lineage` role provider), run
//! **API-only** — hydrofoil ships its own lineage UI, so the `HEADWATERS_SERVE_UI`
//! knob is forced off. Observability is *not* a module: it's an app-level opt-in
//! (a boolean on the environment) that brings up the shared telemetry collector.

use olai_stack_topology::{baseline_catalog, Catalog, EnvManifest, PlanCtx, Role, Selection};

// Re-export the topology types the rest of the desktop crate needs.
pub use olai_stack_topology::{EnvManifest as Manifest, Plan, Role as ServiceRole, Vantage};

/// The headwaters knob (private in the crate) that toggles its bundled lineage UI.
/// Hydrofoil forces it off and renders lineage in its own UI.
const HEADWATERS_SERVE_UI: &str = "HEADWATERS_SERVE_UI";

/// The headwaters OpenLineage REST endpoint id (the `api` endpoint, present in both
/// knob states), used to resolve the lineage sink URL from a [`Plan`].
pub const LINEAGE_ENDPOINT_ID: &str = "api";

/// The vantage hydrofoil's in-process engine occupies when reaching the compose
/// stack: it runs on the Tauri host, so it talks to gatewayed services at
/// `localhost:<gateway_host_port>`.
pub const IN_PROCESS_VANTAGE: Vantage = Vantage::Host;

/// A user-selectable module: a baseline catalog module id plus its UI label.
#[derive(Clone, Copy, serde::Serialize)]
pub struct ModuleDescriptor {
    /// The baseline catalog module id (stored on the environment, sent to the plan).
    pub id: &'static str,
    /// Human-readable label for the UI.
    pub label: &'static str,
}

/// The modules a user can select for an environment, in a stable UI order. These
/// are the user-facing service choices; observability is intentionally absent (it's
/// an app-level opt-in, not a per-env module).
pub fn available_modules() -> &'static [ModuleDescriptor] {
    &[
        ModuleDescriptor {
            id: "headwaters",
            label: "Lineage",
        },
        ModuleDescriptor {
            id: "mlflow",
            label: "Model tracking",
        },
        ModuleDescriptor {
            id: "azurite",
            label: "Object storage",
        },
    ]
}

/// Whether `id` is a module a user is allowed to select (rejects ids the backend
/// won't resolve before they're persisted).
pub fn is_known_module(id: &str) -> bool {
    available_modules().iter().any(|m| m.id == id)
}

/// The shared service catalog hydrofoil plans against: the crate's embedded baseline.
pub fn catalog() -> Catalog {
    baseline_catalog()
}

/// Build the [`Selection`] for a set of selected module ids: the modules plus
/// hydrofoil's fixed knob choices (headwaters API-only).
fn selection(modules: &[String]) -> Selection {
    let mut selection = Selection::modules(modules.to_vec());
    if modules.iter().any(|m| m == "headwaters") {
        // Run headwaters API-only: hydrofoil renders lineage in its own UI.
        selection
            .knob_overrides
            .entry("headwaters".into())
            .or_default()
            .insert(HEADWATERS_SERVE_UI.into(), "false".into());
    }
    selection
}

/// Build the [`PlanCtx`] from host facts. `env_name` becomes the compose project name;
/// `data_root` is the per-environment writable data dir the desktop crate resolves;
/// `gateway_host_port` is the Envoy port published on the host. Object storage prefers
/// Azurite (hydrofoil's choice) over the baseline default (SeaweedFS).
pub fn plan_ctx(
    env_name: impl Into<String>,
    data_root: impl Into<String>,
    gateway_host_port: u16,
) -> PlanCtx {
    let mut ctx = PlanCtx {
        env_name: env_name.into(),
        data_root: data_root.into(),
        gateway_host_port,
        ..PlanCtx::default()
    };
    ctx.provider_preference
        .insert(Role::OBJECT_STORE.to_string(), vec!["azurite".into()]);
    ctx
}

/// Assemble the [`EnvManifest`] (selection + context) for an environment. This is the
/// persisted, re-plannable record of what the environment runs; the desktop crate saves
/// it per environment and re-plans from it on every start/edit.
pub fn manifest(
    modules: &[String],
    env_name: impl Into<String>,
    data_root: impl Into<String>,
    gateway_host_port: u16,
) -> EnvManifest {
    EnvManifest::new(
        selection(modules),
        plan_ctx(env_name, data_root, gateway_host_port),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use olai_stack_topology::render_all;

    fn ids(plan: &Plan) -> Vec<String> {
        plan.graph
            .nodes
            .iter()
            .map(|m| m.id().as_str().to_string())
            .collect()
    }

    fn modules(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn headwaters_selection_is_api_only_and_drops_marquez() {
        let sel = selection(&modules(&["headwaters"]));
        assert!(sel.modules.iter().any(|m| m.as_str() == "headwaters"));
        assert!(!sel.modules.iter().any(|m| m.as_str() == "marquez"));
        // Knob forces the bundled UI off.
        assert_eq!(
            sel.knob_overrides[&"headwaters".into()].get(HEADWATERS_SERVE_UI),
            Some(&"false".to_string())
        );
    }

    #[test]
    fn headwaters_plan_covers_its_deps() {
        let m = manifest(&modules(&["headwaters"]), "lh-test", "/tmp/data", 9080);
        let plan = m.plan(&catalog()).expect("plan");
        let got = ids(&plan);
        // headwaters + its hard dep envoy + the auto-provisioned relational store.
        assert!(
            got.iter().any(|id| id == "headwaters"),
            "headwaters in {got:?}"
        );
        assert!(got.iter().any(|id| id == "envoy"), "envoy in {got:?}");
        assert!(got.iter().any(|id| id == "postgres"), "postgres in {got:?}");
    }

    #[test]
    fn lineage_url_resolves_through_gateway_from_host() {
        let m = manifest(&modules(&["headwaters"]), "lh-test", "/tmp/data", 9080);
        let plan = m.plan(&catalog()).expect("plan");
        let lineage = plan
            .service_by_role(&Role::lineage())
            .expect("lineage service");
        let url = lineage
            .address(IN_PROCESS_VANTAGE, LINEAGE_ENDPOINT_ID)
            .expect("lineage address");
        // Host vantage → gatewayed service → localhost on the gateway host port, at the
        // headwaters OpenLineage API path. This exact URL is what the in-process engine
        // POSTs lineage events to, so assert it whole.
        assert_eq!(url.as_str(), "http://localhost:9080/api/v1/lineage");
    }

    #[test]
    fn mlflow_module_selects_mlflow() {
        let sel = selection(&modules(&["mlflow"]));
        assert!(sel.modules.iter().any(|m| m.as_str() == "mlflow"));
    }

    #[test]
    fn object_storage_prefers_azurite() {
        let ctx = plan_ctx("lh", "/tmp", 9080);
        assert_eq!(
            ctx.provider_preference.get(Role::OBJECT_STORE),
            Some(&vec!["azurite".into()])
        );
    }

    #[test]
    fn empty_selection_plans_to_empty_graph() {
        let m = manifest(&[], "lh", "/tmp", 9080);
        let plan = m.plan(&catalog()).expect("plan");
        assert!(plan.graph.nodes.is_empty());
    }

    #[test]
    fn manifest_round_trips_and_replan_is_port_stable() {
        let m = manifest(&modules(&["headwaters", "mlflow"]), "lh", "/d", 9080);
        let toml = m.to_toml().expect("to_toml");
        let back = EnvManifest::from_toml(&toml).expect("from_toml");
        assert_eq!(m, back);
        let a = m.plan(&catalog()).expect("plan a");
        let b = back.plan(&catalog()).expect("plan b");
        assert_eq!(a.gateway_host_port(), b.gateway_host_port());
        assert_eq!(render_all(&a).compose, render_all(&b).compose);
    }
}
