// Tauri desktop entry point.
//
// This runs BEFORE the UI bootstraps. It installs the Tauri fetch into the UI's
// generic fetch registry, then hands off to the existing UI app — which is reused
// wholesale, with no knowledge that it's running inside Tauri.
//
// `@` resolves to ../ui/src (see vite.config.ts / tsconfig.json), so we import the
// UI's own modules directly without adding any `exports` surface to @open-lakehouse/ui.
import { registerEnvironmentHost } from "@/lib/client/environments";
import { registerFetch, registerTransport } from "@/lib/client/registry";
import {
  setCatalogProvider,
  unityCatalogProvider,
} from "@/lib/editor/catalogProvider";
import { registerFilePicker } from "@/lib/ingest/registry";
// Desktop stylesheet: re-exports the UI's globals AND declares the UI source as a
// Tailwind content root, so utility classes used by the UI components are emitted
// when building from node/desktop (see styles.css).
import "./styles.css";
import { tauriEnvironmentHost } from "./tauri-environments";
import { tauriFetch } from "./tauri-fetch";
import { tauriFilePicker } from "./tauri-ingest";
import { tauriTransport } from "./tauri-transport";

// Route ConnectRPC clients (QueryService, Tags, Files) to the in-process
// executors via Tauri commands…
registerTransport(tauriTransport);
// …and the UC Catalog REST API (and any other plain-HTTP call) through the Tauri
// fetch, which falls through to the network / UC sidecar.
registerFetch(tauriFetch);
// The outer shell owns environment management + service lifecycle on desktop:
// the gate lists/creates environments and spawns the UC sidecar on selection.
registerEnvironmentHost(tauriEnvironmentHost);
// SQL IntelliSense uses real catalog metadata: on desktop UC is reachable (via
// the proxy above), so swap the editor's fixture catalog for the live one.
setCatalogProvider(unityCatalogProvider);
// The import page reads a local file by path; supply the native file picker so
// the page (and its nav entry) light up on desktop.
registerFilePicker(tauriFilePicker);

// Dynamically import the UI bootstrap so it (and the api.ts client it pulls in)
// evaluates only AFTER the fetch is registered. The registry is late-binding so
// order wouldn't actually matter, but this keeps the intent obvious.
void import("@/main");
