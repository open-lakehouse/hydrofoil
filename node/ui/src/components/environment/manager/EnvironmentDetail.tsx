// The environment manager detail pane. The header carries the lifecycle action
// (a GitHub-style split button) and the derived status; it stays visible across
// all tabs. The admin tabs (External Locations, Credentials) are backed by the
// UC metastore and only work against a running environment, so they are disabled
// (with an explanatory tooltip) until the environment is running. The Overview
// tab is read-only metadata (editable config is a separate task).

import {
  Boxes,
  ChevronDown,
  CircleStop,
  KeyRound,
  Loader2,
  Play,
  TriangleAlert,
} from "lucide-react";
import { useEffect, useState } from "react";
import { Meta, MetaGrid } from "@/components/catalog/detail/Meta";
import { StorageTable } from "@/components/storage/StorageTable";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  type ActiveEnvironment,
  type Environment,
  type EnvironmentStatus,
  getEnvironmentHost,
  type KeyStatus,
} from "@/lib/client/environments";
import { cn } from "@/lib/utils";
import { ConfigureKeyDialog } from "../ConfigureKeyDialog";
import { capabilitySummary } from "../capabilitySummary";
import {
  type EnvironmentTransition,
  environmentStatus,
  statusLabel,
} from "../environmentStatus";

type TabId = "overview" | "external_locations" | "credentials";

const TABS: { id: TabId; label: string; adminOnly: boolean }[] = [
  { id: "overview", label: "Overview", adminOnly: false },
  { id: "external_locations", label: "External Locations", adminOnly: true },
  { id: "credentials", label: "Credentials", adminOnly: true },
];

