# 0011 — UC credential vending with the server token; Cedar as the sole access control

> Status: **Accepted** (2026-06) — documents a current architectural property and the
> condition for revisiting it. Relates to
> [`0004-per-session-credential-isolation.md`](0004-per-session-credential-isolation.md),
> [`0008-principal-identity-resolution.md`](0008-principal-identity-resolution.md), and
> the June 2026 service review
> ([finding B9](../reviews/2026-06-service-review/README.md)).

## Context

Hydrofoil resolves Unity Catalog tables and vends temporary storage credentials
through a single `UnityObjectStoreFactory`, constructed once at startup from the
server's configured UC token (`crates/hydrofoil/src/main.rs:78-93`) and shared by all
sessions. The requesting user's identity is never passed to UC credential vending:
every principal's queries read and write storage with the **server's** UC
permissions.

Consequences of this design:

- **Unity Catalog's own ACLs are not enforced per principal.** UC sees one caller —
  the hydrofoil service account.
- **Cedar is the sole access control.** The per-session `RuntimeEnv` isolation
  (ADR 0004) prevents credential *leakage between sessions*, but every session's
  credentials are equally privileged; what a principal may query is decided entirely
  by the Cedar policy gate during planning.
- Any bypass of the Cedar gate is therefore a bypass of *all* access control. The
  June 2026 review found two such weaknesses (subquery-scan gate coverage, and the
  absent authentication of the principal header) — tracked as remediation sessions
  S01 and S09 respectively.

The alternative — vending UC credentials per principal — requires an authenticated
user identity (or user token) to forward to UC, which does not exist until the authn
interceptor (review session S09) lands. It also requires API surface in
`unitycatalog-rs` (`UnityObjectStoreFactory::for_table` taking a principal/token) and
UC-side accounts/grants for the principals hydrofoil serves.

## Decision

- **Accept server-token vending for now**, with Cedar as the single authorization
  layer, and state this property explicitly rather than implying defense in depth
  that does not exist.
- **Precondition for revisiting:** once the authentication interceptor (S09) provides
  a verified per-request identity, thread it into UC credential vending —
  `unitycatalog-rs` grows a per-principal vending seam on
  `UnityObjectStoreFactory::for_table`, hydrofoil passes the authenticated identity —
  so UC ACLs become defense in depth behind Cedar.
- Until then, deployments must treat the hydrofoil UC token's privileges as the upper
  bound of what *any* connecting principal can reach if policy enforcement fails, and
  scope that token accordingly.

## Consequences

- Operators get an honest statement of the trust model: hydrofoil is a policy
  enforcement point in front of a single privileged catalog identity.
- The Cedar gate's correctness is load-bearing; gate-coverage fixes (review S01) and
  authentication (review S09) are prioritized accordingly.
- Per-principal vending is deferred work with a named trigger (S09), not an implied
  property — preventing the misreading that UC ACLs currently constrain hydrofoil
  users.
