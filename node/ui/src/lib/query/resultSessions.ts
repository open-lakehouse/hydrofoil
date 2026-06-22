// Environment-scoped registry of query-result sessions.
//
// Each open SQL tab runs against one RunController (per-tab Arrow results that
// survive tab switches). This registry OWNS those controllers for a single
// environment and correlates each to its query file — so results are scoped to
// an environment (disposed when it is torn down on a switch) and a future view
// can enumerate "what has this environment run, and what's in each result".
//
// It is deliberately host-agnostic and in-memory. Cross-session persistence
// (Tauri-host-backed Arrow IPC files, or IndexedDB on web) plugs in here later:
// the registry is the seam that would hydrate/persist controllers.

import { RunController, type RunMeta } from "@/lib/editor/runController";

export class ResultSessionRegistry {
  private controllers = new Map<string, RunController>();

  /** The RunController for a query file, created on first request. */
  controller(filePath: string): RunController {
    let ctrl = this.controllers.get(filePath);
    if (!ctrl) {
      ctrl = new RunController(filePath);
      this.controllers.set(filePath, ctrl);
    }
    return ctrl;
  }

  /** Drop a single query file's controller (e.g. on tab close), aborting any
   *  in-flight stream. */
  release(filePath: string): void {
    this.controllers.get(filePath)?.dispose();
    this.controllers.delete(filePath);
  }

  /** A snapshot of every tracked run's metadata — the "what has run here" view.
   *  Skips controllers that have never run (no meta yet). */
  list(): RunMeta[] {
    const out: RunMeta[] = [];
    for (const ctrl of this.controllers.values()) {
      const meta = ctrl.get().meta;
      if (meta) out.push(meta);
    }
    return out;
  }

  /** Abort + drop every controller (on environment teardown). */
  dispose(): void {
    for (const ctrl of this.controllers.values()) ctrl.dispose();
    this.controllers.clear();
  }
}
