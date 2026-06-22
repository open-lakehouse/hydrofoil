// Tauri implementation of the UI's environment host (see
// node/ui/src/lib/client/environments.ts). Like tauri-fetch / tauri-transport,
// this is one of the few places on the JS side that imports `@tauri-apps/api`,
// keeping node/ui completely Tauri-free.
//
// It maps the four host operations onto the Rust commands in src-tauri/src/lib.rs:
// listing / activating / creating / selecting environments. `select` resolves
// only after the Unity Catalog sidecar is spawned and the in-process executors
// are bound, so the UI can mount the app immediately afterwards.

import { invoke } from "@tauri-apps/api/core";
import type {
  ActiveEnvironment,
  Environment,
  EnvironmentHost,
} from "@/lib/client/environments";
import { HOME_VOLUME } from "@/lib/editor/volumes";

// The descriptor shape the Rust `active_environment` / `select_environment`
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
  select: async (id: string) => {
    const d = await invoke<EnvDescriptor>("select_environment", { id });
    return toActiveEnvironment(d);
  },
};
