// Public surface of the `data-grid` module — the ONLY entry point other code
// may import from. Internal files (cells, header, the Arrow-support modules
// under ./lib) are not exported and must not be imported from outside.
//
// `data-grid` is the repo's reusable, virtualized, Arrow-backed table primitive.
// It is a LEAF: it depends only on apache-arrow, @tanstack/*, the shadcn
// primitives in @/components/ui/*, and @/lib/utils — never on Unity Catalog or
// any app feature. Both the SQL editor result pane and the import preview build
// on it today; the Unity Catalog module may build on it in the future, never the
// reverse. See ./README.md.

export { DataGrid } from "./data-grid";
// The grid's input contract. Consumers (the editor's run controller, the import
// preview, Storybook fixtures) construct an `ArrowResultStore` and hand it to
// `<DataGrid store={…}>`. Kept Arrow-specific by design — generalizing the grid
// beyond Arrow is a separate, behavior-changing refactor.
export {
  ArrowResultStore,
  type ArrowStoreInfo,
} from "./lib/arrowResultStore";
