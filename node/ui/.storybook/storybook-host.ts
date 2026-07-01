// Storybook's "host" bundle — the analogue of node/desktop/src/main.ts, but it
// registers in-memory FIXTURE fakes at the UI's seams instead of Tauri. Imported
// once from preview.tsx, before any story renders, so every component sees a
// fully-populated backend with no network, no Docker, and no Tauri.
//
// This is the whole point of the registry architecture: the UI components are
// unchanged and unaware: they call the same generic seams the desktop host
// satisfies with Tauri, which Storybook satisfies with fixtures.

import { setDefaultUnityCatalogFetch } from "@open-lakehouse/unity-catalog-client";
import { registerEnvironmentHost } from "@/lib/client/environments";
import { registerFetch, registerTransport } from "@/lib/client/registry";
import { registerFilePicker } from "@/lib/ingest/registry";

import { fixtureEnvironmentHost } from "./fixture-environments";
import { fixtureFetch } from "./fixture-fetch";
import { fixtureTransport } from "./fixture-transport";

let installed = false;

/** Install the fixture fakes at every UI seam. Idempotent. */
export function installStorybookHost(): void {
  if (installed) return;
  installed = true;

  // UC REST -> fixtures. Two seams need it: the app's fetch registry (clientFetch,
  // used by the ConnectRPC clients and any non-UC HTTP), AND the Unity Catalog
  // package's own default client, which routes through its own fetch slot rather
  // than the app registry. The app wires the latter to clientFetch in main.tsx;
  // Storybook doesn't run main.tsx, so it must point the UC default at the fixture
  // fetch directly, or UC-backed stories would hit the real network.
  registerFetch(fixtureFetch);
  setDefaultUnityCatalogFetch(fixtureFetch);
  // ConnectRPC (Query / Ingest / Tags / Files) -> in-memory router transport.
  registerTransport(fixtureTransport);
  // Environment management + capabilities + service status -> fake managed host.
  registerEnvironmentHost(fixtureEnvironmentHost);
  // SQL IntelliSense already defaults to the bundled fixture catalog provider
  // (lib/editor/catalogProvider.ts) with sample metadata, so it needs no setup
  // here — there is no live UC to point it at.

  // Ingest: register a no-op picker so the import UI lights up. It resolves to a
  // fixture path; the fixture transport's previewFile ignores the path and
  // returns canned Arrow, so the preview flow renders end-to-end.
  registerFilePicker(async () => ({
    path: "/fixtures/events.parquet",
    name: "events.parquet",
  }));

  // Notebook host intentionally left unregistered: marimo needs a live sidecar +
  // iframe proxy that has no fixture analogue, so `.py` files stay plain-text
  // tabs in Storybook (matching the web build).
}
