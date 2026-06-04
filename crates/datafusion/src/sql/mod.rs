//! Custom SQL front end for hydrofoil's Unity Catalog DDL.
//!
//! DataFusion's stock parser does not understand Unity Catalog DDL such as
//! `CREATE`/`DROP CATALOG` and `CREATE`/`DROP SCHEMA`. This module layers a
//! thin `sqlparser`-based front end ([`HFParser`]) on top of DataFusion's own
//! parser: it recognizes the Unity Catalog statements (see [`unity`]) and falls
//! back to DataFusion for everything else, producing the [`Statement`] enum.
//!
//! Unity Catalog statements are lowered to a [`unity::ExecuteUnityCatalogPlanNode`]
//! (a DataFusion `Extension` node) so they flow through the normal logical →
//! physical planning path. Authorization for that DDL is the Cedar policy
//! layer's responsibility; see `crates/datafusion-cedar`.
//!
//! `VACUUM` (see [`commands`]) is parsed but not yet routed to an executor, and
//! the `CREATE FOREIGN CATALOG / CONNECTION / LOCATION / SHARE` and
//! `CREATE FUNCTION` statements remain future work.

mod commands;
mod parser;
mod unity;

pub use parser::*;
pub use unity::*;
