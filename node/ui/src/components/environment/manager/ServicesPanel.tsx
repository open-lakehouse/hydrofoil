// Live services panel on the environment Overview tab. While the environment is
// running, it polls the host for per-service status on a gentle interval and
// shows a colored dot per service (running/healthy, starting, down). Deliberately
// lightweight: a periodic fetch, not a streaming subscription. Polling runs only
// while mounted AND running, so it stops when the user leaves or the env stops.

import { useEffect, useState } from "react";
import {
  getEnvironmentHost,
  type ServiceStatus,
} from "@/lib/client/environments";
import { cn } from "@/lib/utils";

const POLL_MS = 4000;

// Map a service's (state, health) to a dot color + label. Health wins when the
// service declares a healthcheck; otherwise we fall back to the run state.
function presentation(s: ServiceStatus): { color: string; label: string } {
  if (s.health === "healthy")
    return { color: "bg-green-500", label: "healthy" };
  if (s.health === "starting")
    return { color: "bg-amber-500", label: "starting" };
  if (s.health === "unhealthy")
    return { color: "bg-destructive", label: "unhealthy" };
  if (s.state === "running") return { color: "bg-green-500", label: "running" };
  if (s.state === "restarting")
    return { color: "bg-amber-500", label: "restarting" };
  return { color: "bg-muted-foreground/50", label: s.state || "stopped" };
}

export function ServicesPanel({ environmentId }: { environmentId: string }) {
  const host = getEnvironmentHost();
  const [services, setServices] = useState<ServiceStatus[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const tick = async () => {
      try {
        const list = await host.serviceStatus(environmentId);
        if (!cancelled) setServices(list);
      } catch {
        if (!cancelled) setServices([]);
      }
      // Re-arm only if still mounted (self-scheduling avoids overlapping calls
      // if one is slow).
      if (!cancelled) timer = setTimeout(tick, POLL_MS);
    };
    tick();

    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [host, environmentId]);

  // Nothing reported (no compose project up, or Docker absent) → render nothing,
  // so the panel only appears when there's something to show.
  if (!services || services.length === 0) return null;

  return (
    <div className="space-y-2 rounded-md border p-3">
      <span className="text-sm font-medium">Services</span>
      <ul className="space-y-1">
        {services.map((s) => {
          const { color, label } = presentation(s);
          return (
            <li
              key={`${s.shared ? "shared:" : ""}${s.service}`}
              className="flex items-center gap-2 text-sm"
            >
              <span className={cn("h-2 w-2 shrink-0 rounded-full", color)} />
              <span className="font-mono text-xs">{s.service}</span>
              {s.shared ? (
                <span className="rounded bg-muted px-1 text-[10px] text-muted-foreground">
                  shared
                </span>
              ) : null}
              <span className="ml-auto text-xs text-muted-foreground">
                {label}
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
