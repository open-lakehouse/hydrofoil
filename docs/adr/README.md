# Architecture Decision Records

Short, dated records of significant decisions, in the
[Nygard](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions)
/ [MADR-lite](https://adr.github.io/madr/) format: **Title, Status, Context,
Decision, Consequences**. Each ADR cross-references the design docs in `docs/`
that it draws on or refines.

| ADR | Title | Status |
| --- | ----- | ------ |
| [0001](0001-layered-session-context-model.md) | Layered session/context model (Engine / Session / per-query) | Accepted |
| [0002](0002-flight-sql-session-identity.md) | Flight SQL session identity via handshake + Bearer/cookie | Accepted |
| [0003](0003-per-statement-run-id-correlation.md) | Per-statement `run_id` for START/COMPLETE correlation | Accepted |
| [0004](0004-per-session-credential-isolation.md) | Per-session `RuntimeEnv` for credential isolation | Accepted |
| [0005](0005-per-query-agent-governance-context.md) | Per-query agent / governance context as a session extension | Accepted |
| [0006](0006-policy-fact-locality-and-session-state.md) | Policy fact locality, the session fact store, and residual handling | Accepted |
| [0007](0007-fact-gathering-pips.md) | Fact-gathering PIPs: resource/catalog facts, the fact store, trait-now/impl-later | Accepted |
| [0008](0008-principal-identity-resolution.md) | Principal/identity resolution: dynamic group membership and enrichment freshness | Accepted |

Related design docs: [`session-management.md`](../session-management.md),
[`open-lineage-design.md`](../open-lineage-design.md),
[`policy-enforcement-design.md`](../policy-enforcement-design.md),
[`platform-policy-architecture.md`](../platform-policy-architecture.md),
[`policy-fact-gathering.md`](../policy-fact-gathering.md).
