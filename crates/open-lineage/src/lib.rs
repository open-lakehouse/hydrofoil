//! OpenLineage integration for Apache DataFusion.
//!
//! Wrap a `SessionState`'s query planner with [`instrument_session_state`] to
//! emit [OpenLineage](https://openlineage.io) run events (START / COMPLETE /
//! FAIL) describing each query's input/output datasets and column-level
//! lineage.
//!
//! The sink is pluggable via the [`Transport`] trait; the default
//! [`CloudClientTransport`] (feature `http`) posts events to a deployed,
//! possibly authenticated, OpenLineage endpoint via `olai-http`. Orchestration
//! metadata (parent run, job ids, custom facets) is injected per query via a
//! [`LineageContextProvider`].
//!
//! ```no_run
//! use std::sync::Arc;
//! use datafusion::execution::SessionStateBuilder;
//! use datafusion_open_lineage::{
//!     instrument_session_state_simple, OpenLineageClient, OpenLineageConfig,
//! };
//!
//! # async fn wire() {
//! let state = SessionStateBuilder::new_with_default_features().build();
//! let client = OpenLineageClient::from_env().unwrap();
//! let state = instrument_session_state_simple(state, client, OpenLineageConfig::default());
//! # let _ = state;
//! # }
//! ```

pub mod builder;
pub mod client;
pub mod column;
pub mod config;
pub mod context;
pub mod event;
pub mod exec;
pub mod extract;
pub mod facets;
pub mod naming;
pub mod planner;
pub mod session;
pub mod transport;

#[cfg(feature = "http")]
pub mod cloud;

pub use client::{ClientError, OpenLineageClient, OpenLineageClientBuilder};
pub use config::OpenLineageConfig;
pub use context::{LineageContext, LineageContextProvider, StaticContextProvider};
pub use event::{Dataset, Job, Run, RunEvent, RunEventType};
pub use exec::OpenLineageExec;
pub use extract::{QueryLineage, extract};
pub use naming::DatasetName;
pub use planner::OpenLineageQueryPlanner;
pub use session::{instrument_session_state, instrument_session_state_simple};
pub use transport::{ConsoleTransport, NoopTransport, Transport, TransportError};

#[cfg(feature = "http")]
pub use cloud::CloudClientTransport;
