//! Hydrofoil's per-query agent / governance context wiring.
//!
//! An upstream agent invocation can differ from one query to the next *within a
//! single session* (the same human session may drive many distinct agent tasks).
//! So unlike the principal (bound per connection in [`crate::identity`]) and the
//! parent-run lineage context, this context is **per request**: it is parsed
//! from the request metadata at the call site and attached to a per-query
//! `SessionState` clone via a `SessionConfig` extension — never persisted into
//! the long-lived session.
//!
//! For now the policy layer only *logs* this context (see
//! `LakehouseSession::create_physical_plan`); it is the seam the deferred agent
//! policy-enforcement point (PEP) and session taint ledger will read — see
//! `docs/platform-policy-architecture.md` (agentic authorization) and
//! `docs/adr/0005-per-query-agent-governance-context.md`.

use tonic::metadata::MetadataMap;

/// gRPC metadata keys carrying per-query agent / governance context.
pub mod headers {
    /// Stable identity of the calling agent (e.g. an OIDC-A `agent_instance`).
    pub const AGENT_ID: &str = "x-hydrofoil-agent-id";
    /// The agent's own session/conversation id (distinct from the SQL session).
    pub const AGENT_SESSION: &str = "x-hydrofoil-agent-session";
    /// The task the agent is performing this turn.
    pub const AGENT_TASK: &str = "x-hydrofoil-agent-task";
    /// Free-form purpose/justification for the access.
    pub const AGENT_PURPOSE: &str = "x-hydrofoil-agent-purpose";
}

/// Per-query agent / governance context, parsed from request metadata.
///
/// All fields are optional: a non-agent client simply supplies none of them and
/// this resolves to `None` (see [`agent_context_from_metadata`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentContext {
    pub agent_id: Option<String>,
    pub agent_session: Option<String>,
    pub task: Option<String>,
    pub purpose: Option<String>,
}

impl AgentContext {
    /// Whether any field is populated. Used to decide whether to attach the
    /// extension / emit a log line at all.
    pub fn is_empty(&self) -> bool {
        self.agent_id.is_none()
            && self.agent_session.is_none()
            && self.task.is_none()
            && self.purpose.is_none()
    }
}

/// The `SessionConfig` extension type carrying the per-query [`AgentContext`].
///
/// A distinct newtype so `SessionConfig::get_extension` (which keys by
/// `TypeId`) resolves it unambiguously — mirroring
/// [`crate::lineage::LineageContextExt`] and [`crate::identity::PrincipalExt`].
#[derive(Debug, Clone, Default)]
pub struct AgentContextExt(pub AgentContext);

/// Parse the per-query [`AgentContext`] from gRPC request metadata.
///
/// Returns `None` when no agent headers are present, so callers can cheaply skip
/// attaching the extension for non-agent clients.
pub fn agent_context_from_metadata(meta: &MetadataMap) -> Option<AgentContext> {
    let get = |key: &str| {
        meta.get(key)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };

    let ctx = AgentContext {
        agent_id: get(headers::AGENT_ID),
        agent_session: get(headers::AGENT_SESSION),
        task: get(headers::AGENT_TASK),
        purpose: get(headers::AGENT_PURPOSE),
    };

    if ctx.is_empty() { None } else { Some(ctx) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_metadata_yields_none() {
        assert!(agent_context_from_metadata(&MetadataMap::new()).is_none());
    }

    #[test]
    fn parses_full_agent_context() {
        let mut meta = MetadataMap::new();
        meta.insert(headers::AGENT_ID, "agent-7".parse().unwrap());
        meta.insert(headers::AGENT_SESSION, "conv-42".parse().unwrap());
        meta.insert(headers::AGENT_TASK, "summarize-sales".parse().unwrap());
        meta.insert(headers::AGENT_PURPOSE, "quarterly report".parse().unwrap());

        let ctx = agent_context_from_metadata(&meta).expect("agent context present");
        assert_eq!(ctx.agent_id.as_deref(), Some("agent-7"));
        assert_eq!(ctx.agent_session.as_deref(), Some("conv-42"));
        assert_eq!(ctx.task.as_deref(), Some("summarize-sales"));
        assert_eq!(ctx.purpose.as_deref(), Some("quarterly report"));
    }

    #[test]
    fn parses_partial_agent_context() {
        // A single header is enough to produce a context; absent fields stay None.
        let mut meta = MetadataMap::new();
        meta.insert(headers::AGENT_ID, "agent-7".parse().unwrap());

        let ctx = agent_context_from_metadata(&meta).expect("agent context present");
        assert_eq!(ctx.agent_id.as_deref(), Some("agent-7"));
        assert!(ctx.agent_session.is_none());
        assert!(ctx.task.is_none());
        assert!(ctx.purpose.is_none());
    }
}
