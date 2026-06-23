// The environment manager detail pane. For the running environment it shows an
// Overview plus admin tabs (External Locations, Credentials) backed by the UC
// metastore — the storage securables that used to live in the catalog explorer's
// "External data" section now live here, where they belong (they are
// metastore-scoped admin resources, and admin calls only work against the running
// environment). For any other selected environment only the Overview is shown,
// with a primary action to open/switch to it (which brings it online and mounts
// the app under it). Admin tabs for a non-running environment are gated behind
// that switch, since its UC services are not online.

import { Boxes, Play } from "lucide-react";
import { useState } from "react";
import { Meta, MetaGrid } from "@/components/catalog/detail/Meta";
import { StorageTable } from "@/components/storage/StorageTable";
import { Button } from "@/components/ui/button";
import {
  type ActiveEnvironment,
  type Environment,
  getEnvironmentHost,
} from "@/lib/client/environments";
import { cn } from "@/lib/utils";
import { capabilitySummary } from "../capabilitySummary";

type TabId = "overview" | "external_locations" | "credentials";

const TABS: { id: TabId; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "external_locations", label: "External Locations" },
  { id: "credentials", label: "Credentials" },
];

export function EnvironmentDetail({
  selected,
  running,
  onOpen,
  onActivated,
}: {
  /** The environment whose card is selected in the sidebar, or null. */
  selected: Environment | null;
  /** The currently-running environment, or null when none is running. */
  running: ActiveEnvironment | null;
  /** Re-open the already-running environment (no restart). */
  onOpen: () => void;
  /** Bring a (possibly different) environment online and switch to the app. */
  onActivated: (env: ActiveEnvironment) => void;
}) {
  const [tab, setTab] = useState<TabId>("overview");

  if (!selected) {
    return (
      <div className="flex h-full items-center justify-center p-8 text-sm text-muted-foreground">
        Select an environment, or create one to get started.
      </div>
    );
  }

  const isRunning = selected.id === running?.id;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center gap-2 border-b px-4 py-3">
        <Boxes className="h-5 w-5 text-muted-foreground" />
        <span className="font-medium">{selected.name}</span>
        <span className="text-xs text-muted-foreground">
          {isRunning ? "Running" : "Stopped"}
        </span>
        {/* Launch is kept in the header (accent color for prominence, fixed
            height so the row keeps its original height) so it stays visible
            across all tabs. */}
        {isRunning ? (
          <Button className="ml-auto h-7" size="sm" onClick={onOpen}>
            <Play className="h-3.5 w-3.5" />
            Launch
          </Button>
        ) : null}
      </div>

      <div className="flex gap-1 border-b px-2">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            onClick={() => setTab(t.id)}
            className={cn(
              "border-b-2 border-transparent px-3 py-2 text-sm text-muted-foreground hover:text-foreground",
              tab === t.id && "border-primary text-foreground",
            )}
          >
            {t.label}
          </button>
        ))}
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {tab === "overview" ? (
          <Overview
            selected={selected}
            running={running}
            isRunning={isRunning}
            onActivated={onActivated}
          />
        ) : !isRunning ? (
          <div className="p-4">
            <SwitchNotice
              selected={selected}
              running={running}
              onActivated={onActivated}
            />
          </div>
        ) : tab === "external_locations" ? (
          <StorageTable kind="external_location" />
        ) : (
          <StorageTable kind="credential" />
        )}
      </div>
    </div>
  );
}

function Overview({
  selected,
  running,
  isRunning,
  onActivated,
}: {
  selected: Environment;
  running: ActiveEnvironment | null;
  isRunning: boolean;
  onActivated: (env: ActiveEnvironment) => void;
}) {
  return (
    <div className="space-y-4 p-4">
      <MetaGrid>
        <Meta label="Name" value={selected.name} />
        <Meta label="Identifier" value={selected.id} mono />
        <Meta label="Status" value={isRunning ? "Running" : "Stopped"} />
        {isRunning && running ? (
          <Meta label="Capabilities" value={capabilitySummary(running)} />
        ) : null}
      </MetaGrid>
      {/* The running env's open action lives in the header; only the
          not-running call-to-action remains in the overview body. */}
      {isRunning ? null : (
        <SwitchNotice
          selected={selected}
          running={running}
          onActivated={onActivated}
        />
      )}
    </div>
  );
}

function SwitchNotice({
  selected,
  running,
  onActivated,
}: {
  selected: Environment;
  running: ActiveEnvironment | null;
  onActivated: (env: ActiveEnvironment) => void;
}) {
  const host = getEnvironmentHost();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const label = running
    ? "Switch to this environment"
    : "Open this environment";

  return (
    <div className="space-y-3 rounded-md border bg-muted/20 p-4">
      <p className="text-sm text-muted-foreground">
        This environment is not running. {label} to browse its catalog and
        manage its external locations and credentials.
      </p>
      <Button
        disabled={busy}
        onClick={async () => {
          setError(null);
          setBusy(true);
          try {
            onActivated(await host.select(selected.id));
          } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
            setBusy(false);
          }
        }}
      >
        {busy ? "Starting…" : label}
      </Button>
      {error ? <p className="text-sm text-destructive">{error}</p> : null}
    </div>
  );
}
