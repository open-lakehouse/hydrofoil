import { createLazyRoute } from "@tanstack/react-router";

import { CatalogExplorer } from "@/features/unity-catalog";

export const Route = createLazyRoute("/catalog")({
  component: CatalogExplorer,
});
