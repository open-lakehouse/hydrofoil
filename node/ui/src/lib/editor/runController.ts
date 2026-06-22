// Per-tab SQL run controller.
//
// A React-hook approach holding its state in component state is inherently
// single-stream, so it can't back N tabs at once. This is the streaming logic
// (coalesced version bumps via rAF, abort-on-rerun, fresh ArrowResultStore per
// run) as a plain subscribable object, so each SQL tab owns its own results that
// survive tab switches. The ResultsPane subscribes to the active tab's controller.

import { ArrowResultStore } from "@/lib/query/arrowResultStore";
import { queryRunner } from "@/lib/query/runner";

export interface RunSnapshot {
  store: ArrowResultStore | null;
  /** Bumped on each appended chunk and on completion — the re-render signal. */
  version: number;
  running: boolean;
  error: string | null;
}

const EMPTY: RunSnapshot = {
  store: null,
  version: 0,
  running: false,
  error: null,
};

export class RunController {
  private snapshot: RunSnapshot = EMPTY;
  private listeners = new Set<() => void>();
  private abort: AbortController | null = null;
  private raf: number | null = null;

  /** getSnapshot for useSyncExternalStore (stable reference between changes). */
  get = (): RunSnapshot => this.snapshot;

  subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  private emit(next: Partial<RunSnapshot>) {
    this.snapshot = { ...this.snapshot, ...next };
    for (const fn of this.listeners) fn();
  }

  private cancelRaf() {
    if (this.raf !== null) {
      cancelAnimationFrame(this.raf);
      this.raf = null;
    }
  }

  /** Abort any in-flight stream (e.g. on tab close or a new run). */
  cancel() {
    this.abort?.abort();
    this.abort = null;
    this.cancelRaf();
    if (this.snapshot.running) this.emit({ running: false });
  }

  async run(sql: string, limit?: number): Promise<void> {
    this.abort?.abort();
    this.cancelRaf();
    const controller = new AbortController();
    this.abort = controller;

    const store = new ArrowResultStore();
    this.snapshot = { store, version: 0, running: true, error: null };
    for (const fn of this.listeners) fn();

    // Coalesce version bumps to at most one re-render per frame.
    const scheduleFlush = () => {
      if (this.raf !== null) return;
      this.raf = requestAnimationFrame(() => {
        this.raf = null;
        if (controller.signal.aborted) return;
        this.emit({ version: this.snapshot.version + 1 });
      });
    };

    try {
      for await (const chunk of queryRunner(
        { sql, limit },
        { signal: controller.signal },
      )) {
        store.append(chunk.arrowIpc);
        scheduleFlush();
      }
      this.cancelRaf();
      this.emit({ version: this.snapshot.version + 1, running: false });
    } catch (err) {
      this.cancelRaf();
      if (controller.signal.aborted) {
        this.emit({ running: false });
        return;
      }
      this.emit({
        running: false,
        error: err instanceof Error ? err.message : String(err),
      });
    }
  }

  /** Abort + drop listeners (on tab close). */
  dispose() {
    this.cancel();
    this.listeners.clear();
  }
}
