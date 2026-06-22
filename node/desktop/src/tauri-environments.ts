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
import type { Environment, EnvironmentHost } from "@/lib/client/environments";

export const tauriEnvironmentHost: EnvironmentHost = {
  managed: true,
  list: () => invoke<Environment[]>("list_environments"),
  active: () => invoke<string | null>("active_environment"),
  create: (name: string) => invoke<Environment>("create_environment", { name }),
  select: async (id: string) => {
    await invoke("select_environment", { id });
  },
};
