// The client-side source of truth for the *currently selected* environment.
//
// `EnvironmentHost` (lib/client/environments.ts) is a lifecycle seam
// (list/create/select). This context is its runtime counterpart: once an
// environment is selected it holds the resulting `ActiveEnvironment` so that
// env-scoped state — the editor's volumes, the query cache, the result-session
// registry — can read capabilities and built-in volumes reactively instead of
// calling `getEnvironmentHost()` imperatively.
//
// The provider takes the active environment as a value, so a Storybook story (or
// any harness) can supply a mock `ActiveEnvironment` with no host at all.

import { createContext, type ReactNode, useContext } from "react";
import type { ActiveEnvironment } from "@/lib/client/environments";

const ActiveEnvironmentContext = createContext<ActiveEnvironment | null>(null);

export function ActiveEnvironmentProvider({
  environment,
  children,
}: {
  environment: ActiveEnvironment;
  children: ReactNode;
}) {
  return (
    <ActiveEnvironmentContext.Provider value={environment}>
      {children}
    </ActiveEnvironmentContext.Provider>
  );
}

/** The active environment. Throws if used outside a provider — every env-scoped
 *  component renders under one (the app shell mounts it once an environment is
 *  selected). */
export function useActiveEnvironment(): ActiveEnvironment {
  const env = useContext(ActiveEnvironmentContext);
  if (!env) {
    throw new Error(
      "useActiveEnvironment must be used within an ActiveEnvironmentProvider",
    );
  }
  return env;
}
