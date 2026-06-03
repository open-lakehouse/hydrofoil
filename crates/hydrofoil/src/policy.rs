//! Policy enforcement for hydrofoil.
//!
//! The reusable, DataFusion-aware policy machinery now lives in the
//! `datafusion-cedar` crate (the policy analog of `datafusion-open-lineage`).
//! This module re-exports it so the rest of hydrofoil keeps a stable
//! `crate::policy::*` path; hydrofoil-specific glue (principal extraction,
//! session wiring) will live alongside it in later phases.

// `CedarPolicy` is wired into the server in Phase 1 (`main.rs` builds it from an
// OCI policy reference); re-exported now so the path is stable.
#[allow(unused_imports)]
pub use datafusion_cedar::{CedarPolicy, Policy, StaticPolicy};
