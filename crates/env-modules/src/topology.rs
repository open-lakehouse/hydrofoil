//! Bridge from hydrofoil's capability vocabulary to the shared
//! [`olai-stack-topology`](olai_stack_topology) model.
//!
//! Hydrofoil's UI speaks [`Capability`](crate::capability::Capability); the topology
//! crate speaks [`Selection`] + [`PlanCtx`] against a [`Catalog`]. This module maps
//! the former to the latter, packaged as an [`EnvManifest`] — the persisted record of
//! *what* an environment is. The desktop crate owns the side effects (planning is pure;
//! writing the manifest and the rendered compose, running Docker, are not).
//!
//! The catalog is the crate's embedded [`baseline_catalog`]: it is the source of truth
//! for the service definitions (Envoy, Postgres, Azurite, MLflow, headwaters lineage,
//! …), so hydrofoil no longer carries its own compose fragments. Lineage is served by
//! **headwaters** (the `lineage` role provider), run **API-only** — hydrofoil ships its
//! own lineage UI, so the `HEADWATERS_SERVE_UI` knob is forced off.

use olai_stack_topology::{Catalog, EnvManifest, PlanCtx, Role, Selection, baseline_catalog};

// Re-export the topology types the desktop orchestrator needs, so it depends on the
// shared model through this crate's bridge rather than taking a second direct
// dependency on `olai-stack-topology` (the desktop crate is a separate workspace).
pub use olai_stack_topology::{EnvManifest as Manifest, Plan, Role as ServiceRole, Vantage};

use crate::capability::Capability;

/// The headwaters knob (private in the crate) that toggles its bundled lineage UI.
/// Hydrofoil forces it off and renders lineage in its own UI.
const HEADWATERS_SERVE_UI: &str = "HEADWATERS_SERVE_UI";

/// The headwaters OpenLineage REST endpoint id (the `api` endpoint, present in both
/// knob states), used to resolve the lineage sink URL from a [`Plan`](olai_stack_topology::Plan).
pub const LINEAGE_ENDPOINT_ID: &str = "api";

/// The shared service catalog hydrofoil plans against: the crate's embedded baseline.
pub fn catalog() -> Catalog {
    baseline_catalog()
}

/// Map a hydrofoil [`Capability`] to the baseline module id(s) that provide it. Returns
/// an empty slice for shared-infra capabilities (observability runs no per-env module).
fn capability_modules(cap: Capability) -> &'static [&'static str] {
    match cap {
        Capability::Lineage => &["headwaters"],
        Capability::ModelTracking => &["mlflow"],
        Capability::ObjectStorage => &["azurite"],
        // Observability opts the env in to emitting to the shared collector; no per-env
        // module. (The shared Jaeger is its own app-level project, like today.)
        Capability::Observability => &[],
    }
}

/// Build the [`Selection`] for a set of selected capabilities: the union of their
/// provider modules, plus hydrofoil's fixed knob choices (headwaters API-only).
fn selection(capabilities: &[Capability]) -> Selection {
    let mut modules: Vec<String> = Vec::new();
    let mut lineage = false;
    for cap in capabilities {
        for id in capability_modules(*cap) {
            if !modules.iter().any(|m| m == id) {
                modules.push((*id).to_string());
            }
        }
        if *cap == Capability::Lineage {
            lineage = true;
        }
    }

    let mut selection = Selection::modules(modules);
    if lineage {
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
    capabilities: &[Capability],
    env_name: impl Into<String>,
    data_root: impl Into<String>,
    gateway_host_port: u16,
) -> EnvManifest {
    EnvManifest::new(
        selection(capabilities),
        plan_ctx(env_name, data_root, gateway_host_port),
    )
}

/// The vantage hydrofoil's in-process engine occupies when reaching the compose stack:
/// it runs on the Tauri host, so it talks to gatewayed services at
/// `localhost:<gateway_host_port>`.
pub const IN_PROCESS_VANTAGE: Vantage = Vantage::Host;

#[cfg(test)]
mod tests {
    use super::*;
    use olai_stack_topology::render_all;

    fn ids(plan: &olai_stack_topology::Plan) -> Vec<String> {
        plan.graph
            .nodes
            .iter()
            .map(|m| m.id().as_str().to_string())
            .collect()
    }

    #[test]
    fn lineage_selects_headwaters_api_only_and_drops_marquez() {
        let sel = selection(&[Capability::Lineage]);
        assert!(sel.modules.iter().any(|m| m.as_str() == "headwaters"));
        assert!(!sel.modules.iter().any(|m| m.as_str() == "marquez"));
        // Knob forces the bundled UI off.
        assert_eq!(
            sel.knob_overrides[&"headwaters".into()].get(HEADWATERS_SERVE_UI),
            Some(&"false".to_string())
        );
    }

    #[test]
    fn lineage_plan_covers_headwaters_and_deps() {
        let m = manifest(&[Capability::Lineage], "lh-test", "/tmp/data", 9080);
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
        let m = manifest(&[Capability::Lineage], "lh-test", "/tmp/data", 9080);
        let plan = m.plan(&catalog()).expect("plan");
        let lineage = plan
            .service_by_role(&Role::lineage())
            .expect("lineage service");
        let url = lineage
            .address(IN_PROCESS_VANTAGE, LINEAGE_ENDPOINT_ID)
            .expect("lineage address");
        // Host vantage → gatewayed service → localhost on the gateway host port.
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("localhost"));
        assert_eq!(url.port(), Some(9080));
    }

    #[test]
    fn model_tracking_selects_mlflow() {
        let sel = selection(&[Capability::ModelTracking]);
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
    fn observability_adds_no_module() {
        let sel = selection(&[Capability::Observability]);
        assert!(sel.modules.is_empty());
    }

    #[test]
    fn manifest_round_trips_and_replan_is_port_stable() {
        let m = manifest(
            &[Capability::Lineage, Capability::ModelTracking],
            "lh",
            "/d",
            9080,
        );
        let toml = m.to_toml().expect("to_toml");
        let back = EnvManifest::from_toml(&toml).expect("from_toml");
        assert_eq!(m, back);
        let a = m.plan(&catalog()).expect("plan a");
        let b = back.plan(&catalog()).expect("plan b");
        assert_eq!(a.gateway_host_port(), b.gateway_host_port());
        assert_eq!(render_all(&a).compose, render_all(&b).compose);
    }
}
