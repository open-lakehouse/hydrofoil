//! Hydrofoil's own error type.
//!
//! Currently unused — the crate threads `datafusion::error::Result` /
//! `DataFusionError` through its planning and execution paths. This type is
//! retained as the home for hydrofoil-specific errors (Unity Catalog / IO) that
//! don't map cleanly onto `DataFusionError`; wire it in by returning
//! `crate::error::Result` from the relevant boundary functions.
#![allow(dead_code)]

// A convenience type for declaring Results.
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("UnityCatalog error: {0}")]
    UnityCatalog(#[from] unitycatalog_common::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Generic error: {0}")]
    Generic(String),
}
