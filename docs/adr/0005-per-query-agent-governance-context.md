# 0005 — Per-query agent / governance context as a session extension

> Status: **Accepted** (2026-06) — placeholder for the deferred agent PEP.
> Implemented in `crates/hydrofoil/src/agent.rs`, attached in
> `crates/hydrofoil/src/server.rs`, observed in
> `crates/hydrofoil/src/session.rs`. Refines
> [`docs/platform-policy-architecture.md`](../platform-policy-architecture.md).

## Context

`docs/platform-policy-architecture.md` describes agentic authorization: an agent
session invokes tools, and policy must constrain actions based on a per-session
taint ledger and on *who the agent is* (OIDC-A: agent type/model/instance,
delegation chain). The agent invocation driving a query can differ from one query
to the next within a single human session — so this context is fundamentally
**per request**, not per connection.

We need a seam to carry that context into query planning/execution now, even
though the policy engine doesn't yet act on it, so the wiring is in place when the
agent policy-enforcement point (PEP) and taint ledger land.

## Decision

- Define `AgentContext { agent_id, agent_session, task, purpose }`, parsed from
  `x-hydrofoil-agent-*` request metadata by `agent_context_from_metadata`.
- Attach it to the per-query `SessionState` clone as a typed `SessionConfig`
  extension (`AgentContextExt`) via `Session::lakehouse_for_query`, mirroring the
  lineage and principal seams. It is **never** persisted into the long-lived
  `Session`.
- For now the policy layer only **observes** it: `create_physical_plan` logs the
  agent id/session/task. This proves the read-back path.

## Consequences

- The per-query axis is established with the right lifetime (request-scoped) and
  the right mechanism (config extension), consistent with the other context
  types — see [ADR-0001](0001-layered-session-context-model.md).
- The deferred agent PEP / session taint ledger will read `AgentContextExt`
  (alongside accumulated taints) to gate tool calls and apply obligations; this
  ADR is the seam, not that enforcement.
- Headers are a transport seam only; agent identity must eventually be
  cryptographically bound (OIDC-A / PoP-bound tokens) per the platform-policy
  design before it is trusted for authorization.
