//! Policy enforcement for hydrofoil.
//!
//! The reusable, DataFusion-aware policy machinery now lives in the
//! `datafusion-cedar` crate (the policy analog of `datafusion-openlineage`).
//! This module re-exports it so the rest of hydrofoil keeps a stable
//! `crate::policy::*` path; hydrofoil-specific glue (principal extraction,
//! session wiring) will live alongside it in later phases.

pub use datafusion_cedar::{CedarPolicy, Policy, StaticPolicy};
