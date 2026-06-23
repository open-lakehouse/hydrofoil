//! Capabilities and providers: the user-facing intent layer over modules.
//!
//! A *capability* is what the user wants ("lineage", "observability", "model
//! tracking", "object storage"). It is satisfied by one *provider* — a technology
//! choice — which declares the modules it runs and the effects it produces or
//! consumes. The environment stores selected capabilities; the resolver maps each
//! to its default provider, then to modules + effects.
//!
//! The provider layer exists so swapping a technology (e.g. object storage from
//! Azurite to SeaweedFS) is a config change, not a refactor — even though only one
//! default provider per capability is offered today (no picker UI yet).

use serde::{Deserialize, Serialize};

use crate::effect::{Effect, EffectConsumer, EffectKind};
use crate::model::ModuleId;

/// A user-facing capability. Stored on the environment; resolved to a provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Data lineage capture (OpenLineage). Sink + Hydrofoil emission.
    Lineage,
    /// Tracing/observability. A per-environment OPT-IN to emit spans — but to a
    /// single, shared, app-level Jaeger collector (telemetry is interesting
    /// across environments, and OpenTelemetry initializes once per process). This
    /// capability runs no per-env service; it signals that this env's engine
    /// should export OTLP to the shared collector. See ADR 0017.
    Observability,
    /// Experiment / model tracking.
    ModelTracking,
    /// Object storage for artifacts (and, later, UC managed storage).
    ObjectStorage,
}

impl Capability {
    /// All capabilities, in a stable UI order.
    pub fn all() -> &'static [Capability] {
        &[
            Capability::Lineage,
            Capability::Observability,
            Capability::ModelTracking,
            Capability::ObjectStorage,
        ]
    }

    /// The stable string id used in persistence and the UI.
    pub fn id(&self) -> &'static str {
        match self {
            Capability::Lineage => "lineage",
            Capability::Observability => "observability",
            Capability::ModelTracking => "model_tracking",
            Capability::ObjectStorage => "object_storage",
        }
    }

    /// Human-readable label for the UI.
    pub fn label(&self) -> &'static str {
        match self {
            Capability::Lineage => "Lineage",
            Capability::Observability => "Observability",
            Capability::ModelTracking => "Model tracking",
            Capability::ObjectStorage => "Object storage",
        }
    }

    /// Whether this capability is satisfied entirely by app-level shared
    /// infrastructure (no per-environment provider/module). Observability is the
    /// one such capability: it opts the env in to emitting to the shared Jaeger.
    pub fn is_shared_infra(&self) -> bool {
        matches!(self, Capability::Observability)
    }

    /// Parse from the persisted/UI id.
    pub fn from_id(id: &str) -> Option<Capability> {
        Capability::all().iter().copied().find(|c| c.id() == id)
    }

    /// Whether a capability selection opts the environment in to emitting
    /// telemetry to the shared app-level collector.
    pub fn wants_observability(caps: &[Capability]) -> bool {
        caps.contains(&Capability::Observability)
    }

    /// The default provider satisfying this capability today, or `None` for a
    /// shared-infra capability (observability) that runs no per-env provider.
    pub fn default_provider(&self) -> Option<Provider> {
        match self {
            Capability::Lineage => Some(Provider::Marquez),
            Capability::ModelTracking => Some(Provider::Mlflow),
            Capability::ObjectStorage => Some(Provider::Azurite),
            Capability::Observability => None,
        }
    }
}

/// A technology choice satisfying a capability. Declares modules + effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// Marquez lineage backend (consumes lineage via Hydrofoil emission).
    Marquez,
    /// MLflow for experiment/model tracking.
    Mlflow,
    /// Azurite object storage.
    Azurite,
}

impl Provider {
    /// The modules (services) this provider runs. These feed the existing module
    /// dependency resolver, which pulls in their transitive `requires`.
    pub fn modules(&self) -> Vec<ModuleId> {
        match self {
            Provider::Marquez => vec!["marquez".into()],
            Provider::Mlflow => vec!["mlflow".into()],
            Provider::Azurite => vec!["azurite".into()],
        }
    }

    /// The effects this provider produces/consumes, with the module that produces
    /// the runtime payload.
    pub fn effects(&self) -> Vec<Effect> {
        match self {
            Provider::Marquez => vec![Effect {
                kind: EffectKind::LineageEndpoint,
                producer: Some("marquez".into()),
                consumers: vec![EffectConsumer::Hydrofoil],
            }],
            // Plain model tracking produces no cross-service effect (the UI talks
            // to MLflow directly through the gateway).
            Provider::Mlflow => vec![],
            Provider::Azurite => vec![Effect {
                kind: EffectKind::ObjectStorage,
                producer: Some("azurite".into()),
                // MLflow consumes the bucket now; UC vending is designed but not
                // wired (see ADR 0017).
                consumers: vec![EffectConsumer::Mlflow, EffectConsumer::UnityCatalog],
            }],
        }
    }
}
