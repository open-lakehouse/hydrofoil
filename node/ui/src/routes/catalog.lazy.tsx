import { CatalogExplorer } from "@open-lakehouse/unity-catalog";
import { createLazyRoute } from "@tanstack/react-router";

export const Route = createLazyRoute("/catalog")({
  component: CatalogExplorer,
});
