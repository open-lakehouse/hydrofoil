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
import { useState } from "react";
import { CatalogDialogsProvider } from "@/components/catalog/dialogs";
import {
  type ActiveEnvironment,
  type Environment,
  getEnvironmentHost,
} from "@/lib/client/environments";
import type { EnvironmentTransition } from "../environmentStatus";
import { EnvironmentDetail } from "./EnvironmentDetail";
import { EnvironmentList } from "./EnvironmentList";

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

  // Default the selected card to the running environment, else the first one.
  const [selectedId, setSelectedId] = useState<string | null>(
    running?.id ?? null,
  );
  const effectiveId = selectedId ?? running?.id ?? envs[0]?.id ?? null;
  const selected = envs.find((e) => e.id === effectiveId) ?? null;

  // A new environment is created idle (not started). Refresh the list so its
  // card appears and select it, landing the user on its detail to configure and
  // then start it deliberately.
  const handleCreated = (env: Environment) => {
    void environments.refetch();
    setSelectedId(env.id);
  };

  return (
    <CatalogDialogsProvider>
      <div className="flex h-[calc(100vh-3rem)] flex-col">
        <div className="grid min-h-0 flex-1 grid-cols-1 overflow-hidden md:grid-cols-[minmax(18rem,24rem)_minmax(0,1fr)]">
          <div className="flex min-h-0 flex-col border-r bg-sidebar">
            <EnvironmentList
              environments={envs}
              isLoading={environments.isLoading}
              runningId={running?.id ?? null}
              runningSummary={running}
              transition={transition}
              selectedId={effectiveId}
              onSelect={setSelectedId}
              onCreated={handleCreated}
            />
          </div>
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
        </div>
      </div>
    </CatalogDialogsProvider>
  );
}
