// Pluggable environment-host registry — the seam that lets a host (the Tauri
// desktop shell) own environment management and service lifecycle, WITHOUT the
// UI depending on the host. Mirrors the fetch/transport registries in
// ./registry.ts.
//
// An "environment" is a named bundle of service configuration. Selecting one is
// what brings its services online (on desktop: spawning the Unity Catalog
// sidecar). The web build has no environments to manage, so the default host is
// a single, always-active environment — the gate is transparent there.
//
// Deliberately framework-agnostic: no Tauri, no `import.meta.env`, no globals.

export type Environment = { id: string; name: string };

export interface EnvironmentHost {
  /** Whether the host supports multiple environments / a selection step. When
   *  false, the UI skips the picker entirely (web build). */
  readonly managed: boolean;
  /** List the configured environments. */
  list(): Promise<Environment[]>;
  /** The id of the currently-active environment (services bound), or null when
   *  none is. Non-null at startup means the host activated one (e.g. via an
   *  escape hatch) and the picker should be skipped. The shell also uses this to
   *  highlight the running environment in the overview. */
  active(): Promise<string | null>;
  /** Create a new environment (no services spawned yet). */
  create(name: string): Promise<Environment>;
  /** Select an environment: the host brings its services online. Resolves once
   *  they are ready, so the UI can mount the app after this returns. */
  select(id: string): Promise<void>;
}

// Default: a single implicit environment that is always active. The web build
// reaches its services over the network regardless, so there is nothing to pick.
const defaultHost: EnvironmentHost = {
  managed: false,
  list: async () => [{ id: "default", name: "Default" }],
  active: async () => "default",
  create: async () => ({ id: "default", name: "Default" }),
  select: async () => {},
};

let currentHost: EnvironmentHost = defaultHost;

/** Install a custom environment host. Hosts call this once, before the UI
 *  bootstraps (see node/desktop). */
export function registerEnvironmentHost(host: EnvironmentHost): void {
  currentHost = host;
}

/** The environment host currently in effect (registered, or the default). */
export function getEnvironmentHost(): EnvironmentHost {
  return currentHost;
}
