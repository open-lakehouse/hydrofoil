# S08 — hydrofoil: identity & policy facts (ACL case-folding, enrichment TTL)

| | |
|---|---|
| Target repo | `open-lakehouse` (crates/hydrofoil) |
| Depends on | — |
| Scope | One PR |
| Findings | B3 (major), B4 (major), B10/B2-adjacent minors |

## Mission

You are working in `open-lakehouse`. Hydrofoil authorizes queries with Cedar; the
policy facts come from Unity Catalog table metadata (owner, readers, writers, tags —
gathered in `crates/hydrofoil/src/catalog/tags.rs`) and from principal enrichment
(group membership via an IdP, cached in `crates/hydrofoil/src/engine.rs`). Cedar
entity uids are case-sensitive. This session fixes fact fidelity and staleness.

## Findings to fix

### B3 [major] Reader/writer ACL uids are lower-cased

- `crates/hydrofoil/src/catalog/tags.rs:102-121` — `table_acl_facts` builds the
  `readers`/`writers` fact sets via `split_tags`, which does
  `.trim().to_ascii_lowercase()`. The values are Cedar principal/group uids (e.g.
  `User::"Alice"`, `UserGroup::"Readers"`), so `User::"Alice"` becomes
  `user::"alice"` and a policy like
  `when { resource.readers.contains("User::\"Alice\"") }` never matches — silently
  denying authorized readers. It is also inconsistent: `owner` is passed verbatim
  (`table.owner.clone()`), so owner and reader/writer identities are normalized
  differently.

**Fix:** add a dedicated `split_principals` (trim, drop empties, **no** case-folding)
and use it for readers/writers; keep `split_tags`' lower-casing for classification
tags only. Add a test with mixed-case uids asserting the fact values are verbatim and
that owner/readers normalization now matches.

### B4 [major] Principal-enrichment cache never expires

- `crates/hydrofoil/src/engine.rs:82` and `:133-149` — `Engine::enrich` caches
  `PrincipalEnrichment` (group membership, attributes) in a `DashMap` keyed by uid,
  forever. A user removed from a group keeps the cached membership until process
  restart — fail-stale over-authorization for a policy information point. The code
  comments anticipate a TTL; none exists.

**Fix:** store `(enrichment, resolved_at: Instant)` and re-resolve past a
configurable TTL (config entry on the engine/identity config, sensible default e.g.
5 minutes). On re-resolve failure, prefer fail-closed-ish behavior: keep serving the
stale entry only within a bounded grace period and log loudly, or deny enrichment-
dependent requests — pick one, document it in the config doc, and test it. Relates to
ADR 0008 (principal/identity resolution) — update that ADR's freshness discussion if
your choice refines it.

### Minors (same area)

1. **Ephemeral-session attribute pinning** — `crates/hydrofoil/src/engine.rs:434-445`
   + `crates/hydrofoil/src/identity.rs:124-129`: `ephemeral_for` keys sessions by
   `ephemeral:{uid}` and binds the *first* request's advisory `role`/`region` headers;
   later requests from the same uid with different attributes reuse the first
   session's principal. Derive per-query principal attributes per request, or key
   ephemeral sessions by `(uid, attribute-hash)`.
2. **Silent wide-open default** — missing principal header falls back to
   `User::"anonymous"` (`identity.rs:118`) and a missing `policy.oci_ref` installs an
   allow-all `StaticPolicy` (`main.rs:69`). Both are intentional ("ungoverned
   default"), but the combination means an unconfigured server is wide open with no
   signal. Log a prominent startup warning when no policy is configured AND no authn
   interceptor is wired (see S09).

## Constraints

- Crates are unpublished: change APIs freely, no compatibility shims.
- Stage your changes and propose a commit message, but do **not** run `git commit` —
  the user signs commits. Attribute AI work as "AI assisted by Isaac" if attribution
  is included.

## Verification

- `cargo test -p hydrofoil` with new tests: mixed-case ACL uids fact test; enrichment
  TTL expiry test (inject a clock or use a tiny TTL); ephemeral-session attribute
  test.
- `cargo clippy -p hydrofoil` clean.
