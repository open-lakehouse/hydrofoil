// Environment-scoped registry of notebook sessions.
//
// Each open notebook tab maps to one NotebookController, which drives the
// async lifecycle of asking the host to prepare the marimo session (copy a
// working copy + ensure the sidecar) and then holds the iframe URL + the
// host's session handle. Mirrors ResultSessionRegistry / RunController: a plain
// subscribable object so the NotebookPane can `useSyncExternalStore` it, and
// the registry owns disposal so sessions are scoped to the environment (torn
// down when the editor provider remounts on an environment switch).

import { getNotebookHost, type NotebookHost } from "./registry";

export interface NotebookSnapshot {
  /** True while the host is preparing the session (sidecar boot / copy). */
  loading: boolean;
  /** The iframe `src` once ready, else null. */
  url: string | null;
  /** Error message if preparation failed, else null. */
  error: string | null;
}

const INITIAL: NotebookSnapshot = { loading: true, url: null, error: null };

export class NotebookController {
  private snapshot: NotebookSnapshot = INITIAL;
  private listeners = new Set<() => void>();
  private sessionId: string | null = null;
  private started = false;

  /** @param volumePath the notebook file (the tab id) this controller serves. */
  constructor(
    private readonly volumePath: string,
    private readonly host: NotebookHost,
  ) {}

  /** getSnapshot for useSyncExternalStore (stable reference between changes). */
  get = (): NotebookSnapshot => this.snapshot;

  subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  private emit(next: Partial<NotebookSnapshot>) {
    this.snapshot = { ...this.snapshot, ...next };
    for (const fn of this.listeners) fn();
  }

  /** Ask the host to prepare the session. Idempotent — the first call starts it;
   *  later calls are no-ops (the iframe is cheap to keep mounted). */
  start(): void {
    if (this.started) return;
    this.started = true;
    this.host
      .openNotebook(this.volumePath)
      .then((session) => {
        this.sessionId = session.sessionId;
        this.emit({ loading: false, url: session.url, error: null });
      })
      .catch((err: unknown) => {
        this.emit({
          loading: false,
          url: null,
          error: err instanceof Error ? err.message : String(err),
        });
      });
  }

  /** Flush the working copy back to the volume (autosave tick / pre-close). */
  async sync(): Promise<void> {
    if (this.sessionId) await this.host.syncNotebook(this.sessionId);
  }

  /** Close the host session and drop listeners (on tab close). */
  async dispose(): Promise<void> {
    this.listeners.clear();
    if (this.sessionId) {
      const id = this.sessionId;
      this.sessionId = null;
      await this.host.closeNotebook(id);
    }
  }
}

export class NotebookSessionRegistry {
  private controllers = new Map<string, NotebookController>();

  /** The NotebookController for a notebook file, created on first request.
   *  Returns null if no notebook host is registered (should not happen for a
   *  tab classified `notebook`, but keeps the contract explicit). */
  controller(volumePath: string): NotebookController | null {
    let ctrl = this.controllers.get(volumePath);
    if (!ctrl) {
      const host = getNotebookHost();
      if (!host) return null;
      ctrl = new NotebookController(volumePath, host);
      this.controllers.set(volumePath, ctrl);
    }
    return ctrl;
  }

  /** Close + drop a single notebook's controller (on tab close). */
  release(volumePath: string): void {
    const ctrl = this.controllers.get(volumePath);
    if (ctrl) {
      this.controllers.delete(volumePath);
      void ctrl.dispose();
    }
  }

  /** Close + drop every controller (on environment teardown). */
  dispose(): void {
    for (const ctrl of this.controllers.values()) void ctrl.dispose();
    this.controllers.clear();
  }
}
