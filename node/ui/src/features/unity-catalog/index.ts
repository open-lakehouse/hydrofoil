// Public surface of the Unity Catalog module — the ONLY entry point other code
// may import from. Everything else (tree internals, detail panes, dialog wiring,
// selection, ExpansionContext, the *EntityDialog and storage dialogs, the uc/
// data layer internals) is module-internal and must not be imported from outside.
//
// The module owns the catalog browser, the per-entity detail panes, the
// create/edit/delete + storage dialogs, and the React-Query data layer that
// talks to the Unity Catalog REST API. It depends on shared primitives
// (@/components/ui/*, @/components/forms/*, @/lib/utils), the generic client
// factory (@/lib/api), and exactly one host edge — the environment scope id,
// isolated in ./env-seam. See ./README.md.
//
// Dependency direction: the module may build on the `data-grid` leaf in the
// future; `data-grid` never builds on it. (Today StorageTable hand-rolls its own
// table and does not use data-grid — porting it is a separate refactor.)

// ── Route-level UI ───────────────────────────────────────────────────────────
export { CatalogExplorer } from "./CatalogExplorer";
// ── Shared presentational primitives reused by the host ──────────────────────
// EnvironmentDetail renders entity metadata; the editor's file browser reuses
// the tree row + list-state primitives.
export { Meta, MetaGrid } from "./detail/Meta";
// Dialog orchestration the environment manager mounts around catalog actions.
export { CatalogDialogsProvider } from "./dialogs";
export { StorageLocationPicker } from "./storage/StorageLocationPicker";
// ── Storage admin surfaces (used by the environment manager) ─────────────────
export { StorageTable } from "./storage/StorageTable";
export { ListStates, TreeRow } from "./TreeRow";
export type { StorageKind } from "./types";
// ── Provider / client injection ─────────────────────────────────────────────
export {
  UnityCatalogProvider,
  useUnityCatalog,
} from "./uc/context";
// ── Error helper ─────────────────────────────────────────────────────────────
export { parseUcError } from "./uc/errors";

// ── Invalidators ─────────────────────────────────────────────────────────────
export { invalidateTables } from "./uc/mutations";
// ── Read hooks ───────────────────────────────────────────────────────────────
export {
  prefetchCatalogs,
  useCatalogs,
  useCredentials,
  useExternalLocations,
  useSchemas,
  useVolumes,
} from "./uc/queries";
