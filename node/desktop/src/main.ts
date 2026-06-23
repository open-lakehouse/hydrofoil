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
import { registerNotebookHost } from "@/lib/notebook/registry";
// Desktop stylesheet: re-exports the UI's globals AND declares the UI source as a
// Tailwind content root, so utility classes used by the UI components are emitted
// when building from node/desktop (see styles.css).
import "./styles.css";
import { tauriEnvironmentHost } from "./tauri-environments";
import { tauriFetch } from "./tauri-fetch";
import { tauriFilePicker } from "./tauri-ingest";
import { tauriNotebookHost } from "./tauri-notebook";
import { tauriTransport } from "./tauri-transport";

// Never bootstrap inside an iframe. The shell embeds service UIs (MLflow, marimo,
// Jaeger) in iframes via gateway-relative paths. If such a path fails to proxy
// (gateway/service down), the dev server's SPA fallback serves THIS app's
// index.html into the iframe — which would re-run this entry in a child browsing
// context that has NO Tauri IPC injected, so the first `invoke` throws
// "window.__TAURI_INTERNALS__.invoke is undefined". Skipping setup keeps a broken
// embed as a blank frame instead of a crashing second app instance.
if (window.self === window.top) {
  // Route ConnectRPC clients (QueryService, Tags, Files) to the in-process
  // executors via Tauri commands…
  registerTransport(tauriTransport);
  // …and the UC Catalog REST API (and any other plain-HTTP call) through the
  // Tauri fetch, which falls through to the network / UC sidecar.
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
  // Notebooks: opening a `.py` file spins up a marimo sidecar and embeds it.
  // Registering this is what makes `.py` files classify as notebook tabs (web
  // leaves it unregistered, so `.py` stays a plain text file).
  registerNotebookHost(tauriNotebookHost);

  // Dynamically import the UI bootstrap so it (and the api.ts client it pulls in)
  // evaluates only AFTER the fetch is registered. The registry is late-binding so
  // order wouldn't actually matter, but this keeps the intent obvious.
  void import("@/main");
}
