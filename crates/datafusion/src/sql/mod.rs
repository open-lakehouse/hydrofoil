//! Work-in-progress custom SQL parser for hydrofoil (currently **disabled**).
//!
//! This module holds a half-finished `sqlparser`-based front end that would let
//! hydrofoil recognize Unity Catalog DDL (`CREATE`/`DROP CATALOG`, see
//! [`unity`]) and maintenance commands (`VACUUM`, see [`commands`]) beyond what
//! DataFusion's own parser handles, producing the `parser::Statement` enum.
//!
//! It is **not wired in**: `mod parser;` / `mod unity;` and the `pub use
//! parser::*` re-export below are intentionally commented out, and
//! `commands/mod.rs` is an empty stub, so none of it compiles today. Hydrofoil
//! currently relies on DataFusion's stock parser plus the live
//! [`crate::catalog::unity`] resolution path; this parser is kept here as the
//! starting point for that work, not as dead-but-live code. Re-enabling it means
//! filling in `commands/mod.rs`, uncommenting the modules below, and threading
//! `HFParser` into hydrofoil's planning path.

mod commands;
// mod parser;
// mod unity;

// pub use parser::*;
