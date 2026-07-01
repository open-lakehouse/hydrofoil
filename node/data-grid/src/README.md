# `@open-lakehouse/data-grid`

The reusable, virtualized, Arrow-backed table primitive. A **leaf** package: a
lower-level building block other features depend on, which itself depends on
nothing application-specific (only Arrow, TanStack, and the shared
`@open-lakehouse/ui-kit`).

## Public surface

Import only from the package root — `@open-lakehouse/data-grid`:

| Export | Kind | Purpose |
| --- | --- | --- |
| `DataGrid` | component | Virtualized, sortable grid that renders an `ArrowResultStore`. Props: `{ store, version, running, className? }`. |
| `ArrowResultStore` | class | The grid's input contract. Consumers build one (from Arrow IPC bytes, query results, or fixtures) and pass it as `store`. |
| `ArrowStoreInfo` | type | Cheap read-only summary of a store's contents. |

Everything else — `data-grid-cell`, `data-grid-header`, the Arrow-support
modules under `lib/` (`arrowResultStore`, `useArrowTable`, `cellFormatters`,
`arrowTypeLabel`, `sortValues`), and `story-fixtures` — is internal; the package
`exports` map only exposes the root barrel, so it cannot be imported from
outside.

## Consumers

- `@open-lakehouse/ui`: the SQL editor result pane (`components/editor/ResultsPane`),
  the import preview (`routes/import.lazy`), and the editor stream controller
  (`lib/editor/runController`, which builds the `ArrowResultStore`).
- The app's fixture world (`lib/fixtures/arrow`) builds stores from IPC for its
  Storybook fakes — note the direction: the fixtures depend on this package, not
  the reverse, which is why the package ships its own `story-fixtures` for its
  own stories.

## Boundary rule (enforced)

`data-grid` is a leaf: a Biome `noRestrictedImports` rule in `node/biome.json`
forbids it from importing `@open-lakehouse/unity-catalog`. The dependency
direction is the invariant: the Unity Catalog package may build on `data-grid`;
`data-grid` never builds on it. The "barrel-only" guarantee is now native — the
package `exports` map reaches only `src/index.ts`.

## Scope note

The grid is intentionally Arrow-specific — its input is `ArrowResultStore`.
Generalizing it beyond Arrow (and porting UC's `StorageTable` onto it) is a
separate, behavior-changing refactor tracked in
`docs/portable-uc-components.md`, not part of this package's contract.