export function EnvironmentDetail({
  selected,
  running,
  transition,
  lastError,
  onOpen,
  onStart,
  onLaunch,
  onStop,
}: {
  /** The environment whose card is selected in the sidebar, or null. */
  selected: Environment | null;
  /** The currently-running environment, or null when none is running. */
  running: ActiveEnvironment | null;
  /** A start/stop in flight (drives transient status), or null. */
  transition: EnvironmentTransition;
  /** Error from the last failed start/stop, or null. */
  lastError: string | null;
  /** Re-open the already-running environment (no restart). */
  onOpen: () => void;
  /** Start the environment's services and stay in the manager. */
  onStart: (id: string) => void;
  /** Start the environment's services and open the app. */
  onLaunch: (id: string) => void;
  /** Stop the environment's services. */
  onStop: (id: string) => void;
}) {
  const [tab, setTab] = useState<TabId>("overview");
  const status = selected
    ? environmentStatus(selected.id, running?.id ?? null, transition)
    : "idle";
  const isRunning = status === "running";

  // Key status is shared between the blocking banner here (above the tabs) and
  // the configurable card in the Overview tab, so configuring from the card
  // clears the banner without a refetch.
  const keyStatus = useKeyStatus(selected?.id ?? null);

  // If the selected tab requires a running environment but it is no longer
  // running (e.g. it was just stopped while viewing Credentials), fall back to
  // Overview so the pane never shows a disabled tab's content.
  useEffect(() => {
    if (!isRunning && tab !== "overview") setTab("overview");
  }, [isRunning, tab]);

  if (!selected) {
    return (
      <div className="flex h-full items-center justify-center p-8 text-sm text-muted-foreground">
        Select an environment, or create one to get started.
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center gap-2 border-b px-4 py-3">
        <Boxes className="h-5 w-5 text-muted-foreground" />
        <span className="font-medium">{selected.name}</span>
        <span className="text-xs text-muted-foreground">
          {statusLabel(status)}
        </span>
        <LifecycleButton
          id={selected.id}
          status={status}
          onOpen={onOpen}
          onStart={onStart}
          onLaunch={onLaunch}
          onStop={onStop}
        />
      </div>

      {lastError ? (
        <p className="border-b bg-destructive/5 px-4 py-1.5 text-sm text-destructive">
          {lastError}
        </p>
      ) : null}

      {keyStatus.status === "unconfigured" ||
      keyStatus.status === "unavailable" ? (
        <p className="flex items-start gap-1.5 border-b bg-destructive/5 px-4 py-1.5 text-sm text-destructive">
          <TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>
            {keyStatus.status === "unavailable"
              ? "The OS keychain is unavailable, so no encryption key could be stored. Configure a key store before starting this environment."
              : "No encryption key is configured yet. Configure one before starting this environment."}
          </span>
        </p>
      ) : null}

      <div className="flex gap-1 border-b px-2">
        {TABS.map((t) => {
          const disabled = t.adminOnly && !isRunning;
          // Use aria-disabled (not the `disabled` attribute) so the tab stays
          // natively focusable and fires hover — a real disabled <button>
          // swallows the events the tooltip needs. Clicks no-op while disabled.
          const button = (
            <button
              key={t.id}
              type="button"
              aria-disabled={disabled}
              onClick={() => {
                if (!disabled) setTab(t.id);
              }}
              className={cn(
                "border-b-2 border-transparent px-3 py-2 text-sm",
                disabled
                  ? "cursor-not-allowed text-muted-foreground/50"
                  : "text-muted-foreground hover:text-foreground",
                tab === t.id && "border-primary text-foreground",
              )}
            >
              {t.label}
            </button>
          );
          if (!disabled) return button;
          return (
            <Tooltip key={t.id}>
              <TooltipTrigger asChild>{button}</TooltipTrigger>
              <TooltipContent>
                Start this environment to manage {t.label.toLowerCase()}.
              </TooltipContent>
            </Tooltip>
          );
        })}
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {tab === "overview" ? (
          <Overview
            selected={selected}
            running={running}
            status={status}
            keyStatus={keyStatus}
          />
        ) : tab === "external_locations" ? (
          <StorageTable kind="external_location" />
        ) : (
          <StorageTable kind="credential" />
        )}
      </div>
    </div>
  );
}

// GitHub-style split button: a primary action joined to a dropdown of the
// alternative action. While a transition is in flight it collapses to a single
// disabled button reflecting the in-progress state.
function LifecycleButton({
  id,
  status,
  onOpen,
  onStart,
  onLaunch,
  onStop,
}: {
  id: string;
  status: EnvironmentStatus;
  onOpen: () => void;
  onStart: (id: string) => void;
  onLaunch: (id: string) => void;
  onStop: (id: string) => void;
}) {
  if (status === "starting" || status === "stopping") {
    return (
      <Button className="ml-auto h-7" size="sm" variant="outline" disabled>
        <Loader2 className="h-3.5 w-3.5 animate-spin" />
        {statusLabel(status)}
      </Button>
    );
  }

  // idle → primary Launch (start + open), dropdown Start (start, stay).
  // running → primary Open (view change), dropdown Stop (tear down).
  const primary =
    status === "running"
      ? { label: "Open", icon: null, onClick: onOpen }
      : {
          label: "Launch",
          icon: <Play className="h-3.5 w-3.5" />,
          onClick: () => onLaunch(id),
        };

  return (
    <div className="ml-auto flex items-center">
      <Button
        className="h-7 rounded-r-none"
        size="sm"
        onClick={primary.onClick}
      >
        {primary.icon}
        {primary.label}
      </Button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            className="h-7 rounded-l-none border-l border-primary-foreground/20 px-1.5"
            size="sm"
            aria-label="More lifecycle actions"
          >
            <ChevronDown className="h-3.5 w-3.5" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {status === "running" ? (
            <DropdownMenuItem variant="destructive" onSelect={() => onStop(id)}>
              <CircleStop />
              Stop
            </DropdownMenuItem>
          ) : (
            <DropdownMenuItem onSelect={() => onStart(id)}>
              <Play />
              Start
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

function Overview({
  selected,
  running,
  status,
  keyStatus,
}: {
  selected: Environment;
  running: ActiveEnvironment | null;
  status: EnvironmentStatus;
  keyStatus: KeyStatusState;
}) {
  const isRunning = status === "running";
  return (
    <div className="space-y-4 p-4">
      <MetaGrid>
        <Meta label="Name" value={selected.name} />
        <Meta label="Identifier" value={selected.id} mono />
        <Meta label="Status" value={statusLabel(status)} />
        {isRunning && running ? (
          <Meta label="Capabilities" value={capabilitySummary(running)} />
        ) : null}
      </MetaGrid>
      <KeyManagement
        environmentId={selected.id}
        editable={!isRunning}
        keyStatus={keyStatus}
      />
    </div>
  );
}

type KeyStatusState = {
  status: KeyStatus | null;
  setStatus: (status: KeyStatus) => void;
};

// Fetch (and hold) an environment's credential-encryption (KEK) status. Shared
// by the blocking banner and the Overview card so configuring from the card
// updates both. A null id (no selection) leaves the status unfetched.
function useKeyStatus(environmentId: string | null): KeyStatusState {
  const host = getEnvironmentHost();
  const [status, setStatus] = useState<KeyStatus | null>(null);

  useEffect(() => {
    if (!environmentId) {
      setStatus(null);
      return;
    }
    let cancelled = false;
    host
      .keyStatus(environmentId)
      .then((s) => {
        if (!cancelled) setStatus(s);
      })
      .catch(() => {
        if (!cancelled) setStatus("unavailable");
      });
    return () => {
      cancelled = true;
    };
  }, [host, environmentId]);

  return { status, setStatus };
}

// The credential-encryption (KEK) status card, with a configure affordance.
// Editing is idle-only — the key is fixed for a running environment (changing it
// would orphan already-sealed secrets), so when running we show the status
// read-only. The blocking `unconfigured`/`unavailable` warning is rendered as a
// banner above the tabs (see EnvironmentDetail), not here.
function KeyManagement({
  environmentId,
  editable,
  keyStatus,
}: {
  environmentId: string;
  editable: boolean;
  keyStatus: KeyStatusState;
}) {
  const [dialogOpen, setDialogOpen] = useState(false);
  const { status, setStatus } = keyStatus;

  return (
    <div className="space-y-2 rounded-md border p-3">
      <div className="flex items-center gap-2">
        <KeyRound className="h-4 w-4 text-muted-foreground" />
        <span className="text-sm font-medium">Encryption key</span>
        {status === "keychain" ? (
          <Badge variant="outline">OS keychain</Badge>
        ) : status === "remote" ? (
          <Badge variant="outline">Remote store</Badge>
        ) : null}
        {editable ? (
          <Button
            className="ml-auto h-7"
            size="sm"
            variant="outline"
            onClick={() => setDialogOpen(true)}
          >
            Configure key
          </Button>
        ) : null}
      </div>

      {status === "keychain" ? (
        <p className="text-xs text-muted-foreground">
          Credentials are encrypted with a key stored in your OS keychain.
        </p>
      ) : status === "remote" ? (
        <p className="text-xs text-muted-foreground">
          Credentials are encrypted with a key from a remote key store.
        </p>
      ) : (
        <p className="text-xs text-muted-foreground">
          Configure where this environment's credential-encryption key is
          stored.
        </p>
      )}

      <ConfigureKeyDialog
        environmentId={environmentId}
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        onConfigured={setStatus}
      />
    </div>
  );
}
