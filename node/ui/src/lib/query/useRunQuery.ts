import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowResultStore } from "./arrowResultStore";
import { queryRunner } from "./runner";

/**
 * State for a streaming query run. The decoded results live in `store` (an
 * `ArrowResultStore` that holds the Arrow batches and serves cells zero-copy);
 * `version` is bumped as chunks arrive so consumers re-render without us ever
 * copying or spreading the Arrow data. `store` keeps a stable identity across
 * a run — mutate-in-place + version bump, not immutable replacement.
 */
export interface RunQueryState {
  store: ArrowResultStore | null;
  /** Increments on each appended chunk; the re-render / invalidation signal. */
  version: number;
  /** True while the stream is open. */
  running: boolean;
  error: string | null;
}

/**
 * Run a SQL query through the pluggable `queryRunner` and accumulate the decoded
 * Arrow batches in an `ArrowResultStore`, re-rendering as chunks arrive
 * (progressive rendering) by bumping a version counter. The version bump is
 * coalesced to one per animation frame so a burst of small batches doesn't cause
 * a render per chunk.
 *
 * Execution is host-pluggable: this hook depends only on `queryRunner` (see
 * lib/query/runner.ts), never on the ConnectRPC client directly, so a Tauri host
 * can swap in a different execution mechanism.
 *
 * Returns the current state plus `run(sql, limit)` and `cancel()`. Re-running
 * aborts any in-flight stream first; `cancel()` aborts and tears down the
 * server-side query.
 */
export function useRunQuery() {
  const [state, setState] = useState<RunQueryState>({
    store: null,
    version: 0,
    running: false,
    error: null,
  });
  const abortRef = useRef<AbortController | null>(null);
  // Pending rAF handle for coalescing version bumps during fast streaming.
  const flushRef = useRef<number | null>(null);

  const cancelFlush = useCallback(() => {
    if (flushRef.current !== null) {
      cancelAnimationFrame(flushRef.current);
      flushRef.current = null;
    }
  }, []);

  const cancel = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    cancelFlush();
    setState((s) => ({ ...s, running: false }));
  }, [cancelFlush]);

  // Tear down any in-flight stream / pending flush on unmount.
  useEffect(() => {
    return () => {
      abortRef.current?.abort();
      cancelFlush();
    };
  }, [cancelFlush]);

  const run = useCallback(
    async (sql: string, limit?: number) => {
      // Abort any previous in-flight stream before starting a new one.
      abortRef.current?.abort();
      cancelFlush();
      const controller = new AbortController();
      abortRef.current = controller;

      // Fresh store per run so the previous one (and its Arrow buffers) is GC'd.
      const store = new ArrowResultStore();
      setState({ store, version: 0, running: true, error: null });

      // Coalesce version bumps: schedule at most one re-render per frame.
      const scheduleFlush = () => {
        if (flushRef.current !== null) return;
        flushRef.current = requestAnimationFrame(() => {
          flushRef.current = null;
          if (controller.signal.aborted) return;
          setState((s) => ({ ...s, version: s.version + 1 }));
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
        // Final flush so the last batch is reflected even if a frame was pending.
        cancelFlush();
        setState((s) => ({ ...s, version: s.version + 1, running: false }));
      } catch (err) {
        cancelFlush();
        // An aborted stream is a user action, not an error.
        if (controller.signal.aborted) {
          setState((s) => ({ ...s, running: false }));
          return;
        }
        setState((s) => ({
          ...s,
          running: false,
          error: err instanceof Error ? err.message : String(err),
        }));
      }
    },
    [cancelFlush],
  );

  return { ...state, run, cancel };
}
