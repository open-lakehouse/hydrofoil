# 0002 — Flight SQL session identity via handshake + Bearer/cookie

> Status: **Accepted** (2026-06). Implemented in `crates/hydrofoil/src/server.rs`
> (`do_handshake`, `resolve_session`, `session_id_from_metadata`). Refines
> [`docs/session-management.md`](../session-management.md).

## Context

To correlate the RPCs of one logical operation (and to host per-connection
state), the server needs a stable, protocol-derived session id — replacing the
old per-principal context cache. Flight SQL (arrow-flight 58.3) exposes
`do_handshake` but no built-in cookie/session plumbing; the spec says sessions
are persisted "using an implementation-defined mechanism, typically RFC 6265
cookies," and the reference clients (ADBC) echo an `authorization: Bearer` token
returned from the handshake.

Many demo clients (DuckDB, plain pyarrow) never handshake at all, and must keep
working.

## Decision

- **Mint** a session id (`Uuid`) in `do_handshake`, bind the authenticated
  principal to it, and return the id to the client three ways: as the handshake
  response payload, as `authorization: Bearer <id>`, and as an `x-session-id`
  metadata header. ADBC echoes the Bearer automatically; cookie- and
  header-based clients are covered too.
- **Resolve** the session per RPC from (in order) `x-session-id`, the `cookie`
  header (`session_id=…`), or `authorization: Bearer <id>`.
- **Ephemeral fallback:** when no (or an unknown/expired) session id is present,
  resolve the principal's *stable* ephemeral session, keyed
  `ephemeral:{principal_uid}`. This preserves statement-handle continuity across
  the two RPCs of one query for no-handshake clients, matching prior behaviour.

## Consequences

- Handshake clients get true per-connection sessions with isolated state;
  no-handshake clients keep working via the ephemeral path.
- **`Bearer`-as-session-id is a dev shortcut, not authentication.** The metadata
  is the transport seam, not the trust boundary (see the trust-boundary note in
  `crates/hydrofoil/src/identity.rs`). When a real auth interceptor (mTLS /
  validated bearer token) lands, session identity must be split from the auth
  token.
- Sessions are swept on an idle TTL (see
  [ADR-0004](0004-per-session-credential-isolation.md) for why their teardown
  matters); ephemeral sessions are subject to the same TTL.
- OpenLineage defines no standard parent-run header, so we keep our own
  `x-openlineage-parent-*` metadata keys (documented in `session-management.md`).
