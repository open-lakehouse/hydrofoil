// Per-tab SQL run controller.
//
// A React-hook approach holding its state in component state is inherently
// single-stream, so it can't back N tabs at once. This is the streaming logic
// (coalesced version bumps via rAF, abort-on-rerun, fresh ArrowResultStore per
// run) as a plain subscribable object, so each SQL tab owns its own results that
// survive tab switches. The ResultsPane subscribes to the active tab's controller.

import {
  ArrowResultStore,
  type ArrowStoreInfo,
} from "@open-lakehouse/data-grid";
import { queryRunner } from "@/lib/query/runner";

/** Metadata correlating a run to its origin and recording its outcome. Read by
 *  the ResultSession registry to track runs per environment / query file. */
export interface RunMeta {
  /** The query file this run came from (the tab id). */
  filePath: string;
  /** The SQL that was executed. */
  sql: string;
  /** Epoch ms when the run started. */
  startedAt: number;
  /** Wall-clock duration in ms, set once the run completes (or errors). */
  durationMs?: number;
  /** A snapshot of what the result store holds, set on completion. */
  info?: ArrowStoreInfo;
}

export interface RunSnapshot {
  store: ArrowResultStore | null;
  /** Bumped on each appended chunk and on completion — the re-render signal. */
  version: number;
  running: boolean;
  error: string | null;
  /** The current/last run's metadata, or null before the first run. */
  meta: RunMeta | null;
}

const EMPTY: RunSnapshot = {
  store: null,
  version: 0,
  running: false,
  error: null,
  meta: null,
};

export class RunController {
  private snapshot: RunSnapshot = EMPTY;
  private listeners = new Set<() => void>();
  private abort: AbortController | null = null;
  private raf: number | null = null;

  /** @param filePath the query file this controller's runs originate from. */
  constructor(private readonly filePath: string) {}

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
    const startedAt = Date.now();
    const meta: RunMeta = { filePath: this.filePath, sql, startedAt };
    this.snapshot = { store, version: 0, running: true, error: null, meta };
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
      // Correlate the completed run: duration + a summary of what it produced.
      this.emit({
        version: this.snapshot.version + 1,
        running: false,
        meta: {
          ...meta,
          durationMs: Date.now() - startedAt,
          info: store.inspect(),
        },
      });
    } catch (err) {
      this.cancelRaf();
      if (controller.signal.aborted) {
        this.emit({
          running: false,
          meta: { ...meta, durationMs: Date.now() - startedAt },
        });
        return;
      }
      this.emit({
        running: false,
        error: err instanceof Error ? err.message : String(err),
        meta: { ...meta, durationMs: Date.now() - startedAt },
      });
    }
  }

  /** Abort + drop listeners (on tab close). */
  dispose() {
    this.cancel();
    this.listeners.clear();
  }
}
