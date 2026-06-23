// Tauri implementation of the UI's environment host (see
// node/ui/src/lib/client/environments.ts). Like tauri-fetch / tauri-transport,
// this is one of the few places on the JS side that imports `@tauri-apps/api`,
// keeping node/ui completely Tauri-free.
//
// It maps the host operations onto the Rust commands in src-tauri/src/lib.rs:
// listing / activating / creating / starting / stopping environments. `start`
// resolves only after the Unity Catalog sidecar is spawned and the in-process
// executors are bound; `stop` kills the sidecar and clears the active services.

import { invoke } from "@tauri-apps/api/core";
import type {
  ActiveEnvironment,
  Capability,
  ConfigArtifact,
  Environment,
  EnvironmentHost,
  KeyProvider,
  KeyStatus,
  ServiceStatus,
} from "@/lib/client/environments";
import { HOME_VOLUME } from "@/lib/editor/volumes";

// The descriptor shape the Rust `active_environment` / `start_environment`
// commands return. The UI's `ActiveEnvironment` adds derived built-in volumes.
interface EnvDescriptor {
  id: string;
  name: string;
  capabilities: { hasHome: boolean };
}

// Map a Rust descriptor onto the UI's `ActiveEnvironment`, deriving built-in
// volumes from capabilities: a local Home volume when the host serves one.
function toActiveEnvironment(d: EnvDescriptor): ActiveEnvironment {
  return {
    id: d.id,
    name: d.name,
    capabilities: { hasHome: d.capabilities.hasHome },
    volumes: d.capabilities.hasHome ? [HOME_VOLUME] : [],
  };
}

export const tauriEnvironmentHost: EnvironmentHost = {
  managed: true,
  list: () => invoke<Environment[]>("list_environments"),
  active: async () => {
    const d = await invoke<EnvDescriptor | null>("active_environment");
    return d ? toActiveEnvironment(d) : null;
  },
  create: (name: string) => invoke<Environment>("create_environment", { name }),
  start: async (id: string) => {
    const d = await invoke<EnvDescriptor>("start_environment", { id });
    return toActiveEnvironment(d);
  },
  stop: (id: string) => invoke<void>("stop_environment", { id }),
  keyStatus: (id: string) =>
    invoke<KeyStatus>("environment_key_status", { id }),
  configureKey: (id: string, provider: KeyProvider) =>
    invoke<KeyStatus>("configure_environment_key", { id, provider }),
  dockerStatus: () => invoke<boolean>("docker_status"),
  availableCapabilities: () => invoke<Capability[]>("available_capabilities"),
  environmentCapabilities: (id: string) =>
    invoke<string[]>("environment_capabilities", { id }),
  setEnvironmentCapabilities: (id: string, capabilities: string[]) =>
    invoke<void>("set_environment_capabilities", { id, capabilities }),
  configArtifacts: (id: string) =>
    invoke<ConfigArtifact[]>("environment_config_artifacts", { id }),
  serviceStatus: (id: string) =>
    invoke<ServiceStatus[]>("environment_service_status", { id }),
  telemetryStatus: () => invoke<boolean>("telemetry_status"),
};
