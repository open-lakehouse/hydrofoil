//! Cedar policy enforcement for Apache DataFusion.
//!
//! This crate is the DataFusion-aware, reusable half of hydrofoil's policy
//! stack — the policy analog of [`datafusion-open-lineage`]. It owns the
//! [`Policy`] trait, the Cedar-backed implementation ([`CedarPolicy`]), and the
//! [`LogicalPlan`](datafusion::logical_expr::LogicalPlan) walk that turns a
//! query into a set of Cedar authorization requests. Policy *sourcing* (pulling
//! a policy set / schema / entities from an OCI registry) lives in the
//! `cedar-oci` crate; engine-specific *glue* (extracting the principal from a
//! request, wiring into a session) lives in the host (`hydrofoil`).
//!
//! Two layers, mirroring `docs/policy-enforcement-design.md`:
//!
//! - **Layer 1 — coarse access gate** ([`Policy::is_allowed`]): does the
//!   principal have access to the tables/actions a query references?
//! - **Layer 2 — fine-grained governance** (feature `governance`): row filters
//!   and column masks derived from Cedar partial-evaluation residuals.

mod cedar;
mod policy;
mod principal;
mod visitor;

#[cfg(feature = "governance")]
pub mod govern;
#[cfg(feature = "governance")]
mod translate;

pub use cedar::CedarPolicy;
pub use policy::{Policy, StaticPolicy};
pub use principal::PrincipalIdentity;

#[cfg(feature = "governance")]
pub use govern::{TablePolicy, govern_plan};
#[cfg(feature = "governance")]
pub use translate::{CedarResidualTranslator, ResidualTranslator};

// Re-export the cedar identity/decision types through this crate so consumers
// have a single import surface (they originate in `cedar-oci`).
pub use cedar_oci::{Decision, EntityId, EntityTypeName, EntityUid};

// Cedar value type the host needs to build principal/resource attributes.
pub use cedar_policy::RestrictedExpression;

// Cedar provider traits a `CedarPolicy` is generic over, re-exported for
// consumers building an authorizer.
pub use cedar_local_agent::public::{SimpleEntityProvider, SimplePolicySetProvider};
