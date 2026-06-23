import type { QueryClient } from "@tanstack/react-query";
import {
  createRootRouteWithContext,
  createRoute,
  type ErrorComponentProps,
  useRouter,
} from "@tanstack/react-router";
import { EnvironmentGate } from "@/components/EnvironmentGate";
import { ErrorFallback } from "@/components/ErrorBoundary";
import { prefetchCatalogs } from "@/lib/uc/queries";

export interface RouterContext {
  queryClient: QueryClient;
}

// Catches errors from route loaders (e.g. prefetchCatalogs) and route-component
// renders. Rendered inside EnvironmentGate's <Outlet />, so the persistent
// header + nav stay mounted and only the routed content shows the fallback.
function RouteError({ error }: ErrorComponentProps) {
  const router = useRouter();
  // invalidate() re-runs loaders and re-mounts the route, clearing the error.
  return <ErrorFallback error={error} onReset={() => router.invalidate()} />;
}

const rootRoute = createRootRouteWithContext<RouterContext>()({
  component: EnvironmentGate,
  // Inherited by all child routes (none override it): catches loader/render
  // errors and shows RouteError inside EnvironmentGate's <Outlet />.
  errorComponent: RouteError,
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
}).lazy(() => import("./routes/index.lazy").then((m) => m.Route));

const serviceRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "services/$serviceId",
}).lazy(() => import("./routes/services.$serviceId.lazy").then((m) => m.Route));

interface EditorSearch {
  // Active tab, encoded as the file path. The open-tab set is persisted to
  // sessionStorage; only the active path lives in the URL (deep-linkable).
  path?: string;
  // Active volume, encoded as its file-API root path (e.g. "/home" or
  // "/Volumes/main/default/data"). Deep-linkable; the tree roots here.
  volume?: string;
}

const editorRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "editor",
  validateSearch: (search: Record<string, unknown>): EditorSearch => ({
    path: typeof search.path === "string" ? search.path : undefined,
    volume: typeof search.volume === "string" ? search.volume : undefined,
  }),
}).lazy(() => import("./routes/editor.lazy").then((m) => m.Route));

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

const importRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "import",
}).lazy(() => import("./routes/import.lazy").then((m) => m.Route));

export const routeTree = rootRoute.addChildren([
  indexRoute,
  serviceRoute,
  editorRoute,
  catalogRoute,
  importRoute,
]);
