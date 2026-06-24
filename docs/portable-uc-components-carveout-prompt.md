# Carve-out task: two self-contained in-repo components (data grid + Unity Catalog)

> A ready-to-issue prompt for an implementation session, plus the reasoning and
> prototype findings behind it. The companion strategy doc is
> [`portable-uc-components.md`](./portable-uc-components.md); read it first for
> the "why". This file is the "what to do now" for the **in-repo** step.

## The task, in one paragraph

Carve **two** self-contained component modules out of `node/ui`, each behind a
single public entry point, so everything else references them *only* through that
boundary — the shape we'd want if each were an external package, but staying
in-repo for now:

1. **`data-grid`** — the virtualized, Arrow-backed table primitive
   (`components/data-grid/` + its `lib/query` Arrow-support modules). A
   **lower-level** building block with **no Unity Catalog dependency**. Both the
   main application (the SQL editor's result pane, the import preview) and the UC
   component (eventually) depend on it.
2. **Unity Catalog** — the catalog tree, detail panes, entity/storage dialogs,
   the storage admin table, and the `lib/uc` data layer. Depends on `data-grid`
   in the future, never the reverse.

Do **not** change runtime behavior, swap the API client, generalize the grid
beyond Arrow, or wait on anything external. This is pure restructuring + import
rewrites with a behavior-preserving acceptance test (app + Storybook render
identically).

The dependency direction is the whole point: `data-grid` is the leaf both others
build on, so it must not import UC or app code; UC may build on `data-grid` but
not vice-versa.

## Why now, and why in-repo

The end goal (separate doc) is a portable UC component other projects pull in,
backed by a WASM client generated in the `mangrove` repo. That WASM client is
being built separately and we are **not** blocking on it. What we *can* do
independently — and what makes the eventual extraction mechanical — is establish
the **module boundary inside this repo first**: one directory, one barrel, no
inbound reach-around imports. Once the boundary holds, swapping the client (the
prototype below proves that seam) and lifting the directory into a package are
both small, isolated follow-ups.

This is deliberately the low-risk half: it's a pure restructuring with a
behavior-preserving acceptance test (the app and Storybook render identically).

## Prototype findings (already on this branch — build on them, don't redo)

A behavior-preserving dependency inversion is already implemented and verified
(`tsc` clean, 20/20 tests pass, touched files lint clean):

- **`lib/api.ts`** now exposes `createUnityCatalogClient(opts)` returning a
  `UnityCatalogClient` (`{ $api, fetchClient }`), plus `defaultUnityCatalogClient`
  and back-compat `$api` / `fetchClient` exports (the old singleton is now the
  default instance — identical config).
- **`lib/uc/context.tsx`** (new) provides `UnityCatalogProvider` +
  `useUnityCatalog()`, falling back to the default client when no provider is
  mounted (so tests/stories need no wrapper).
- **`lib/uc/queries.ts` and `mutations.ts`** — every list/mutation **hook** now
  reads `$api` from `useUnityCatalog()` instead of the module singleton. The
  non-React helpers (`*DetailQuery`, `prefetch*`) intentionally keep the default
  client; query-key derivation is client-independent so caches still align.
- **`main.tsx`** mounts `<UnityCatalogProvider>` with the default client.

**Implication for this task:** the client is already injectable, so the carve-out
does not need to touch transport at all. The injected-client work is done; what
remains is *physical and structural* consolidation. Leave the prototype's seam
intact — the future WASM swap depends on it.

**Known follow-up, out of scope here:** detail fetches still bake the default
client's `fetchClient` into their queryFn via `*DetailQuery` (used by
`TableDetail` etc. through `useQuery(tableDetailQuery(...))`). Converting those to
hooks so detail fetches also route through the injected client is a separate
step; do not do it as part of the carve-out.

---

# Component A — `data-grid` (the lower-level primitive)

Do this one **first**: it's smaller, has no cycle, and UC's eventual dependency
on it means the leaf should exist before UC is reshaped.

## What it is, and why it's a clean leaf

`components/data-grid/` is a virtualized, sortable table that renders **Arrow
query results**. It was checked end-to-end:

- Files: `data-grid.tsx`, `data-grid-cell.tsx`, `data-grid-header.tsx`,
  `data-grid.stories.tsx`.
