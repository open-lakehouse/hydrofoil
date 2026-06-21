//! Custom SQL front end for hydrofoil's Unity Catalog DDL and Delta commands.
//!
//! DataFusion's stock parser does not understand Unity Catalog DDL such as
//! `CREATE`/`DROP CATALOG` and `CREATE`/`DROP SCHEMA`. This module layers a
//! thin `sqlparser`-based front end ([`HFParser`]) on top of DataFusion's own
//! parser: it recognizes the Unity Catalog statements and the Delta `VACUUM`
//! command (see [`commands`]) and falls back to DataFusion for everything else,
//! producing the [`Statement`] enum.
//!
//! The Unity Catalog statement *types* and their planner/executor live in
//! `datafusion-unitycatalog` (the `datafusion_unitycatalog::sql` module) so the
//! UC↔DataFusion integration is owned in one place; they are re-exported here so
//! the host (hydrofoil) has a single SQL facade. The parser stays here because
//! it also owns Delta-table maintenance commands (`VACUUM`, and `OPTIMIZE` in
//! future) which are not Unity Catalog concerns.
//!
//! Unity Catalog statements are lowered to a
//! [`ExecuteUnityCatalogPlanNode`](datafusion_unitycatalog::sql::ExecuteUnityCatalogPlanNode)
//! (a DataFusion `Extension` node) so they flow through the normal logical →
//! physical planning path. Authorization for that DDL is the Cedar policy
//! layer's responsibility; see `crates/datafusion-cedar`.
//!
//! `VACUUM` is parsed but not yet routed to an executor, and the
//! `CREATE FOREIGN CATALOG / CONNECTION / LOCATION / SHARE` and `CREATE FUNCTION`
//! statements remain future work.

mod commands;
mod parser;

pub use commands::*;
pub use parser::*;
// Re-export the Unity Catalog DDL statement types + planner from
// `datafusion-unitycatalog` so the host can keep importing them from
// `deltalake_datafusion::sql`.
pub use datafusion_unitycatalog::sql::*;
