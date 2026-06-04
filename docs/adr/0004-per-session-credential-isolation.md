# 0004 — Per-session `RuntimeEnv` for credential isolation

> Status: **Accepted** (2026-06), with a documented follow-up. Implemented in
> `crates/hydrofoil/src/engine.rs` and `crates/hydrofoil/src/session.rs`
> (`create_session_for`). Refines
> [`docs/session-management.md`](../session-management.md) and
> [`docs/platform-policy-architecture.md`](../platform-policy-architecture.md).

## Context

Unity Catalog credential vending registers a vended `ObjectStore` — with
short-lived, scoped credentials baked in — on the DataFusion `RuntimeEnv`
object-store registry, **keyed by table URL**
(`UnityCatalogProviderList` → `register_object_store`, reached via
`build_unity_resolver`, which today passes the session's `runtime_env()`).

The natural way to build per-connection sessions is to fork a shared base via
`SessionStateBuilder::new_from_existing` (as Spice.ai/IOx do). But
`new_from_existing` **`Arc`-shares the base `RuntimeEnv`** — and therefore its
object-store registry. If all sessions shared one runtime, principal A's vended
(possibly elevated) store for `s3://bucket/t1/` would be resolvable by principal
B's queries: a privilege-escalation leak.

The previous code avoided this only *by accident* — each principal happened to
get a separate `SessionContext` (hence a separate `RuntimeEnv`).

Separately: vending today uses a single shared `UC_TOKEN`, and
`UnityObjectStoreFactory::for_table(table, op)` takes **no principal** — so UC
cannot scope credentials to the requesting user. UC-side authorization is the
current enforcement.

## Decision

- **Each `Session` owns its own `RuntimeEnv` / object-store registry**, made
  explicit rather than accidental. `Engine::new_session` builds a fresh
  `SessionContext` via `create_session_for(...)` (which constructs a new
  `RuntimeEnv` and registers the seaweedfs store) and binds a Unity Catalog
  resolver to *that* session's runtime. We deliberately do **not** fork sessions
  from a shared base runtime.
- Vended credentials live on the session's registry and die with the session
  (idle TTL / close).
- **Per-user vending is deferred.** This ADR fixes the isolation *boundary*;
  threading the authenticated request identity/token into `for_table` so UC
  vends per-principal credentials is the next step (likely a
  `unitycatalog-object-store` factory API change / fork).

## Consequences

- No vended credential can cross principals; a unit test
  (`engine::tests::sessions_have_isolated_runtimes`) asserts distinct sessions
  have distinct `RuntimeEnv`s.
- Building a fresh per-session `SessionContext` is heavier than a
  `new_from_existing` fork. This is the correct safety boundary and matches the
  prior per-principal `create_session` cost; catalog/UDF/rule setup is cheap, and
  the per-session object-store registration is unavoidable for isolation.
- Until per-user vending lands, do **not** mistake this for enforced per-user
  credential scoping — UC still vends with a shared token; the per-session
  registry is the seam that makes per-user vending a later drop-in.