- It depends on four `lib/query` modules — `arrowResultStore.ts`,
  `useArrowTable.ts`, `cellFormatters.tsx`, `arrowTypeLabel.ts` — plus
  `sortValues.ts` (pulled in transitively by `useArrowTable`). These are the
  Arrow result-shaping support; they depend only on each other, `apache-arrow`,
  and `@tanstack/react-table`/`react-virtual`.
- **It does NOT touch query execution.** None of those modules import
  `lib/query/runner.ts`, ConnectRPC, or `gen/hydrofoil/*`. Display of Arrow
  results is already cleanly separated from running queries. So the grid + its
  Arrow-support modules form a self-contained unit with **zero UC dependency and
  zero query-engine dependency**.

## Its consumers (all main-app, none UC)

- `components/editor/ResultsPane.tsx` — the SQL editor result pane
  (`<DataGrid store={…} version={…} running={…} />`).
- `routes/import.lazy.tsx` — the import preview.
- `lib/fixtures/arrow.ts` and the Storybook story feed it fixtures.

UC does not consume it today; it will once `StorageTable` is refactored onto it
(future cleanup, see the strategy doc — NOT this task).

## What to build (Component A)

1. **Make it a root-level isolated component.** Create one module directory
   (suggest `src/components/data-grid/` stays as the home, OR a clearer root like
   `src/data-grid/` — match whatever root convention Component B uses so the two
   are siblings). Move into it the four/five Arrow-support modules it owns
   (`arrowResultStore`, `useArrowTable`, `cellFormatters`, `arrowTypeLabel`,
   `sortValues`) so the grid and its input contract live together. The unit's
   public input stays the existing `ArrowResultStore` contract — **do not
   generalize the grid away from Arrow**; that's a separate refactor.
2. **One public barrel** (`index.ts`) exporting `DataGrid`, the
   `ArrowResultStore` type/constructor its consumers build, and any types in the
   `<DataGrid>` props (e.g. `ArrowColumnMeta`). Internal cell/header components
   stay unexported.
3. **The leaf rule — enforce it.** This module must import **nothing** from UC
   (`lib/uc/*`, the UC component) and nothing from app feature dirs. Allowed
   dependencies: `apache-arrow`, `@tanstack/*`, `@/components/ui/*` (shadcn
   primitives), `@/lib/utils`. If `lib/fixtures/arrow.ts` is only used by the
   story, move it inside the module's test/story fixtures; if it's shared, keep
   it out and have the story import it.
4. **Rewrite consumers** (`ResultsPane`, `import.lazy`, the story) to import from
   the barrel only.
