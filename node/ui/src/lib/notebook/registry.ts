// Pluggable notebook seam — lets a host environment (the Tauri desktop shell)
// open a Python file as a live marimo notebook WITHOUT the UI taking on a Tauri
// dependency. The editor depends only on the types and functions here; the
// desktop host registers its implementation before the UI bootstraps (mirrors
// `lib/client/registry.ts`, `lib/query/runner.ts`, and `lib/ingest/registry.ts`).
//
// This is desktop-only: marimo runs as a per-environment `uvx` sidecar that the
// desktop shell supervises, and the notebook UI is embedded in an <iframe>
// served through a Tauri custom-protocol proxy. A web build registers nothing,
// so `.py` files fall back to the plain Monaco text editor (see
// `lib/editor/language.ts`).

/** A prepared notebook session: where to embed it and how to refer back to it. */
export interface NotebookSession {
  /**
   * The iframe `src` for the notebook UI. On desktop this is a Tauri
   * custom-protocol URL (e.g. `olservice://notebook/?file=<rel>`) that proxies
   * to the marimo sidecar, so it loads same-origin to that protocol.
   */
  url: string;
  /** Opaque handle the host uses to sync/close this notebook's working copy. */
  sessionId: string;
}

/**
 * Host capability for the notebook editor. The desktop shell implements this by
 * copying the volume file into a sandboxed working dir, ensuring the shared
 * marimo sidecar is up, and proxying its UI. The default (web) is unregistered.
 */
export interface NotebookHost {
  /**
   * Ensure the shared marimo sidecar is running and a working copy exists for
   * `volumePath`; resolves to the session to embed.
   */
  openNotebook(volumePath: string): Promise<NotebookSession>;
  /** Flush the working copy back to its volume (autosave / on close). */
  syncNotebook(sessionId: string): Promise<void>;
  /** Release the working copy and any per-session bookkeeping (on tab close). */
  closeNotebook(sessionId: string): Promise<void>;
}

let current: NotebookHost | null = null;

/** Install a host notebook implementation. Hosts call this once, before the UI bootstraps. */
export function registerNotebookHost(host: NotebookHost): void {
  current = host;
}

/**
 * The registered host, or `null` on web. Callers that have a `notebook` tab
 * already know a host exists (dispatch gates on `notebookSupported`), but this
 * keeps the late-binding contract explicit.
 */
export function getNotebookHost(): NotebookHost | null {
  return current;
}

/**
 * Whether a host has registered notebook support. Gates `.py` → notebook tab
 * classification: on web (unregistered) `.py` stays a plain text file.
 */
export function notebookSupported(): boolean {
  return current !== null;
}
