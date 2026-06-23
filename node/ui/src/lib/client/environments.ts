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

/** Where an environment's credential-encryption key (KEK) lives. `keychain` keeps
 *  it in the OS-native secret store (desktop); `remote` defers to an external key
 *  store (scaffolded — not yet fully wired). */
export type KeyProvider = "keychain" | "remote";

/** Encryption-key status the UI can render without starting the environment.
 *  `unconfigured` — no key yet; `keychain`/`remote` — provisioned under that
 *  provider; `unavailable` — the OS keychain can't be reached, so the environment
 *  cannot start until a key store is configured. */
export type KeyStatus = "unconfigured" | "keychain" | "remote" | "unavailable";

/** Lifecycle status of an environment as the UI presents it. Forward-compatible
 *  with multiple running environments later; today only the single active
 *  environment is "running" and everything else is "idle". "starting"/"stopping"
 *  are transient UI states held while the async host call is in flight. */
export type EnvironmentStatus = "idle" | "starting" | "running" | "stopping";

/** A capability the user can enable on an environment (lineage, observability,
 *  model tracking, object storage). The host owns the technology mapping; the UI
 *  just renders the checklist and persists the selection. */
export type Capability = { id: string; label: string };

/** A read-only config artifact surfaced for inspection/learning (the generated
 *  compose, a service fragment, the Envoy/collector config). `language` is the
 *  Monaco language id. */
export type ConfigArtifact = {
  id: string;
  label: string;
  description: string;
  language: string;
  content: string;
};

/** Live status of one running service (container) in an environment. `shared`
 *  marks the app-level telemetry collector, which is not per-environment. */
export type ServiceStatus = {
  service: string;
  /** Compose state: `running`, `exited`, `restarting`, … */
  state: string;
  /** Health when declared: `healthy`, `starting`, `unhealthy`, or empty. */
  health: string;
  shared: boolean;
};

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
  /** Start an environment: the host brings its services online and resolves to
   *  the active environment (capabilities + built-in volumes) once they are
   *  ready. Starting does NOT imply opening the app — the caller decides whether
   *  to navigate into it (launch) or stay in the manager. */
  start(id: string): Promise<ActiveEnvironment>;
  /** Stop an environment: tear down its services. Idempotent — a no-op when the
   *  environment is not running. After this resolves the environment is idle. */
  stop(id: string): Promise<void>;
  /** Current encryption-key status for an environment, resolvable without
   *  starting it. */
  keyStatus(id: string): Promise<KeyStatus>;
  /** Choose the key provider for an environment, returning the resulting status.
   *  For `keychain` the host mints the key eagerly, so a keychain failure surfaces
   *  here rather than at start. */
  configureKey(id: string, provider: KeyProvider): Promise<KeyStatus>;
  /** Whether the host's container runtime (Docker) is available. Drives the
   *  graceful-degrade banner: capabilities needing Docker are disabled when false.
   *  Always true for hosts with no container dependency. */
  dockerStatus(): Promise<boolean>;
  /** The capabilities a user can enable, for the checklist. */
  availableCapabilities(): Promise<Capability[]>;
  /** An environment's currently-enabled capability ids (for pre-checking). */
  environmentCapabilities(id: string): Promise<string[]>;
  /** Persist an environment's enabled capabilities. Takes effect on next start. */
  setEnvironmentCapabilities(id: string, capabilities: string[]): Promise<void>;
  /** Read-only config artifacts (generated compose + static configs) for the
   *  environment's selected capabilities — for the inspection/learning viewer.
   *  Generated on demand, so available before the environment has started. */
  configArtifacts(id: string): Promise<ConfigArtifact[]>;
  /** Live per-service status (state + health) for a running environment. Polled
   *  on a gentle interval by the UI; empty when nothing is up. */
  serviceStatus(id: string): Promise<ServiceStatus[]>;
  /** Whether the shared, app-level telemetry collector (Jaeger) is running.
   *  Drives the Telemetry entry's status and whether its UI is embeddable. */
  telemetryStatus(): Promise<boolean>;
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
  start: async () => DEFAULT_ENVIRONMENT,
  // The web build has a single always-on environment and no services to tear
  // down, so stopping is a no-op.
  stop: async () => {},
  // The web build's server owns key management; there is nothing local to
  // provision or surface, so report a remote-managed key and treat configuration
  // as a no-op.
  keyStatus: async () => "remote",
  configureKey: async () => "remote",
  // The web build has no local container runtime and no per-env capabilities to
  // manage — Docker is "available" (nothing to gate), the capability set is empty,
  // and there are no local config artifacts to inspect.
  dockerStatus: async () => true,
  availableCapabilities: async () => [],
  environmentCapabilities: async () => [],
  setEnvironmentCapabilities: async () => {},
  configArtifacts: async () => [],
  serviceStatus: async () => [],
  telemetryStatus: async () => false,
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
