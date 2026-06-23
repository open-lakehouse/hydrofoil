//! Desktop environment **service modules**: the pure model and dependency
//! resolver for the optional services an environment can run alongside its Unity
//! Catalog server (MLflow, Marquez, Azurite, marimo, …).
//!
//! This crate is intentionally side-effect-free — no process spawning, no I/O, no
//! Tauri. It answers one question: given a user's module selection, what is the
//! full, ordered set of modules to run and how do they depend on each other? The
//! desktop crate (`node/desktop/src-tauri`) consumes [`resolve`] and turns the
//! resulting [`ResolvedGraph`] into compose artifacts + uvx launches.
//!
//! Topology: Unity Catalog runs on the host (not as a module); modules consume it
//! via an injected URL. See `docs/adr` / the env-service-modules design.

pub mod capability;
pub mod effect;
pub mod generate;
pub mod model;
pub mod resolve;

pub use capability::{Capability, Provider};
pub use effect::{Effect, EffectConsumer, EffectKind};
pub use generate::{ComposeArtifacts, LaunchContext, generate_compose, uvx_uc_uri};
pub use model::{Module, ModuleId, ModuleKind, registry};
pub use resolve::{
    Edge, ResolveError, ResolvedGraph, resolve, resolve_capabilities, resolve_with,
};

#[cfg(test)]
mod tests {
    use super::model::{Module, ModuleKind};
    use super::resolve::{ResolveError, resolve, resolve_with};

    fn ids(graph: &super::ResolvedGraph) -> Vec<String> {
        graph.nodes.iter().map(|m| m.id.clone()).collect()
    }

    /// A node always appears after every module it requires (valid startup order).
    fn assert_deps_before_dependents(graph: &super::ResolvedGraph) {
        let order: std::collections::HashMap<&str, usize> = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id.as_str(), i))
            .collect();
        for m in &graph.nodes {
            for dep in &m.requires {
                if let Some(&dep_pos) = order.get(dep.as_str()) {
                    assert!(
                        dep_pos < order[m.id.as_str()],
                        "dependency {dep} must precede dependent {}",
                        m.id
                    );
                }
            }
        }
    }

    #[test]
    fn mlflow_pulls_in_postgres_azurite_and_envoy() {
        let graph = resolve(&["mlflow".into()]).unwrap();
        let got = ids(&graph);
        for required in ["mlflow", "postgres", "azurite", "envoy"] {
            assert!(got.contains(&required.to_string()), "missing {required} in {got:?}");
        }
        assert_deps_before_dependents(&graph);
    }

    #[test]
    fn marquez_pulls_in_postgres_and_envoy() {
        let graph = resolve(&["marquez".into()]).unwrap();
        let got = ids(&graph);
        for required in ["marquez", "postgres", "envoy"] {
            assert!(got.contains(&required.to_string()), "missing {required} in {got:?}");
        }
        // Marquez does not need Azurite.
        assert!(!got.contains(&"azurite".to_string()));
        assert_deps_before_dependents(&graph);
    }

    #[test]
    fn marimo_alone_needs_no_docker() {
        let graph = resolve(&["marimo".into()]).unwrap();
        assert_eq!(ids(&graph), vec!["marimo".to_string()]);
        assert!(!graph.needs_docker());
        assert_eq!(graph.uvx_modules().len(), 1);
        assert!(graph.docker_modules().is_empty());
    }

    #[test]
    fn shared_dependency_is_included_once() {
        // Both mlflow and marquez require postgres + envoy; the closure must not
        // duplicate them.
        let graph = resolve(&["mlflow".into(), "marquez".into()]).unwrap();
        let got = ids(&graph);
        assert_eq!(got.iter().filter(|id| *id == "postgres").count(), 1);
        assert_eq!(got.iter().filter(|id| *id == "envoy").count(), 1);
        assert_deps_before_dependents(&graph);
    }

    #[test]
    fn selection_order_does_not_change_result() {
        let a = resolve(&["mlflow".into(), "marquez".into()]).unwrap();
        let b = resolve(&["marquez".into(), "mlflow".into()]).unwrap();
        assert_eq!(a, b, "resolution must be deterministic regardless of input order");
    }

    #[test]
    fn empty_selection_is_empty_graph() {
        let graph = resolve(&[]).unwrap();
        assert!(graph.nodes.is_empty());
        assert!(graph.edges.is_empty());
        assert!(!graph.needs_docker());
    }

    #[test]
    fn unknown_module_errors() {
        let err = resolve(&["nope".into()]).unwrap_err();
        assert_eq!(err, ResolveError::UnknownModule("nope".into()));
    }

    #[test]
    fn lineage_capability_runs_marquez_and_wires_hydrofoil() {
        use super::capability::Capability;
        use super::effect::{EffectConsumer, EffectKind};
        let graph = super::resolve_capabilities(&[Capability::Lineage]).unwrap();
        // Marquez (+ its deps postgres, envoy) run.
        for required in ["marquez", "postgres", "envoy"] {
            assert!(ids(&graph).contains(&required.to_string()), "missing {required}");
        }
        // A lineage effect wires Hydrofoil, produced by marquez.
        let effect = graph.effect(EffectKind::LineageEndpoint).expect("lineage effect");
        assert_eq!(effect.producer.as_deref(), Some("marquez"));
        assert!(effect.consumers.contains(&EffectConsumer::Hydrofoil));
    }

    #[test]
    fn duplicate_capability_provider_listed_once() {
        use super::capability::Capability;
        // Selecting model tracking pulls MLflow in exactly once (and its deps).
        let graph = super::resolve_capabilities(&[Capability::ModelTracking]).unwrap();
        assert_eq!(ids(&graph).iter().filter(|id| *id == "mlflow").count(), 1);
    }

    #[test]
    fn observability_is_shared_infra_runs_no_module() {
        use super::capability::Capability;
        // Observability alone runs no per-env service (it opts the env in to
        // emitting to the shared app-level collector), so the graph is empty.
        let graph = super::resolve_capabilities(&[Capability::Observability]).unwrap();
        assert!(graph.nodes.is_empty(), "observability must run no per-env module");
        assert!(!graph.needs_docker());
        assert!(Capability::Observability.is_shared_infra());
        assert!(Capability::wants_observability(&[Capability::Observability]));
        assert!(!Capability::wants_observability(&[Capability::Lineage]));
    }

    #[test]
    fn object_storage_effect_lists_mlflow_and_uc_consumers() {
        use super::capability::Capability;
        use super::effect::{EffectConsumer, EffectKind};
        let graph = super::resolve_capabilities(&[Capability::ObjectStorage]).unwrap();
        let effect = graph.effect(EffectKind::ObjectStorage).expect("object storage effect");
        // MLflow consumes the bucket now; UC vending is designed (carried) but not wired.
        assert!(effect.consumers.contains(&EffectConsumer::Mlflow));
        assert!(effect.consumers.contains(&EffectConsumer::UnityCatalog));
    }

    #[test]
    fn cycle_is_detected() {
        let registry = vec![
            Module {
                id: "a".into(),
                name: "A".into(),
                kind: ModuleKind::DockerService { fragment: "a.yaml".into() },
                requires: vec!["b".into()],
            },
            Module {
                id: "b".into(),
                name: "B".into(),
                kind: ModuleKind::DockerService { fragment: "b.yaml".into() },
                requires: vec!["a".into()],
            },
        ];
        let err = resolve_with(&["a".into()], &registry).unwrap_err();
        assert_eq!(err, ResolveError::Cycle(vec!["a".into(), "b".into()]));
    }
}
