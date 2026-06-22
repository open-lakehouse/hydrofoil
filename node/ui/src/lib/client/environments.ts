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

import type { Volume } from "@/lib/editor/volumes";

export type Environment = { id: string; name: string };

/** What a selected environment can do. A capability is shaped by which services
 *  the host wired up for it; the UI shells itself to fit. Grows over time
 *  (lineage, credential vending, a write path, …) — add fields here rather than
 *  reading host booleans imperatively at call sites. */
export interface EnvironmentCapabilities {
  /** Whether the host provides a local "home" volume (a `/home` file tree backed
   *  by local disk). True on desktop; false on the web build, which has no local
   *  disk — the editor then offers only Unity Catalog volumes. */
  readonly hasHome: boolean;
}

/** A selected, online environment: the client-side source of truth that
 *  env-scoped state (volumes, the query cache, result sessions) keys off of. */
export interface ActiveEnvironment {
  readonly id: string;
  readonly name: string;
  readonly capabilities: EnvironmentCapabilities;
  /** Built-in volumes this environment serves (e.g. Home on desktop). The editor
   *  layers user-browsed UC volumes on top of these. */
  readonly volumes: readonly Volume[];
}

export interface EnvironmentHost {
  /** Whether the host supports multiple environments / a selection step. When
   *  false, the UI skips the picker entirely (web build). */
  readonly managed: boolean;
  /** List the configured environments. */
  list(): Promise<Environment[]>;
  /** The currently-active environment (services bound), or null when none is.
   *  Non-null at startup means the host activated one (e.g. via an escape hatch)
   *  and the picker should be skipped — the shell uses it both to scope state and
   *  to highlight the running environment in the overview. */
  active(): Promise<ActiveEnvironment | null>;
  /** Create a new environment (no services spawned yet). */
  create(name: string): Promise<Environment>;
  /** Select an environment: the host brings its services online and resolves to
   *  the active environment (capabilities + built-in volumes) once they are
   *  ready, so the UI can mount the app and scope its state after this returns. */
  select(id: string): Promise<ActiveEnvironment>;
}

// Default: a single implicit environment that is always active. The web build
// reaches its services over the network regardless, so there is nothing to pick,
// no local disk (no Home volume), and no built-in volumes.
const DEFAULT_ENVIRONMENT: ActiveEnvironment = {
  id: "default",
  name: "Default",
  capabilities: { hasHome: false },
  volumes: [],
};

const defaultHost: EnvironmentHost = {
  managed: false,
  list: async () => [{ id: "default", name: "Default" }],
  active: async () => DEFAULT_ENVIRONMENT,
  create: async () => ({ id: "default", name: "Default" }),
  select: async () => DEFAULT_ENVIRONMENT,
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
