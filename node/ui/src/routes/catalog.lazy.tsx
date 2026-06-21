import { createLazyRoute } from "@tanstack/react-router";

import { CatalogExplorer } from "@/components/catalog/CatalogExplorer";

export const Route = createLazyRoute("/catalog")({
  component: CatalogExplorer,
});
