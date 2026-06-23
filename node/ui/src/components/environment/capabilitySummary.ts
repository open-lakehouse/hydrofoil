// A one-line summary of what an environment provides, derived from its
// capabilities and built-in volumes (the correlated data the client holds for an
// active environment). Shared by the header switcher chip and the environment
// management cards/detail so both describe an environment the same way.

import type { ActiveEnvironment } from "@/lib/client/environments";
import { HOME_VOLUME } from "@/lib/editor/volumes";

export function capabilitySummary(env: ActiveEnvironment): string {
  const parts: string[] = [];
  if (env.capabilities.hasHome) parts.push("Home");
  const ucCount = env.volumes.filter((v) => v.id !== HOME_VOLUME.id).length;
  if (ucCount > 0)
    parts.push(`${ucCount} UC volume${ucCount === 1 ? "" : "s"}`);
  return parts.length > 0 ? parts.join(" · ") : "No local volumes";
}
