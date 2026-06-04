# 0008 — Principal/identity resolution: dynamic group membership and enrichment freshness

> Status: **Accepted** (2026-06). Implemented in `crates/datafusion-cedar`
> (`PrincipalIdentity`, `IdentityProvider`) and `crates/hydrofoil`
> (`ConfigIdentityProvider`, `Engine`, server enrichment). The principal-side
> companion to [ADR-0007](0007-fact-gathering-pips.md); realizes the
> **principal/identity PIP** (decision 4) of
> [`platform-policy-architecture.md`](../platform-policy-architecture.md).

## Context

We cannot see all of a principal's facts from the request alone. Role, region,
group membership, and agent-vs-human status live in external IdP / directory /
group systems that must be *queried* — a potentially large body of facts to pull
*selectively*. Today:

- `principal_from_metadata` reads flat, self-asserted attributes from
  `x-hydrofoil-{role,region}` headers.
- `PrincipalIdentity::to_entity()` emits **empty parents**, so the principal
  entity carries no group membership.
- The group hierarchy that actually makes the coarse `read_table` permit fire
  (`alice ∈ privileged_readers ⊂ readers`) is **hardcoded in the static OCI
  bundle** `config/policies/lakhouse.entities.json`.

So membership is neither dynamic nor authoritative. Cedar resolves
`principal in UserGroup::"readers"` from the principal entity's *parents* and the
group entities supplied in the `Entities` store — both of which we must produce
at request time for membership to be real.

A correctness observation sharpens the freshness question: catalog/table
resolution is **already per-query** (`create_logical_plan` runs `unity.resolve` +
`register_catalog` on every query, `crates/hydrofoil/src/session/mod.rs`), because
upstream catalog metadata can change between queries. A principal's group
membership can likewise change mid-session, so resolving it once at session
creation is knowingly *less fresh* than the catalog path.

## Decision

**Group membership as dynamic Cedar parents, replacing the static bundle.**
`PrincipalIdentity` gains `groups: Vec<EntityUid>` (the principal's direct
parents) and a carried `group_entities` closure (the transitive groups, each with
their own parents/attrs). `to_entity()` now emits the groups as parents;
`to_entities()` returns the principal entity *plus* the group closure — the value
folded into `Entities::from_entities`. Once membership is provider-sourced, the
bundle's per-user/per-group entities become redundant and are shrunk/emptied.

**An `IdentityProvider` PIP, symmetric to the resource PIP.** The trait lives in
`datafusion-cedar` (it deals only in `PrincipalIdentity`/Cedar types); concrete
IdP/directory-querying impls live in hydrofoil. v1 is a `ConfigIdentityProvider`
reading a config map (it moves today's hardcoded bundle facts behind the seam); a
future `OidcIdentityProvider` / `DirectoryIdentityProvider` (OIDC userinfo / SCIM
/ LDAP) is the same trait. The provider walks *up* from the authenticated
principal (groups → ancestors, stop) — it pulls only that principal's closure,
never the directory; that is what "selective" means here.

**Engine owns the provider and a uid-keyed enrichment cache.** For v1,
enrichment is resolved once per session (in the server, between
`principal_from_metadata` and session creation) and the enriched
`PrincipalIdentity` is stored on the `Session`. The cache and the enrichment call
live on `Engine` (a sibling of the fact store) specifically so the resolution
call can **move to the per-query `create_logical_plan` boundary with a short TTL
later** — co-located with `unity.resolve`, the natural fresh-fetch seam — without
re-plumbing. v1 = resolve-once-per-session; documented evolution = per-query
lookup against the Engine cache with a TTL.

**Trust rules.** Enrichment keys off the *authenticated* uid; IdP-sourced
attributes override self-asserted header attributes of the same key; group
membership **never** comes from a client header. On `IdentityProvider` error,
**fail closed** (fail the session / deny) — missing facts could under- *or*
over-authorize. An unknown uid returning *empty* enrichment is a success, not an
error, so the anonymous/dev path still works.

## Consequences

- The static bundle is no longer load-bearing for membership; the headline test
  is that `is_allowed` permits via dynamically-resolved membership with an
  **empty** bundle (and denies without enrichment).
- The `Policy` trait signature does **not** change for this concern — enrichment
  is fully captured by the enriched `PrincipalIdentity`, which is already a
  parameter; the group closure reaches Cedar via `to_entities()`, not via
  `EvalContext`. Resource facts and principal facts meet only at the single
  `Entities::from_entities` call.
- Per-session caching trades freshness for hot-path cost in v1; TTL/refresh and
  catalog/attribute-scoped selective fetching are documented future optimizations.
- Agent identity: stable agent identity could enrich the principal, but per-query
  agent claims (`AgentContext` varies per query, [ADR-0005](0005-per-query-agent-governance-context.md))
  stay in the per-query context. The `PrincipalClaims.agent` seam is left in
  place; agent principal-enrichment is deferred to the agent-PEP work.
- A real authentication interceptor (mTLS / validated bearer) upstream of
  `principal_from_metadata` remains future work; the provider keys off whatever
  uid that interceptor establishes (today: the header, advisory).
