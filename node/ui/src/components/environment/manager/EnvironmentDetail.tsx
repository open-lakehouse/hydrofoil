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
  Fingerprint,
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
import { CapabilitiesCard } from "./CapabilitiesCard";
import { ConfigViewer } from "./ConfigViewer";
import { ServicesPanel } from "./ServicesPanel";

type TabId = "overview" | "config" | "external_locations" | "credentials";

const TABS: { id: TabId; label: string; adminOnly: boolean }[] = [
  { id: "overview", label: "Overview", adminOnly: false },
  { id: "config", label: "Config", adminOnly: false },
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

  // Docker availability gates the capability checklist (all capabilities need
  // Docker today) and drives the install-hint banner.
  const docker = useDockerStatus();

  // If the selected tab is an admin tab requiring a running environment but it is
  // no longer running (e.g. stopped while viewing Credentials), fall back to
  // Overview. Non-admin tabs (Config) stay available when idle.
  useEffect(() => {
    const current = TABS.find((t) => t.id === tab);
    if (current?.adminOnly && !isRunning) setTab("overview");
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

      {docker === false ? <DockerBanner /> : null}

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
            dockerAvailable={docker !== false}
          />
        ) : tab === "config" ? (
          <ConfigViewer environmentId={selected.id} />
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
  dockerAvailable,
}: {
  selected: Environment;
  running: ActiveEnvironment | null;
  status: EnvironmentStatus;
  keyStatus: KeyStatusState;
  dockerAvailable: boolean;
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
      {isRunning ? <ServicesPanel environmentId={selected.id} /> : null}
      <CapabilitiesCard
        environmentId={selected.id}
        editable={!isRunning}
        dockerAvailable={dockerAvailable}
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

// Whether the host's container runtime is available. `null` while the (single)
// probe is in flight. Drives the capability checklist's enabled state and the
// install-hint banner.
function useDockerStatus(): boolean | null {
  const host = getEnvironmentHost();
  const [available, setAvailable] = useState<boolean | null>(null);
  useEffect(() => {
    let cancelled = false;
    host
      .dockerStatus()
      .then((ok) => {
        if (!cancelled) setAvailable(ok);
      })
      .catch(() => {
        if (!cancelled) setAvailable(false);
      });
    return () => {
      cancelled = true;
    };
  }, [host]);
  return available;
}

// Non-blocking banner shown when Docker isn't available: capabilities need it.
// Mirrors the KEK banner; an expandable details section gives OS-aware install
// hints (Homebrew on macOS) without being verbose.
function DockerBanner() {
  const [open, setOpen] = useState(false);
  const mac = isMac();
  return (
    <div className="border-b bg-destructive/5 px-4 py-1.5 text-sm text-destructive">
      <div className="flex items-start gap-1.5">
        <TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
        <span className="flex-1">
          Docker isn't running. Capabilities (MLflow, lineage, …) need it.
        </span>
        <button
          type="button"
          className="shrink-0 underline underline-offset-2 hover:no-underline"
          onClick={() => setOpen((o) => !o)}
        >
          {open ? "Hide" : "How to install"}
        </button>
      </div>
      {open ? (
        <div className="mt-1.5 ml-5 space-y-1 text-xs text-muted-foreground">
          {mac ? (
            <>
              <p>Install Docker Desktop, then start it:</p>
              <code className="block rounded bg-muted px-1.5 py-0.5 font-mono text-foreground">
                brew install --cask docker
              </code>
            </>
          ) : (
            <p>
              Install Docker Desktop from{" "}
              <span className="font-mono">docker.com/get-started</span> and
              start it.
            </p>
          )}
          <p>Then reopen this environment.</p>
        </div>
      ) : null}
    </div>
  );
}

// Best-effort OS sniff for the install hint (no Tauri import in node/ui — we read
// the UA, which is sufficient to pick the macOS vs. generic message).
function isMac(): boolean {
  if (typeof navigator === "undefined") return false;
  return /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
}

// The credential-encryption (KEK) status card, with a configure affordance.
// Editing is idle-only — the key is fixed for a running environment (changing it
// would orphan already-sealed secrets), so when running we show the status
// read-only. The blocking `unconfigured`/`unavailable` warning is rendered as a
// banner above the tabs (see EnvironmentDetail), not here.
//
// Once a key exists the storage provider is locked (the key is minted once and
// never rotated), so "Configure key" only appears while `unconfigured`. Touch ID,
// by contrast, is an in-place toggle on the same key bytes and stays available
// (idle-only, macOS only).
function KeyManagement({
  environmentId,
  editable,
  keyStatus,
}: {
  environmentId: string;
  editable: boolean;
  keyStatus: KeyStatusState;
}) {
  const host = getEnvironmentHost();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [bioBusy, setBioBusy] = useState(false);
  const [bioError, setBioError] = useState<string | null>(null);
  const { status, setStatus } = keyStatus;

  const isKeychain = status === "keychain" || status === "keychain-biometric";
  const biometricOn = status === "keychain-biometric";
  // The Touch ID toggle is offered only for a keychain key, while idle, on a host
  // that supports biometry. The provider can't be changed after creation, so a
  // configured keychain key never reverts to the configure dialog.
  const canToggleBiometric = editable && isKeychain && host.biometricSupported;

  async function toggleBiometric() {
    setBioError(null);
    setBioBusy(true);
    try {
      // Reading the existing biometric key prompts for Touch ID; the OS dialog is
      // the confirmation, so no extra UI needed.
      const next = await host.setKeyBiometric(environmentId, !biometricOn);
      setStatus(next);
    } catch (e) {
      setBioError(e instanceof Error ? e.message : String(e));
    } finally {
      setBioBusy(false);
    }
  }

  return (
    <div className="space-y-2 rounded-md border p-3">
      <div className="flex items-center gap-2">
        <KeyRound className="h-4 w-4 text-muted-foreground" />
        <span className="text-sm font-medium">Encryption key</span>
        {biometricOn ? (
          <Badge variant="outline">
            <Fingerprint className="mr-1 h-3 w-3" />
            Touch ID
          </Badge>
        ) : status === "keychain" ? (
          <Badge variant="outline">OS keychain</Badge>
        ) : status === "remote" ? (
          <Badge variant="outline">Remote store</Badge>
        ) : null}
        {/* Initial provisioning only — the provider locks once a key exists. */}
        {editable && (status === "unconfigured" || status === "unavailable") ? (
          <Button
            className="ml-auto h-7"
            size="sm"
            variant="outline"
            onClick={() => setDialogOpen(true)}
          >
            Configure key
          </Button>
        ) : null}
        {canToggleBiometric ? (
          <Button
            className="ml-auto h-7"
            size="sm"
            variant={biometricOn ? "secondary" : "outline"}
            disabled={bioBusy}
            onClick={toggleBiometric}
          >
            <Fingerprint className="mr-1 h-3.5 w-3.5" />
            {bioBusy
              ? "Updating…"
              : biometricOn
                ? "Disable Touch ID"
                : "Require Touch ID"}
          </Button>
        ) : null}
      </div>

      {biometricOn ? (
        <p className="text-xs text-muted-foreground">
          Credentials are encrypted with a key in your OS keychain, unlocked by
          Touch ID. Starting this environment prompts for your fingerprint.
        </p>
      ) : status === "keychain" ? (
        <p className="text-xs text-muted-foreground">
          Credentials are encrypted with a key stored in your OS keychain.
          {host.biometricSupported && editable
            ? " Require Touch ID to unlock it on each start."
            : null}
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

      {bioError ? <p className="text-xs text-destructive">{bioError}</p> : null}

      <ConfigureKeyDialog
        environmentId={environmentId}
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        onConfigured={setStatus}
      />
    </div>
  );
}
