# S09 — hydrofoil: authentication interceptor (design, then implement)

| | |
|---|---|
| Target repo | `open-lakehouse` (crates/hydrofoil) |
| Depends on | S08 (identity fixes; avoids conflicts in identity.rs/engine.rs) |
| Scope | Design doc/ADR update + one implementation PR |
| Findings | B2 (critical) |

## Mission

You are working in `open-lakehouse`. Hydrofoil currently has **no authentication**:
the Cedar principal is derived directly from a client-supplied header.

- `crates/hydrofoil/src/identity.rs:73-131` — `principal_from_metadata` /
  `principal_from_http_headers` read `x-hydrofoil-principal` from gRPC metadata /
  HTTP headers and parse it into the Cedar principal. Any client can send
  `x-hydrofoil-principal: User::"admin"` and assume that identity; group membership
  is then IdP-resolved *for the asserted uid*. The module doc explicitly defers an
  authenticating interceptor.
- `crates/hydrofoil/src/server.rs:66-85, 174-184` — sessions are additionally
  resolvable via `x-session-id` / cookie / `authorization: Bearer` carrying a random
  v4 session UUID; a session stays bound to its creation principal (good), but
  nothing ties the session id to a transport identity, so a leaked id is a full
  bearer credential.
- Advisory `role`/`region` headers are folded as attributes and IdP-overridden — the
  spoofable part is the **uid itself**, which is authoritative.

This is the known-deferred authn gap; this session designs and lands it. Because
access control currently rests *entirely* on Cedar (UC credentials are vended with
the server's token — see ADR 0011), identity spoofing equals full data access.

## Approach

**Design first.** Before writing code, survey prior art for Arrow Flight SQL
authentication (the Flight handshake/bearer-token pattern used by arrow-flight's
middleware, how Ballista/Dremio/InfluxDB IOx handle it) and pick the mechanism(s) to
support. Record the decision as a new ADR in `docs/adr/` (Nygard/MADR-lite: Title,
Status, Context, Decision, Consequences), cross-referencing ADR 0002 (Flight SQL
session identity), ADR 0008 (principal resolution), and ADR 0011 (server-token
credential vending). Keep the design grounded in what existing integrations expect —
clients in this stack include `notebooks/duckdb_flight.py` (duckdb/ADBC) and the
marimo notebooks.

Recommended shape (validate during design):

1. A tonic interceptor (Flight) and axum middleware (HTTP `/query`) that validate a
   bearer token (OIDC/JWT verification against a configurable issuer, or a static
   token map for dev) or an mTLS client-cert subject, and inject the **verified**
   subject into request extensions.
2. `principal_from_*` derives the uid from the verified subject only.
   `x-hydrofoil-principal` is at most a *requested* identity that must match the
   verified subject (or be permitted by an explicit impersonation policy) — otherwise
   reject with `permission_denied`.
3. Session reuse: on each RPC carrying a session id, verify the session's bound
   principal matches the authenticated caller; reject otherwise. This closes the
   leaked-session-id hole.
4. A dev mode (`auth.mode = "insecure-headers"` or similar, default OFF) preserving
   today's behavior for local stacks, with the loud startup warning from S08 wired to
   it.
5. Config under the existing hydrofoil TOML config (`environments/config/*/
   hydrofoil.toml`) — add the auth section to both local and live configs.

## Constraints

- Do not weaken the existing invariant that a session stays bound to its creation
  principal.
- Crates are unpublished: change APIs freely, no compatibility shims.
- Stage your changes and propose a commit message, but do **not** run `git commit` —
  the user signs commits. Attribute AI work as "AI assisted by Isaac" if attribution
  is included.

## Verification

- Tests: (1) request with no/invalid token rejected (`unauthenticated`); (2) valid
  token → principal derived from verified claims, spoofed `x-hydrofoil-principal`
  mismatching the subject rejected; (3) session created by principal A, replayed
  with principal B's token → rejected; (4) dev mode preserves current behavior and
  logs the warning.
- `cargo test -p hydrofoil`, `cargo clippy -p hydrofoil` clean.
- Live-stack smoke (`environments/`, `just`): duckdb/ADBC notebook flow works with a
  configured token.
