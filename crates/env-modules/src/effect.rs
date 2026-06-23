//! Effects: declarative produce/consume wiring records.
//!
//! An *effect* is the cross-service wiring that makes a capability useful beyond
//! starting containers — "pass a piece of information around, or create a
//! management object." A provider *produces* effects (e.g. a lineage sink
//! produces a `LineageEndpoint`); an engine or service *consumes* them (Hydrofoil
//! consumes the lineage endpoint by emitting OpenLineage to it).
//!
//! Effects are computed during resolution but their payloads are only fully known
//! at runtime (an endpoint URL depends on the resolved port). So the model carries
//! the *kind* of effect a provider produces/consumes; the desktop crate fills in
//! the concrete values after the producing services are healthy, then applies them
//! to consumers (for in-process Hydrofoil, by setting `HostConfig` fields).

use serde::{Deserialize, Serialize};

/// The kind of an effect — what is produced/consumed, independent of its runtime
/// payload. Used in the resolved graph to connect producers to consumers.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    /// An OpenLineage HTTP endpoint a lineage sink exposes; Hydrofoil emits to it.
    LineageEndpoint,
    /// Object storage: a bucket/container for artifacts, plus (designed, not yet
    /// wired) the credentials to vend into Unity Catalog as an external location.
    ObjectStorage,
}

/// Who consumes an effect — determines how/where the desktop crate applies it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectConsumer {
    /// The in-process Hydrofoil engine — applied via `HostConfig` before build.
    Hydrofoil,
    /// MLflow — consumes an object-storage bucket for its artifact store.
    Mlflow,
    /// Unity Catalog — consumes object storage as an external location +
    /// credential. Designed; the vending call is deferred (see ADR 0017).
    UnityCatalog,
}

/// A declarative wiring edge: a provider produces `kind`, and the listed
/// `consumers` are wired to it once its payload is known. The runtime payload
/// (an endpoint URL, a bucket name + credentials) is resolved by the desktop
/// crate after the producing services are healthy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effect {
    pub kind: EffectKind,
    /// The module that produces this effect (its service must be healthy before
    /// the payload is known). `None` for effects produced by always-present
    /// infrastructure rather than a module.
    #[serde(default)]
    pub producer: Option<String>,
    /// The consumers wired to this effect.
    pub consumers: Vec<EffectConsumer>,
}