5. **Add the boundary guard** (same mechanism as Component B, below): forbid deep
   imports into `data-grid` internals from outside, and forbid `data-grid` from
   importing UC/app code (a `noRestrictedImports`-style rule, or a README note if
   that's heavier than the repo wants).

Acceptance for A: `tsc` clean, tests green, no new lint; `ResultsPane` and the
import preview render identically; the grid module imports no UC/app code; story
still renders.

---

# Component B — Unity Catalog

## The actual coupling (what makes this non-trivial)

A grep of the current tree shows the UC feature is not one directory — it's three
loosely-defined groups with a **dependency cycle** across the boundary:

1. **Core** — `components/catalog/**` (tree, detail panes, `selection.ts`,
   `groups.tsx`, `dialog-types.ts`, `ExpansionContext.tsx`) and `lib/uc/**`
   (`queries`, `mutations`, `context`, `errors`).

2. **Entity dialogs living *outside* the catalog dir but logically part of it** —
   `components/CreateEntityDialog.tsx`, `EditEntityDialog.tsx`,
   `DeleteEntityDialog.tsx`. These are the cycle:
   - `components/catalog/dialogs.tsx` imports them from `@/components/*`, **and**
   - each `*EntityDialog` imports back into `@/components/catalog/dialog-types`,
     `@/components/catalog/selection`, `@/lib/uc/mutations`, `@/lib/uc/errors`.
   So the boundary cuts straight through a cycle. These dialogs must move *inside*
   the module.

3. **Storage components — all UC-internal (decided).** `components/storage/`
   (`StorageTable.tsx`, `CredentialDialog.tsx`, `ExternalLocationDialog.tsx`,
   `StorageLocationPicker.tsx`) display and manage metastore-level UC securables
   (credentials, external locations). They all consume `lib/uc/*`, and the first
   three are part of the dialog cycle (imported by `catalog/dialogs.tsx`).
   `StorageTable` in particular reaches deep into the module — it imports
   `catalog/detail/{CredentialDetail,ExternalLocationDetail}`, `catalog/dialogs`,
   `RowMenu`, and `catalog/types`; its own header comment calls it "a presentation
   change over existing plumbing." **Move the entire `storage/` group inside the
   module.** Two of these have external consumers, so they go in the public barrel
   (see #4): `StorageTable` (used by `environment/manager/EnvironmentDetail.tsx`)
   and `StorageLocationPicker` (used by `CreateEntityDialog`, which is itself
   moving inside).

   **`StorageTable` is UC-specific, not a reusable low-level primitive — do not
   confuse it with `components/data-grid/`.** It was checked: `StorageTable`
   references no Arrow / query-result plumbing (`lib/query/*`,
   `ArrowResultStore`, `react-virtual`, `react-table`) and does not use the
   generic `data-grid` component — it hand-rolls a `<table>` bound to UC types
   (`CredentialInfo`/`ExternalLocationInfo`), UC hooks
   (`useCredentials`/`useExternalLocations`), and UC detail/dialog internals.
   Stripped of UC, nothing reusable remains. So it belongs inside UC, full stop.
   The genuine reusable table primitive is `data-grid` (Component A) — a separate
   sibling root, not part of the UC module. **Do not attempt to merge
   `StorageTable` into `data-grid` or refactor `StorageTable` onto it during this
   work** — `StorageTable` keeps its own hand-rolled `<table>` for now; porting it
   onto `data-grid` is a separate, behavior-changing refactor (noted in the
   strategy doc as future cleanup), out of scope here. UC's *eventual* dependency
   on `data-grid` is exactly why A is carved first, but UC does not gain that
   dependency in this task.

4. **Genuine external consumers (one-directional — these define the public API
   surface you must keep working):**
   - `components/editor/AddVolumeDialog.tsx` → `useCatalogs, useSchemas, useVolumes`
   - `routes/import.lazy.tsx` → `useCatalogs, useSchemas, invalidateTables`
   - `routes/catalog.lazy.tsx` → the `CatalogExplorer` component
   - `environment/manager/EnvironmentDetail.tsx` → `StorageTable` (renders both
     `kind="external_location"` and `kind="credential"`)
   - `main.tsx`, `routeTree.tsx` → provider / route wiring
   - `ExpansionContext.tsx` reaches *out* to
     `@/components/environment/ActiveEnvironmentContext` — this is the one outbound
     edge from core that is NOT UC; keep it as an injected/optional dependency
     rather than pulling environment code into the module.

The whole feature's only shared-leaf dependencies are `@/components/ui/*`
(shadcn primitives) and `@/lib/utils` — those stay shared, not absorbed.

## What to build (Component B)

1. **Pick a module root** (suggest `src/features/unity-catalog/`, or
   `src/uc/` — match repo convention; there is no `features/` dir today, so
   confirm; make it a sibling of the `data-grid` root from Component A). Move
   into it:
   - all of `components/catalog/**`,
   - the three `*EntityDialog` components,
   - all of `components/storage/**` (see #3 — decided UC-internal),
   - `lib/uc/**`.
   Keep `lib/api.ts` where it is (it's the generic client factory, not UC-feature
   internals) — the module *depends on* it.

2. **Define one public entry point** — a top-level `index.ts` barrel that
   re-exports exactly what external consumers need and nothing else:
   - `CatalogExplorer` (the route-level component),
   - `StorageTable` (used by `EnvironmentDetail`) and `StorageLocationPicker`
     (used by `CreateEntityDialog` once it's internal — keep it exported in case
     other consumers appear; verify),
   - the `UnityCatalogProvider` / `useUnityCatalog` (re-export or relocate),
   - the read hooks external consumers use (`useCatalogs`, `useSchemas`,
     `useVolumes`, `useExternalLocations`, `useCredentials`),
   - the invalidators they use (`invalidateTables`, …),
   - any types those signatures expose (e.g. `StorageKind` for `StorageTable`).
   Everything else (tree internals, detail panes, dialog wiring, `selection`,
   `ExpansionContext`, the `*EntityDialog`s, the storage dialogs) stays
   **module-internal** — not in the barrel.

3. **Rewrite the boundary imports.** External consumers
   (`editor/AddVolumeDialog`, `routes/*`, `environment/manager/EnvironmentDetail`,
   `main.tsx`, `routeTree.tsx`) import from the barrel only
   (e.g. `@/features/unity-catalog`), never deep paths. Internal files keep
   relative imports within the module. This breaks the cycle because the
   `*EntityDialog`s and storage dialogs now live inside and import siblings
   relatively.

4. **Handle the one non-UC outbound edge** (`ExpansionContext` →
   `ActiveEnvironmentContext`): keep it working as-is for now, but isolate it so
   it's the single documented seam to environment state (a prop, context, or a
   small adapter) rather than a deep import buried in core. Note it in the
   module's README as the one thing extraction would need to parameterize.

5. **Add a lightweight boundary guard** so the structure doesn't rot: a Biome
   `noRestrictedImports` (or equivalent) rule forbidding imports of the module's
   internal paths from outside it — only the barrel is allowed. If that's heavier
   than the repo wants, leave a short `README.md` in the module stating the rule
   instead. (There is no boundary lint in the repo today; `@/*` is the only
   alias.)

6. **Update Storybook** — `CatalogExplorer.stories.tsx` and the fixture
   transport/mocks move with the module; verify stories still render.

## Constraints (both components)

- **No behavior change.** Pure restructuring + import rewrites. The prototype's
  client injection stays exactly as-is.
- **Dependency direction is the invariant.** `data-grid` (A) imports no UC and no
  app-feature code — it is the leaf. UC (B) may depend on A but does not in this
  task; nothing UC ever flows back into A. Enforce with the boundary guard.
- **Don't generalize `data-grid` beyond Arrow.** It keeps the `ArrowResultStore`
  input contract. Generalizing it (and porting `StorageTable` onto it) is the
  separate future refactor in the strategy doc.
- **Don't touch transport or the WASM story.** Out of scope; tracked separately.
- **Don't convert `*DetailQuery` to hooks.** Noted above as deliberate follow-up.
- **The storage components are in-scope and UC-internal** (decided — see B/#3).
  `StorageTable` is a UC data-display surface, not an external consumer; move it
  (and the rest of `storage/`) inside UC, exporting `StorageTable` /
  `StorageLocationPicker` through the UC barrel for their outside consumers.
- **`routes/import.lazy.tsx` consumes BOTH components** (`DataGrid` from A;
  `useCatalogs`/`useSchemas`/`invalidateTables` from B) — it gets rewritten in
  both steps. It also carries pre-existing lint errors (see Acceptance).
- Match the repo's existing conventions (alias usage, file naming, the design-doc
  style in `docs/`). Read the repo + `node/ui` `CLAUDE.md` first.

## Acceptance (both components)

- `npx tsc --noEmit` clean; `npm test` green; `npm run lint` introduces no new
  errors (there are pre-existing lint errors in `ingest/schema.ts` and
  `import.lazy.tsx` unrelated to this work — don't fix them here, don't add to
  them).
- **Component A:** no file outside `data-grid` imports a `data-grid` internal
  path (only its barrel); `data-grid` imports no UC and no app-feature code;
  `ResultsPane` and the import preview render identically; its story renders.
- **Component B:** no file outside UC imports a UC internal path (only its
  barrel); the cycle (`catalog/dialogs.tsx` ↔ `*EntityDialog`) is gone; the
  catalog browser + UC Storybook stories render identically to `main`.
- Each module has a one-paragraph `README.md` documenting its public surface (the
  barrel). UC's also documents the single environment-context seam
  (`ExpansionContext` → `ActiveEnvironmentContext`).

## Sequencing note (for the implementer)

Do **Component A first, then B** — A is the leaf B will eventually build on, and
doing it first means the boundary guard for the leaf already exists when B lands.

Within each, do the physical moves and import rewrites as **one or a few small
commits** with a green build at each step (move + rewrite + verify), not a single
giant rename — it makes the no-behavior-change claim reviewable. Keeping A and B
as separate commits (or separate PRs) is preferred since they're independent.
Follow the repo's unsigned-commit / sign-at-the-end workflow.
