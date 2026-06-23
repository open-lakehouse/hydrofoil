import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { AppShell } from "@/components/AppShell";
import { ActiveEnvironmentProvider } from "@/components/environment/ActiveEnvironmentContext";
import { EnvironmentSwitcher } from "@/components/environment/EnvironmentSwitcher";
import { EnvironmentManager } from "@/components/environment/manager/EnvironmentManager";
import { ThemeToggle } from "@/components/ThemeToggle";
import {
  type ActiveEnvironment,
  getEnvironmentHost,
} from "@/lib/client/environments";

// The persistent top header — always visible, over both the picker and the app.
// In the app view (on managed hosts) the active-environment switcher sits to the
// right of the label: a chip summarizing the running environment that also
// switches environments and offers "Manage environments" (returns to the
// overview WITHOUT stopping the running environment).
function ShellHeader({
  active,
  onSwitch,
  onManage,
}: {
  active?: ActiveEnvironment | null;
  onSwitch?: (id: string) => Promise<void>;
  onManage?: () => void;
}) {
  const showSwitcher =
    active && onSwitch && onManage && getEnvironmentHost().managed;
  return (
    <header className="sticky top-0 z-50 flex h-12 shrink-0 items-center justify-between border-b bg-background/80 px-4 backdrop-blur-sm">
      <div className="flex items-center gap-3">
        <span className="text-sm font-semibold tracking-tight">
          Open Lakehouse
        </span>
        {showSwitcher ? (
          <EnvironmentSwitcher
            active={active}
            onSwitch={onSwitch}
            onManage={onManage}
          />
        ) : null}
      </div>
      <ThemeToggle />
    </header>
  );
}

// The outer shell: persistent header + either the environment overview or the
// inner app. Two independent pieces of state:
//   - `activeId`: which environment is running (services bound). Set only by
//     selecting one; going back to the overview does NOT clear it.
//   - `view`: whether the overview or the app is showing.
// Going back from the app keeps the environment running; re-opening it from the
// overview is a pure view change (no restart).
//
// On hosts that don't manage environments (the web build), the default host
// reports an active id and the overview is never shown.
export function EnvironmentGate() {
  const host = getEnvironmentHost();
  const queryClient = useQueryClient();
  // The active environment at startup (the host may have activated one via an
  // escape hatch, e.g. OPEN_LAKEHOUSE_UC_URL). Drives the initial view.
  const initial = useQuery({
    queryKey: ["environment-active"],
    queryFn: () => host.active(),
  });

  // Local overrides layered over the startup query: the running environment and
  // whether the app (vs. the overview) is showing. `undefined` = defer to query.
  const [active, setActive] = useState<ActiveEnvironment | null | undefined>(
    undefined,
  );
  const [showApp, setShowApp] = useState<boolean | undefined>(undefined);

  if (initial.isLoading) {
    return (
      <div className="flex min-h-screen flex-col">
        <ShellHeader />
        <div className="px-4 py-16 text-sm text-muted-foreground">Loading…</div>
      </div>
    );
  }

  const running = active !== undefined ? active : (initial.data ?? null);
  // Default the view to the app when something is running at startup, else the
  // overview; explicit navigation overrides.
  const viewingApp = showApp !== undefined ? showApp : running !== null;

  // Adopt a brought-online environment as the running one and show the app.
  // Switch protocol: a newly-selected environment must not inherit the previous
  // one's server-state cache (catalogs, schemas, tables) — drop it so the app
  // re-fetches against the new environment. The `key={env.id}` remount handles
  // component-held state (per-tab run controllers, volume selection); per-env
  // sessionStorage namespacing handles the rest. Skip the clear when this is the
  // already-running environment (re-open, not a switch).
  const adopt = (env: ActiveEnvironment) => {
    if (env.id !== running?.id) queryClient.clear();
    setActive(env);
    setShowApp(true);
  };

  // Switch directly to another environment from the in-app switcher: bring it
  // online via the host, then adopt it.
  const switchTo = async (id: string) => {
    if (id === running?.id) return;
    adopt(await host.select(id));
  };

  return (
    <div className="flex min-h-screen flex-col">
      <ShellHeader
        active={viewingApp ? running : null}
        onSwitch={switchTo}
        // "Manage environments" returns to the overview WITHOUT stopping the
        // running environment (it stays highlighted there).
        onManage={() => setShowApp(false)}
      />
      {viewingApp && running ? (
        // The app and all env-scoped state mount under the active environment.
        // Re-keying on the id unmounts/remounts the subtree on a switch, so no
        // env-A state survives into env B (see the switch protocol in the ADR).
        <ActiveEnvironmentProvider key={running.id} environment={running}>
          <AppShell />
        </ActiveEnvironmentProvider>
      ) : (
        <EnvironmentManager
          running={running}
          onOpen={() => setShowApp(true)}
          onActivated={adopt}
        />
      )}
    </div>
  );
}
