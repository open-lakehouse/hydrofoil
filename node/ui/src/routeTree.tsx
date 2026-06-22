import type { QueryClient } from "@tanstack/react-query";
import {
  createRootRouteWithContext,
  createRoute,
} from "@tanstack/react-router";
import { AppShell } from "@/components/AppShell";
import { prefetchCatalogs } from "@/lib/uc/queries";

export interface RouterContext {
  queryClient: QueryClient;
}

const rootRoute = createRootRouteWithContext<RouterContext>()({
  component: AppShell,
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
}).lazy(() => import("./routes/index.lazy").then((m) => m.Route));

const serviceRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "services/$serviceId",
}).lazy(() => import("./routes/services.$serviceId.lazy").then((m) => m.Route));

const queryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "query",
}).lazy(() => import("./routes/query.lazy").then((m) => m.Route));

interface CatalogSearch {
  // Selected object, encoded as `kind:fullName` (see components/catalog/selection.ts).
  sel?: string;
}

const catalogRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "catalog",
  // Selection is URL-addressable so detail views are deep-linkable.
  validateSearch: (search: Record<string, unknown>): CatalogSearch => ({
    sel: typeof search.sel === "string" ? search.sel : undefined,
  }),
  // Warm the catalog list before the route component mounts (prefetch-on-intent
  // pairs with defaultPreload: "intent" in main.tsx).
  loader: ({ context }) => prefetchCatalogs(context.queryClient),
}).lazy(() => import("./routes/catalog.lazy").then((m) => m.Route));

export const routeTree = rootRoute.addChildren([
  indexRoute,
  serviceRoute,
  queryRoute,
  catalogRoute,
]);
