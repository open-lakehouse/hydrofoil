//! Capabilities: the user-facing intent layer over the topology catalog.
//!
//! A *capability* is what the user wants ("lineage", "observability", "model
//! tracking", "object storage"). The environment stores selected capabilities; the
//! [`topology`](crate::topology) bridge maps each to the baseline catalog module(s)
//! that provide it, assembling the environment's selection.

use serde::{Deserialize, Serialize};

/// A user-facing capability. Stored on the environment; mapped to catalog modules by
/// [`topology::manifest`](crate::topology::manifest).
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
}
