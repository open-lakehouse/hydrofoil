import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, CheckCircle2, Plus } from "lucide-react";
import { useState } from "react";
import { AppShell } from "@/components/AppShell";
import { ActiveEnvironmentProvider } from "@/components/environment/ActiveEnvironmentContext";
import { ThemeToggle } from "@/components/ThemeToggle";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  type ActiveEnvironment,
  type Environment,
  getEnvironmentHost,
} from "@/lib/client/environments";
import { cn } from "@/lib/utils";

// The persistent top header — always visible, over both the picker and the app.
// When an environment is active and the app view is showing, a back button
// returns to the environment overview WITHOUT stopping the environment.
function ShellHeader({ onBack }: { onBack?: () => void }) {
  return (
    <header className="sticky top-0 z-50 flex h-12 shrink-0 items-center justify-between border-b bg-background/80 px-4 backdrop-blur-sm">
      <div className="flex items-center gap-2">
        {onBack ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={onBack}
            className="-ml-2 h-7 gap-1.5 px-2 text-muted-foreground"
            title="Back to environments"
          >
            <ArrowLeft className="h-4 w-4" />
            Environments
          </Button>
        ) : null}
        <span className="text-sm font-semibold tracking-tight">
          Open Lakehouse
        </span>
      </div>
      <ThemeToggle />
    </header>
  );
}

// The onboarding / overview screen. Lists existing environments — highlighting
// the running one (if any) — and offers a name-only create form. Selecting the
// running environment re-opens the app without restarting it; selecting another
// switches to it (the host stops the previous one and starts the new).
function EnvironmentPicker({
  activeId,
  onOpen,
  onActivated,
}: {
  // The currently-running environment, or null when none is running.
  activeId: string | null;
  // Re-open the already-running environment (no restart).
  onOpen: () => void;
  // A (possibly different) environment was brought online — switch to the app.
  onActivated: (env: ActiveEnvironment) => void;
}) {
  const host = getEnvironmentHost();
  const environments = useQuery({
    queryKey: ["environments"],
    queryFn: () => host.list(),
  });

  const [name, setName] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function choose(env: Environment) {
    // Re-opening the running environment is a view change, not a restart.
    if (env.id === activeId) {
      onOpen();
      return;
    }
    setError(null);
    setBusy(env.id);
    try {
      const active = await host.select(env.id);
      onActivated(active);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(null);
    }
  }

  async function create() {
    const trimmed = name.trim();
    if (!trimmed) return;
    setError(null);
    setBusy("__create__");
    try {
      const env = await host.create(trimmed);
      setName("");
      // Newly created environments are selected immediately so the user lands in
      // the app — create + open is the common first-run path.
      const active = await host.select(env.id);
      onActivated(active);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(null);
    }
  }

  const envs = environments.data ?? [];

  return (
    <div className="mx-auto flex w-full max-w-md flex-col gap-6 px-4 py-16">
      <div>
        <h1 className="text-lg font-semibold">Environments</h1>
        <p className="text-sm text-muted-foreground">
          An environment bundles the local services (Unity Catalog and more).
          Select one to open it, or create a new one.
        </p>
      </div>

      {environments.isLoading ? (
        <p className="text-sm text-muted-foreground">Loading…</p>
      ) : envs.length > 0 ? (
        <ul className="space-y-2">
          {envs.map((env) => {
            const running = env.id === activeId;
            return (
              <li key={env.id}>
                <button
                  type="button"
                  disabled={busy !== null}
                  onClick={() => choose(env)}
                  className={cn(
                    "flex w-full items-center justify-between rounded-md border px-3 py-2 text-left text-sm hover:bg-accent hover:text-accent-foreground disabled:opacity-50",
                    running && "border-primary/50 bg-accent/40",
                  )}
                >
                  <span className="flex items-center gap-2 font-medium">
                    {env.name}
                    {running ? (
                      <span className="flex items-center gap-1 text-xs font-normal text-green-600 dark:text-green-500">
                        <CheckCircle2 className="h-3.5 w-3.5" />
                        Running
                      </span>
                    ) : null}
                  </span>
                  <span className="text-xs text-muted-foreground">
                    {busy === env.id ? "Starting…" : running ? "Open" : env.id}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="text-sm text-muted-foreground">
          No environments yet. Create one to get started.
        </p>
      )}

      <form
        className="flex items-end gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        <div className="flex-1 space-y-1">
          <label htmlFor="env-name" className="text-xs font-medium">
            New environment
          </label>
          <Input
            id="env-name"
            placeholder="my-environment"
            value={name}
            disabled={busy !== null}
            onChange={(e) => setName(e.target.value)}
          />
        </div>
        <Button type="submit" disabled={busy !== null || !name.trim()}>
          <Plus className="h-4 w-4" />
          Create
        </Button>
      </form>

      {error ? <p className="text-sm text-destructive">{error}</p> : null}
    </div>
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

  return (
    <div className="flex min-h-screen flex-col">
      <ShellHeader
        onBack={
          // Only offer "back" when there's a running app to step out of and the
          // host actually manages environments (no overview to return to on web).
          viewingApp && host.managed ? () => setShowApp(false) : undefined
        }
      />
      {viewingApp && running ? (
        // The app and all env-scoped state mount under the active environment.
        // Re-keying on the id unmounts/remounts the subtree on a switch, so no
        // env-A state survives into env B (see the switch protocol in the ADR).
        <ActiveEnvironmentProvider key={running.id} environment={running}>
          <AppShell />
        </ActiveEnvironmentProvider>
      ) : (
        <EnvironmentPicker
          activeId={running?.id ?? null}
          onOpen={() => setShowApp(true)}
          onActivated={(env) => {
            // Switch protocol: a newly-selected environment must not inherit the
            // previous one's server-state cache (catalogs, schemas, tables). Drop
            // it so the app re-fetches against the new environment. The
            // `key={env.id}` remount below handles component-held state
            // (per-tab run controllers, volume selection); per-env sessionStorage
            // namespacing handles the rest. Skip the clear when re-selecting the
            // already-running environment (no actual switch).
            if (env.id !== running?.id) queryClient.clear();
            setActive(env);
            setShowApp(true);
          }}
        />
      )}
    </div>
  );
}
