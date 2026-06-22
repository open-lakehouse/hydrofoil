//! Hydrofoil — the catalog-native query engine, exposed both as a binary
//! (Flight SQL gRPC + HTTP query surface, see `main.rs`) and as a library.
//!
//! The library surface exists so embedders — notably the Tauri desktop backend
//! via `desktop-host` — can build the same engine and ConnectRPC `QueryService`
//! executor and drive it in-process, without standing up the HTTP/gRPC servers.
//! The pieces an embedder needs are re-exported at the crate root
//! ([`FlightSqlServiceImpl`], [`QueryAppState`], [`Config`]); everything the
//! binary uses lives in the same modules and resolves through the same
//! `crate::` paths.

// Handlers implement the generated ConnectRPC service traits with plain
// `async fn`, whose concrete return types refine the trait's `impl Encodable +
// Send` — the idiomatic connect-rust pattern (see `crate::query_service`).
#![allow(refining_impl_trait)]

/// buffa + connect-rust generated code for `hydrofoil.query.v1` (the
/// QueryService). Proto source lives in `proto/hydrofoil-query`; regenerate with
/// `just hydrofoil-gen`.
pub mod generated {
    /// buffa-generated message types (owned structs + zero-copy views).
    #[path = "buffa/mod.rs"]
    pub mod buffa;
    /// connect-rust-generated service traits, dispatchers, and clients.
    #[path = "connect/mod.rs"]
    pub mod connect;
}

/// Convenience re-exports for the generated message and service trees.
pub(crate) use generated::buffa::hydrofoil::query as proto;
pub(crate) use generated::connect::hydrofoil::query as services;

mod agent;
mod catalog;
pub mod config;
mod engine;
mod error;
mod execution;
pub mod http;
mod identity;
mod lineage;
mod planner;
pub mod policy;
mod query;
pub mod query_service;
pub mod server;
mod session;
mod stream;
pub mod telemetry;

// Re-exports for embedders (e.g. `desktop-host`): the shared engine/session
// service, the ConnectRPC QueryService executor wrapper, and the layered config.
pub use config::Config;
pub use query_service::QueryAppState;
pub use server::FlightSqlServiceImpl;
