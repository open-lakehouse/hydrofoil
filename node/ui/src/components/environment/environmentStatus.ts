// Single source of truth for deriving an environment's lifecycle status in the
// UI. Status is computed per id (not read off a single "running" singleton at
// call sites), so this stays correct when multiple environments can run at once
// later: today only the single active environment is "running". A transient
// start/stop in flight wins over the steady-state running/idle.

import type { EnvironmentStatus } from "@/lib/client/environments";

export type EnvironmentTransition = {
  id: string;
  kind: "starting" | "stopping";
} | null;

export function environmentStatus(
  id: string,
  runningId: string | null,
  transition: EnvironmentTransition,
): EnvironmentStatus {
  if (transition?.id === id) return transition.kind;
  return runningId === id ? "running" : "idle";
}

/** Human-readable label for a status (used in badges, headers, the Status meta). */
export function statusLabel(status: EnvironmentStatus): string {
  switch (status) {
    case "running":
      return "Running";
    case "starting":
      return "Starting…";
    case "stopping":
      return "Stopping…";
    default:
      return "Idle";
  }
}
