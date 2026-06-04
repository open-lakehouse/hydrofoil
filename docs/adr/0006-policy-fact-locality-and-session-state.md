# 0006 — Policy fact locality, the session fact store, and residual handling

> Status: **Accepted** (2026-06). The v1 decision (re-evaluate-fully, option A)
> is validated end-to-end by
> `crates/datafusion-cedar/examples/fact_gathering_walkthrough.rs`. The
> session-fact-store and central session-state PDP it governs remain future work
> (named seams in [`platform-policy-architecture.md`](../platform-policy-architecture.md));
> this ADR fixes the *policy* (locality classification + option A) those will
> implement. Refines
> [`docs/policy-fact-gathering.md`](../policy-fact-gathering.md) and
> [`docs/platform-policy-architecture.md`](../platform-policy-architecture.md).

## Context

The hybrid PDP ([`platform-policy-architecture.md`](../platform-policy-architecture.md),
decision 1C) places an embedded Cedar engine on the hot path and a central
session-state PDP for dynamic state. To make that concrete we must decide **how
facts reach a decision and what is kept between decisions** — a single logical
operation crosses several decision points (catalog → engine coarse gate → engine
governance → agent tool-call), and a *different* subset of facts is known at each.

Two observations force a choice:

1. **Facts differ in locality and lifetime.** A principal parsed from a request,
   or a table's `readers`/`writers` resolved from the catalog at planning, is
   needed only for the evaluation at hand and is cheap to re-derive. Accumulated
   taints, by contrast, are established at one point (the engine, reading a tagged
   column) and must be readable at a *later* point in a *different* service (the
   agent tool-call PEP). Treating these the same — shoving everything into a shared
   store, or re-deriving everything everywhere — is either a locality/privacy
   regression or wasted work.

2. **Residuals are themselves candidate session state.** Cedar partial evaluation
   with an unknown resource yields a *residual* — the not-yet-decided slice of the
   policy. The engine already produces one (to build row filters / column masks via
   `is_authorized_partial`). A later PEP could either re-evaluate from scratch with
   more facts bound, or *resume* from the carried residual. The second avoids
   redundant work but introduces a residual lifecycle.

This sits above the session/context model of [ADR 0001](0001-layered-session-context-model.md)
and the per-query agent context of [ADR 0005](0005-per-query-agent-governance-context.md):
those decide *how* context is carried; this decides *which facts are shared vs.
local* and *whether partial decisions are carried*.

## Decision

**1. Classify every fact by locality + lifetime.**
- **Local-ephemeral** — resolved at the point of use, folded into the Cedar
  `Request` / `Entities`, then discarded: principal uid + attrs, agent context,
  `in_trusted_environment`, table identity, accessed columns, table/column catalog
  attributes (`readers`/`writers`, tags).
- **Shared-session-scoped** — persisted, keyed by the correlation id, so a later /
  other PEP can read them: accumulated `observed_taints`, prior decisions, consent
  / step-up flags.

**2. The session fact store holds *only* shared-session-scoped facts.** Keyed by
the correlation id ([`platform-policy-architecture.md`](../platform-policy-architecture.md),
decision 3); taint accrual is monotonic within a session. Local-ephemeral facts are
never persisted there.

**3. For evaluation continuity, adopt re-evaluate-fully (option A) for v1.** Each
PEP re-evaluates with the accumulated facts read from the store; it does **not**
depend on a residual carried from a prior PEP. Carry-residual (option B) is retained
as a **deferred, feature-gated optimization**, keyed by `(correlation_id,
bundle_version)` so a policy-bundle change invalidates the cache automatically.

## Consequences

- The session fact-store interface need only support: record-fact-by-correlation-id,
  query-facts-by-correlation-id, and monotonic taint accrual. It does **not** store
  policy fragments in v1.
- Both the engine (writing taints at governance time) and the agent tool-call PEP
  (reading them) depend on the store; in the hybrid model it is owned by the central
  session-state PDP.
- Keeping local-ephemeral facts out of the store preserves locality and avoids
  leaking resolved catalog attributes / principal data beyond the evaluation that
  needs them.
- Choosing A keeps each hop simple and stateless-with-respect-to-residuals, at the
  cost of re-doing partial-eval work a later hop shares with an earlier one. Because
  the engine's governance partial-eval is already on the hot path and the agent
  PEP's policy slice is small, this cost is acceptable for v1.
- If/when B is enabled, two obligations become load-bearing: **invalidation** (the
  `(correlation_id, bundle_version)` key must drop the cached residual on a bundle
  bump *and* when an underlying fact changes) and **non-leakage** (a residual encodes
  which attributes still gate access, so it must not be handed to a less-trusted hop).

## Alternatives considered

- **Everything in the shared store.** Simpler mental model (one place for all facts),
  but it persists local-ephemeral data unnecessarily, worsens locality and privacy,
  and couples every decision point to the store even for facts it could resolve
  itself. Rejected.
- **Always carry residuals (B-only).** The elegant end-state for progressive
  refinement, but it makes residual lifecycle (invalidation, leakage) load-bearing
  from day one against a still-maturing Cedar `partial-eval` feature. Rejected for
  v1; preserved as the feature-gated path B above.

## Notes

The runnable walkthrough (`fact_gathering_walkthrough.rs`) demonstrates option A
end-to-end and prints the residual the engine produces, marking exactly where a
B-mode design would cache and resume.
