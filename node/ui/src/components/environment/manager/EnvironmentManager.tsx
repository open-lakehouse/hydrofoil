// The "Manage environments" overview: a master-detail layout (environment cards
// on the left, details on the right) mirroring the catalog explorer's shape.
// Reached from the in-app environment switcher's "Manage environments" item and
// shown at first run. The detail pane hosts the environment overview and — for
// the running environment — the admin tables for its UC metastore (external
// locations, credentials), since those calls only work against a live UC.
//
// Rendered OUTSIDE the active-environment provider (the running environment may be
// null here), so it is driven entirely by props rather than the active-env
// context. Wrapped in the shared dialog provider so the storage tables can drive
// the existing create/edit/delete flows.

import { useQuery } from "@tanstack/react-query";
import { Activity } from "lucide-react";
import { useState } from "react";
import { CatalogDialogsProvider } from "@/features/unity-catalog";
import {
  type ActiveEnvironment,
  type Environment,
  getEnvironmentHost,
} from "@/lib/client/environments";
import { cn } from "@/lib/utils";
import type { EnvironmentTransition } from "../environmentStatus";
import { EnvironmentDetail } from "./EnvironmentDetail";
import { EnvironmentList } from "./EnvironmentList";
import { TelemetryView } from "./TelemetryView";

// What the right detail pane shows: a per-environment detail, or the app-level
// telemetry (Jaeger) view. App-level items live in a separate rail section above
// the environment list so they read as cross-environment, not env-scoped.
type Selection = { kind: "environment"; id: string } | { kind: "telemetry" };

export function EnvironmentManager({
  running,
  transition,
  lastError,
  onOpen,
  onStart,
  onLaunch,
  onStop,
}: {
  /** The currently-running environment, or null when none is running. */
  running: ActiveEnvironment | null;
  /** A start/stop in flight (drives transient status), or null. */
  transition: EnvironmentTransition;
  /** Error from the last failed start/stop, or null. */
  lastError: string | null;
  /** Re-open the already-running environment (a view change, no restart). */
  onOpen: () => void;
  /** Start the environment's services and stay in the manager. */
  onStart: (id: string) => void;
  /** Start the environment's services and open the app (launch). */
  onLaunch: (id: string) => void;
  /** Stop the environment's services. */
  onStop: (id: string) => void;
}) {
  const host = getEnvironmentHost();
  const environments = useQuery({
    queryKey: ["environments"],
    queryFn: () => host.list(),
  });
  const envs = environments.data ?? [];

  // Default to the running environment's card, else the first. `null` means
  // "fall through to the default env"; a telemetry selection is explicit.
  const [selection, setSelection] = useState<Selection | null>(null);
  const fallbackEnvId = running?.id ?? envs[0]?.id ?? null;
  const effectiveId =
    selection?.kind === "environment" ? selection.id : fallbackEnvId;
  const showTelemetry = selection?.kind === "telemetry";
  const selected = envs.find((e) => e.id === effectiveId) ?? null;

  // A new environment is created idle (not started). Refresh the list so its
  // card appears and select it, landing the user on its detail to configure and
  // then start it deliberately.
  const handleCreated = (env: Environment) => {
    void environments.refetch();
    setSelection({ kind: "environment", id: env.id });
  };

  return (
    <CatalogDialogsProvider>
      <div className="flex h-[calc(100vh-3rem)] flex-col">
        <div className="grid min-h-0 flex-1 grid-cols-1 overflow-hidden md:grid-cols-[minmax(18rem,24rem)_minmax(0,1fr)]">
          <div className="flex min-h-0 flex-col border-r bg-sidebar">
            <AppServicesSection
              selected={showTelemetry}
              onSelect={() => setSelection({ kind: "telemetry" })}
            />
            <EnvironmentList
              environments={envs}
              isLoading={environments.isLoading}
              runningId={running?.id ?? null}
              runningSummary={running}
              transition={transition}
              selectedId={showTelemetry ? null : effectiveId}
              onSelect={(id) => setSelection({ kind: "environment", id })}
              onCreated={handleCreated}
            />
          </div>
          {showTelemetry ? (
            <TelemetryView />
          ) : (
            <EnvironmentDetail
              selected={selected}
              running={running}
              transition={transition}
              lastError={lastError}
              onOpen={onOpen}
              onStart={onStart}
              onLaunch={onLaunch}
              onStop={onStop}
            />
          )}
        </div>
      </div>
    </CatalogDialogsProvider>
  );
}

// App-level services section atop the environment list — items that span all
// environments rather than belonging to one. Today: the shared Telemetry (Jaeger)
// collector, with a live up/down dot, always visible (discoverable) but conveying
// via the dot whether its UI is available.
function AppServicesSection({
  selected,
  onSelect,
}: {
  selected: boolean;
  onSelect: () => void;
}) {
  const host = getEnvironmentHost();
  const running = useQuery({
    queryKey: ["telemetry-status"],
    queryFn: () => host.telemetryStatus(),
    // Gentle refresh so the dot reflects the collector coming up after an
    // observability-enabled environment starts.
    refetchInterval: 4000,
  });
  const up = running.data ?? false;

  return (
    <div className="border-b">
      <div className="px-3 py-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        App
      </div>
      <button
        type="button"
        onClick={onSelect}
        className={cn(
          "flex w-full items-center gap-2 px-3 py-2 text-left text-sm",
          selected
            ? "bg-accent text-accent-foreground"
            : "text-muted-foreground hover:bg-accent/50 hover:text-foreground",
        )}
      >
        <Activity className="h-4 w-4 shrink-0" />
        <span className="flex-1">Telemetry</span>
        <span
          className={cn(
            "h-2 w-2 rounded-full",
            up ? "bg-green-500" : "bg-muted-foreground/40",
          )}
          title={up ? "Collector running" : "Collector not running"}
        />
      </button>
    </div>
  );
}
