import {
  ThemeProvider,
  Toaster,
  TooltipProvider,
} from "@open-lakehouse/ui-kit";
import {
  defaultUnityCatalogClient,
  setDefaultUnityCatalogFetch,
  UnityCatalogProvider,
} from "@open-lakehouse/unity-catalog";
import { QueryClientProvider } from "@tanstack/react-query";
import { createRouter, RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { ErrorBoundary } from "@/components/ErrorBoundary";
import { clientFetch } from "@/lib/client/registry";
import { createQueryClient } from "@/lib/query-client";
import { routeTree } from "./routeTree";
import "./app/globals.css";

// Route the default Unity Catalog client through the app's fetch registry, so a
// host that registers an alternative fetch (the Tauri desktop shell) transports
// UC calls over it — matching the pre-carve-out behavior where the default
// client used `clientFetch` directly. Must run before the first UC request.
setDefaultUnityCatalogFetch(clientFetch);

const queryClient = createQueryClient();

const router = createRouter({
  routeTree,
  context: { queryClient },
  defaultPreload: "intent",
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

const rootElement = document.getElementById("root");
if (!rootElement) throw new Error("Root element #root not found");

createRoot(rootElement).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <UnityCatalogProvider client={defaultUnityCatalogClient}>
        <ThemeProvider>
          <TooltipProvider delayDuration={300}>
            {/* Whole-app safety net: a render fault degrades to a recoverable
                screen instead of a blank page. The router supplies a finer-grained
                per-route fallback (see routeTree.tsx); this catches anything above
                the router. Toaster sits outside so toasts survive a faulted tree. */}
            <ErrorBoundary>
              <RouterProvider router={router} />
            </ErrorBoundary>
            <Toaster position="bottom-right" />
          </TooltipProvider>
        </ThemeProvider>
      </UnityCatalogProvider>
    </QueryClientProvider>
  </StrictMode>,
);
