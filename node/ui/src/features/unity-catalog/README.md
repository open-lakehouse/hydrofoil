# `unity-catalog`

The Unity Catalog feature module: the catalog browser, per-entity detail panes,
the create/edit/delete and storage dialogs, the metastore storage admin table,
and the React-Query data layer that talks to the Unity Catalog REST API.

## Public surface

Import only from the barrel — `@/features/unity-catalog`:

| Export | Used by |
| --- | --- |
| `UnityCatalogProvider`, `useUnityCatalog` | `main.tsx` (client injection) |
| `CatalogExplorer` | `routes/catalog.lazy` (route-level browser) |
| `CatalogDialogsProvider` | `environment/manager/EnvironmentManager` |
| `StorageTable`, `StorageLocationPicker`, type `StorageKind` | `environment/manager/EnvironmentDetail` (StorageTable); StorageLocationPicker is exported for future consumers |
| `Meta`, `MetaGrid` | `environment/manager/EnvironmentDetail` |
| `ListStates`, `TreeRow` | `editor/fileTree/FileTree` |
| `useCatalogs`, `useSchemas`, `useVolumes`, `useCredentials`, `useExternalLocations`, `prefetchCatalogs` | `editor/AddVolumeDialog`, `routes/import.lazy`, `routeTree` |
| `invalidateTables` | `routes/import.lazy` |
| `parseUcError` | `EnvironmentGate`, `ErrorBoundary` |

Everything else — the tree internals (`CatalogTree`, `RowMenu`, `DetailPane`,
`selection`, `ExpansionContext`, `groups`, `dialog-types`), the detail panes
under `detail/`, the dialog wiring (`dialogs`, the `*EntityDialog`s, the storage
dialogs under `storage/`), and the `uc/` data layer internals — is
module-internal and must not be imported from outside.

## Client injection

`uc/queries.ts` and `uc/mutations.ts` read their client from `useUnityCatalog()`
(provided by `UnityCatalogProvider`) rather than a module singleton, so the host
decides base URL / transport / auth. This is the seam a future proto-generated
WASM client swaps into with no hook changes (see
`docs/portable-uc-components.md`).

Every fetch — list reads (`useCatalogs`, …), mutations, AND detail reads
(`useCatalogDetail`, … and the storage dialogs) — routes through the injected
client. The `*DetailQuery(id)` functions and `prefetch*` helpers deliberately
bind the *default* client because they only derive query keys / warm caches;
key derivation is client-independent, so the keys they produce match the
injected-client hooks and caches stay aligned. `mutations.ts` reads
`*DetailQuery(id).queryKey` for `setQueryData` / `removeQueries` — a key, not a
fetch — which is why those stay on the default client.

## External dependencies

The module depends only on shared primitives and one host edge:

- `@/components/ui/*` — shadcn primitives (shared).
- `@/components/forms/*` — the generic JSON-schema form renderer (`SchemaForm` +
  `schemas`), a shared form primitive used by the entity/storage dialogs.
- `@/lib/utils` — `cn`.
- `@/lib/api` — the generic Unity Catalog client factory the data layer injects.
- **`./env-seam` — the single host edge.** `ExpansionContext` namespaces its
  persisted tree-expansion state per active environment, so it needs the current
  environment id. That is the ONE outbound dependency from core that is not
  itself UC; it is isolated in `env-seam.ts` (which wraps
  `@/components/environment/ActiveEnvironmentContext`). To extract the module,
  replace that file's import with whatever supplies the embedder's scope id — and
  nothing in core changes.

## Boundary guard (enforced)

A Biome `noRestrictedImports` rule in `node/biome.json` forbids importing the
module's internal paths (`@/features/unity-catalog/**`) from outside it; only the
barrel `@/features/unity-catalog` is allowed.

## Scope notes

- `StorageTable` is UC-specific — it hand-rolls its own `<table>` bound to UC
  types/hooks/detail panes. It is **not** the reusable `data-grid` primitive
  (a separate sibling module) and does not use it. Porting it onto a generic
  table is a separate, behavior-changing refactor (see the strategy doc).
- Dependency direction: this module may build on `data-grid` in the future;
  `data-grid` never builds on it.
- `selection.ts` / `ExpansionContext` read TanStack Router and `sessionStorage`
  directly; making them prop/callback-driven is a separate headless-ification
  tracked in the strategy doc.
