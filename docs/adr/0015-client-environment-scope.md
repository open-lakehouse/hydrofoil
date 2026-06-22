# 0015 — Client-side environment is a first-class scope for UI state

> Status: **Accepted** (2026-06). Implemented in `node/ui/src/lib/client/environments.ts`
> (the `ActiveEnvironment` / `EnvironmentCapabilities` types),
> `node/ui/src/components/environment/ActiveEnvironmentContext.tsx` (the provider),
> `node/ui/src/components/EnvironmentGate.tsx` (the switch protocol),
> `node/ui/src/lib/query/resultSessions.ts` (the env-scoped result-session
> registry), and `node/ui/src/lib/query/arrowResultStore.ts` (the `inspect()`
> API). Complements the Monaco worker plan in
> [`docs/monaco-sql-worker-tauri.md`](../monaco-sql-worker-tauri.md).

## Context

The UI (`node/ui`) runs unchanged across deployment shapes — a local Tauri shell
that spawns its own services, and a build served from a web server that reaches
deployed services. The seam is three late-binding registries (fetch, ConnectRPC
transport, environment host) plus a `QueryRunner` registry and a pluggable
catalog provider. `node/ui` imports no Tauri. That foundation is sound and is
kept.

The gap was in **state scope**. Environment switching was a *host-side,
imperative singleton swap*: Rust mutates `AppState.active` on
`select_environment`, and the JS registries are set once at boot. But the
*client-side* state had no notion of "which environment are we in":

- The TanStack Query cache (catalog/schema/table metadata) was global.
- Per-tab `RunController`s (Arrow results) were owned by an
  `EditorSessionContext` ref map, unscoped.
- Added volumes, open tabs, and tree-expansion state were persisted to
  `sessionStorage` under fixed, un-namespaced keys.
- `EnvironmentGate` switched environments by flipping a view flag and re-mounting
  `AppShell` — it never invalidated any of that client state.

So metadata, results, and volumes from environment A bled into environment B
after a switch. `hasHome` was also read imperatively
(`getEnvironmentHost().hasHome`), rather than as a property of the selected
environment, which doesn't generalize as capabilities grow.

Two further needs shaped the design: Arrow result data can be large and we want
to (a) inspect what a store holds and (b) correlate runs to their query file and
environment, with a seam for later cross-session persistence.

## Decision

Make the **selected environment a first-class client-side scope** that env-scoped
state keys off of.

1. **Capability descriptor.** The host's `select()` (and `active()`) resolve to an
   `ActiveEnvironment { id, name, capabilities, volumes }` rather than a bare id.
   `EnvironmentCapabilities` starts with `hasHome` and grows (lineage, credential
   vending, write path) instead of new host booleans. The Tauri host derives this
   from Rust (`active_environment` / `select_environment` now return a descriptor
   with `capabilities.hasHome`); the default web host returns one fixed
   environment with no capabilities and no built-in volumes.

2. **`ActiveEnvironmentProvider`** holds the `ActiveEnvironment` as the client
   source of truth. Env-scoped components read `useActiveEnvironment()` instead of
   calling `getEnvironmentHost()`. The provider takes the environment as a value,
   so a Storybook story can supply a mock with no host.

3. **Switch protocol.** On selecting a different environment, `EnvironmentGate`
   (a) clears the TanStack Query cache (`queryClient.clear()`), and (b) remounts
   the app subtree via `key={env.id}` — which disposes per-tab run controllers
   and the result-session registry and resets component-held volume state. All
   `sessionStorage` keys (added volumes, open tabs, catalog + file-tree
   expansion) are **namespaced by environment id**, so no A→B bleed and returning
   to A restores its own state.

4. **`ArrowResultStore.inspect()`** — an additive, zero-copy summary (schema,
   row/column/batch counts, accumulated `byteLength`). No change to the existing
   batch-array + binary-search `getCell` internals.

5. **Env-scoped `ResultSessionRegistry`** owns the per-tab `RunController`s
   (moved out of `EditorSessionContext`) and correlates each run to its query file
   via `RunMeta { filePath, sql, startedAt, durationMs, info }`. The registry
   shares the editor provider's lifetime, so it is disposed on a switch. It is the
   seam where future cross-session persistence plugs in.

## Consequences

- **Switching environments yields clean, correctly-scoped client state** — no
  metadata/results/volume bleed; per-env sessionStorage restores on return.
- **Capabilities are reactive and extensible** — the UI shells itself from
  `useActiveEnvironment().capabilities` / `.volumes`; adding a capability is a
  field, touched in the host + the consuming component, not a new global getter.
- **Results are inspectable and correlated** — `inspect()` powers a footprint/
  timing chip in `ResultsPane`; `ResultSessionRegistry.list()` enumerates an
  environment's runs.
- **Storybook-ready** — the provider, `DataGrid` (plain props), and the existing
  `fixtureCatalogProvider` mean the key components can be storied with mocks; the
  harness itself is deferred.
- The `hasHome` host getter was **removed** (clean cut, not a shim): the codebase
  is unpublished and prefers no compatibility shims, and the only consumer
  (`editor.lazy.tsx`) migrated in the same change.

## Rejected alternatives / future directions

- **Adopt an off-the-shelf large-Arrow engine (FINOS Perspective, DuckDB-Wasm).**
  Both genuinely manage large Arrow state, but each replaces the *whole*
  query-result + grid stack: data lives in the WASM heap (not JS `apache-arrow`
  objects), Perspective ships its own virtualized grid, and adopting either means
  giving up the zero-copy `getCell` path, the TanStack-Virtual DataGrid, and the
  cell formatters, plus a multi-MB WASM dependency and inverted data ownership.
  We keep building on `apache-arrow` directly; revisit only if we want an
  off-the-shelf high-end analytical grid.
- **Namespace query keys by env id to *retain* per-env caches** (instead of
  clearing on switch). Simpler-correct now is to clear; key-namespacing is a
  later optimization once retaining A's cache while in B is worth the complexity.
- **Host-side result virtualization for "ludicrous size" results** — stream/window
  results from the host rather than holding everything client-side. A natural
  future extension of the `ResultSessionRegistry` seam (host backs the session,
  client windows it); dovetails with cross-session persistence.
