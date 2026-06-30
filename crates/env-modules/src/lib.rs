//! Desktop environment **service modules**: the bridge from hydrofoil's user-facing
//! capability vocabulary to the shared [`olai-stack-topology`](olai_stack_topology)
//! model that defines, plans, and renders the environment's Docker Compose stack.
//!
//! This crate is intentionally side-effect-free — no process spawning, no I/O, no
//! Tauri. It answers one question: given a user's selected [`Capability`]s, what is the
//! environment's [`Manifest`](topology::Manifest) (selection + context) to plan and
//! render? The desktop crate (`node/desktop/src-tauri`) consumes [`topology`] and owns
//! the side effects (writing the rendered project, running `docker compose`).
//!
//! Topology: Unity Catalog and the query/ingest engine run in-process on the host (not
//! as compose modules); the in-process engine resolves the URLs it needs (the lineage
//! sink, …) from the plan at the host [`Vantage`](topology::Vantage). See the
//! stack-topology adoption plan / the env-service-modules design.

pub mod capability;
pub mod topology;

pub use capability::Capability;
