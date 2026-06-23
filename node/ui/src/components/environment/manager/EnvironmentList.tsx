// The environment manager sidebar: a list of environment cards plus a header
// "New" affordance that opens the create dialog (mirroring the catalog view). The
// running environment is highlighted; cards carry the data we can show without
// bringing an environment online (name + status) and, for the running one, a
// capability summary. Selecting a card is a pure view change — it picks which
// environment's detail the right pane shows; it does NOT switch the running
// environment (that is an explicit action in the detail pane).

import { CheckCircle2, Circle, Loader2, Plus } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import type {
  ActiveEnvironment,
  Environment,
  EnvironmentStatus,
} from "@/lib/client/environments";
import { cn } from "@/lib/utils";
import { CreateEnvironmentDialog } from "../CreateEnvironmentDialog";
import { capabilitySummary } from "../capabilitySummary";
import {
  type EnvironmentTransition,
  environmentStatus,
  statusLabel,
} from "../environmentStatus";

export function EnvironmentList({
  environments,
  isLoading,
  runningId,
  runningSummary,
  transition,
  selectedId,
  onSelect,
  onCreated,
}: {
  environments: Environment[];
  isLoading: boolean;
  /** The running environment's id, or null when none is running. */
  runningId: string | null;
  /** The running environment (for its capability summary), or null. */
  runningSummary: ActiveEnvironment | null;
  /** A start/stop in flight (drives transient status), or null. */
  transition: EnvironmentTransition;
  /** Which card is shown in the detail pane. */
  selectedId: string | null;
  onSelect: (id: string) => void;
  /** A new (idle, not-started) environment was created. */
  onCreated: (env: Environment) => void;
}) {
  const [creating, setCreating] = useState(false);

  return (
    <div className="flex min-h-0 flex-col">
      <div className="flex items-center justify-between border-b px-3 py-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        Environments
        <Button
          variant="ghost"
          size="sm"
          className="h-6 px-1.5 text-xs"
          onClick={() => setCreating(true)}
        >
          <Plus className="h-3.5 w-3.5" />
          New
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-auto p-2">
        {isLoading ? (
          <p className="px-1 py-2 text-sm text-muted-foreground">Loading…</p>
        ) : environments.length === 0 ? (
          <p className="px-1 py-2 text-sm text-muted-foreground">
            No environments yet. Create one to get started.
          </p>
        ) : (
          <ul className="space-y-2">
            {environments.map((env) => (
              <EnvironmentCard
                key={env.id}
                env={env}
                status={environmentStatus(env.id, runningId, transition)}
                selected={env.id === selectedId}
                summary={
                  env.id === runningId && runningSummary
                    ? capabilitySummary(runningSummary)
                    : null
                }
                onSelect={() => onSelect(env.id)}
              />
            ))}
          </ul>
        )}
      </div>

      {creating ? (
        <CreateEnvironmentDialog
          onClose={() => setCreating(false)}
          onCreated={onCreated}
        />
      ) : null}
    </div>
  );
}

function EnvironmentCard({
  env,
  status,
  selected,
  summary,
  onSelect,
}: {
  env: Environment;
  status: EnvironmentStatus;
  selected: boolean;
  summary: string | null;
  onSelect: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={onSelect}
        className={cn(
          "flex w-full flex-col gap-1 rounded-md border px-3 py-2 text-left hover:bg-accent hover:text-accent-foreground",
          selected && "border-primary/50 bg-accent/40",
        )}
      >
        <span className="flex items-center justify-between">
          <span className="font-medium">{env.name}</span>
          <StatusBadge status={status} />
        </span>
        <span className="text-xs text-muted-foreground">
          {summary ?? env.id}
        </span>
      </button>
    </li>
  );
}

function StatusBadge({ status }: { status: EnvironmentStatus }) {
  const label = statusLabel(status);
  if (status === "running") {
    return (
      <span className="flex items-center gap-1 text-xs font-normal text-green-600 dark:text-green-500">
        <CheckCircle2 className="h-3.5 w-3.5" />
        {label}
      </span>
    );
  }
  if (status === "starting" || status === "stopping") {
    return (
      <span className="flex items-center gap-1 text-xs font-normal text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" />
        {label}
      </span>
    );
  }
  return (
    <span className="flex items-center gap-1 text-xs font-normal text-muted-foreground">
      <Circle className="h-3 w-3" />
      {label}
    </span>
  );
}
