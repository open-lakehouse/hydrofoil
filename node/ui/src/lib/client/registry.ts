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
