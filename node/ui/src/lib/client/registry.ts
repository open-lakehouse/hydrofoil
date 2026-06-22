// Pluggable fetch registry — the single seam that lets a host environment route
// the UI's HTTP calls somewhere other than the network, WITHOUT the UI taking on
// any dependency on that host.
//
// By default this is plain `globalThis.fetch`, so the web build behaves exactly
// as if it called fetch directly. A host (e.g. the Tauri desktop shell in
// node/desktop) can call `registerFetch` before the UI bootstraps to swap in its
// own implementation — that implementation decides per request whether to handle
// it itself or fall through to the network.
//
// Deliberately framework-agnostic: no Tauri, no `import.meta.env`, no globals.

import type { Transport } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";

export type FetchImpl = typeof globalThis.fetch;

// Default to the platform fetch. We wrap rather than alias so the binding stays
// correct (`fetch` must be called with `globalThis` as its receiver).
let currentFetch: FetchImpl = (...args) => globalThis.fetch(...args);

/** Install a custom fetch. Hosts call this once, before the UI bootstraps. */
export function registerFetch(fn: FetchImpl): void {
  currentFetch = fn;
}

/** The fetch currently in effect (the registered one, or the platform default). */
export function getFetch(): FetchImpl {
  return currentFetch;
}

// Stable reference handed to the API client. It dereferences `currentFetch` on
// every call (late binding), so a host can register its fetch before OR after
// this module — and before or after the client is constructed — and still take
// effect. This removes any module-evaluation ordering constraint.
export const clientFetch: FetchImpl = (...args) => currentFetch(...args);

// --- ConnectRPC transport registry --------------------------------------------
//
// The transport analogue of the fetch registry: the seam that lets a host route
// ConnectRPC client calls (QueryService, Tags, Files) somewhere other than the
// network, WITHOUT the UI depending on the host. ConnectRPC clients are built
// with `clientTransport`; by default it speaks Connect over `clientFetch`, so the
// web build is a normal client. The Tauri desktop host registers a transport that
// dispatches against the in-process executors via `invoke` (see node/desktop).

// Default: Connect-over-fetch. An empty base resolves against the dev origin; the
// Vite proxy forwards the RPC path prefixes (see vite.config.ts). `useBinaryFormat`
// keeps payloads (e.g. Arrow IPC) as binary protobuf, not base64-in-JSON.
const defaultTransport: Transport = createConnectTransport({
  baseUrl: import.meta.env.VITE_QUERY_API_URL ?? "/",
  fetch: clientFetch,
  useBinaryFormat: true,
});

let currentTransport: Transport = defaultTransport;

/** Install a custom transport. Hosts call this once, before the UI bootstraps. */
export function registerTransport(t: Transport): void {
  currentTransport = t;
}

/** The transport currently in effect (the registered one, or the default). */
export function getTransport(): Transport {
  return currentTransport;
}

// Stable, late-binding transport handed to ConnectRPC clients. Each method
// dereferences `currentTransport` on every call, so registration order relative
// to client construction never matters.
export const clientTransport: Transport = {
  unary(...args) {
    return currentTransport.unary(...args);
  },
  stream(...args) {
    return currentTransport.stream(...args);
  },
};
