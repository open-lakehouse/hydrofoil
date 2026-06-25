# `data-grid`

The repo's reusable, virtualized, Arrow-backed table primitive. A **leaf**
module: a lower-level building block other features depend on, which itself
depends on nothing application-specific.

## Public surface

Import only from the barrel — `@/features/data-grid`:

| Export | Kind | Purpose |
| --- | --- | --- |
| `DataGrid` | component | Virtualized, sortable grid that renders an `ArrowResultStore`. Props: `{ store, version, running, className? }`. |
| `ArrowResultStore` | class | The grid's input contract. Consumers build one (from Arrow IPC bytes, query results, or fixtures) and pass it as `store`. |
| `ArrowStoreInfo` | type | Cheap read-only summary of a store's contents. |

Everything else — `data-grid-cell`, `data-grid-header`, and the Arrow-support
modules under `lib/` (`arrowResultStore`, `useArrowTable`, `cellFormatters`,
`arrowTypeLabel`, `sortValues`) — is internal and must not be imported from
outside the module.

## Consumers

- `components/editor/ResultsPane` — the SQL editor result pane.
- `routes/import.lazy` — the import preview.
- `lib/editor/runController` — builds the `ArrowResultStore` the editor streams into.
- `lib/fixtures/arrow` + `data-grid.stories` — fixtures and Storybook.

## Boundary rules (enforced)

Two Biome `noRestrictedImports` rules in `node/biome.json` keep the boundary honest:

1. **Barrel-only** — no file outside `data-grid` may import its internals
   (`@/features/data-grid/**`); only the barrel `@/features/data-grid` is allowed.
2. **Leaf rule** — `data-grid` must not import Unity Catalog (`@/lib/uc/**`,
   `@/features/unity-catalog`) or any app-feature code (`@/components/catalog`,
   `storage`, `editor`, `environment`, `@/routes`). Allowed dependencies:
   `apache-arrow`, `@tanstack/*`, the shadcn primitives in `@/components/ui/*`,
   and `@/lib/utils`.

The dependency direction is the invariant: the Unity Catalog module may build on
`data-grid` in the future; `data-grid` never builds on it.

## Scope note

The grid is intentionally Arrow-specific — its input is `ArrowResultStore`.
Generalizing it beyond Arrow (and porting `StorageTable` onto it) is a separate,
behavior-changing refactor tracked in `docs/portable-uc-components.md`, not part
of this module's contract.
