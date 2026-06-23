// The environment manager detail pane. The header carries the lifecycle action
// (a GitHub-style split button) and the derived status; it stays visible across
// all tabs. The admin tabs (External Locations, Credentials) are backed by the
// UC metastore and only work against a running environment, so they are disabled
// (with an explanatory tooltip) until the environment is running. The Overview
// tab is read-only metadata (editable config is a separate task).

import { Boxes, ChevronDown, CircleStop, Loader2, Play } from "lucide-react";
import { useEffect, useState } from "react";
import { Meta, MetaGrid } from "@/components/catalog/detail/Meta";
import { StorageTable } from "@/components/storage/StorageTable";
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
import type {
  ActiveEnvironment,
  Environment,
  EnvironmentStatus,
} from "@/lib/client/environments";
import { cn } from "@/lib/utils";
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
          <Overview selected={selected} running={running} status={status} />
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
}: {
  selected: Environment;
  running: ActiveEnvironment | null;
  status: EnvironmentStatus;
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
    </div>
  );
}
