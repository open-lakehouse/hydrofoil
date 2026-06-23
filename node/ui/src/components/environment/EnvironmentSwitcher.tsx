// The active-environment chip + dropdown shown in the app header (right of the
// "Open Lakehouse" label). It surfaces the data we have on the running
// environment — name + capability summary — and lets the user switch to another
// environment or return to the environment overview ("Manage environments").
//
// Only rendered in the app view on managed hosts; the web build has a single
// implicit environment, so there is nothing to switch.

import { useQuery } from "@tanstack/react-query";
import { Boxes, ChevronDown, HardDrive, Settings2 } from "lucide-react";
import { useState } from "react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  type ActiveEnvironment,
  getEnvironmentHost,
} from "@/lib/client/environments";
import { capabilitySummary } from "./capabilitySummary";

export function EnvironmentSwitcher({
  active,
  onSwitch,
  onManage,
}: {
  /** The currently-running environment. */
  active: ActiveEnvironment;
  /** Switch to another environment (brings it online, then swaps the app). */
  onSwitch: (id: string) => Promise<void>;
  /** Return to the environment overview without stopping the running one. */
  onManage: () => void;
}) {
  const host = getEnvironmentHost();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);

  // The full environment list, so the menu can offer the others to switch to.
  const environments = useQuery({
    queryKey: ["environments"],
    queryFn: () => host.list(),
    enabled: open,
  });
  const others = (environments.data ?? []).filter((e) => e.id !== active.id);

  async function switchTo(id: string) {
    setBusy(id);
    try {
      await onSwitch(id);
      setOpen(false);
    } finally {
      setBusy(null);
    }
  }

  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="flex h-7 items-center gap-2 rounded-md border bg-background px-2 text-sm hover:bg-accent hover:text-accent-foreground"
          title="Active environment"
        >
          <Boxes className="h-3.5 w-3.5 text-muted-foreground" />
          <span className="font-medium">{active.name}</span>
          <span className="hidden text-xs text-muted-foreground sm:inline">
            {capabilitySummary(active)}
          </span>
          <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-[16rem]">
        <DropdownMenuLabel className="flex flex-col gap-0.5">
          <span>{active.name}</span>
          <span className="flex items-center gap-1 text-xs font-normal text-muted-foreground">
            <HardDrive className="h-3 w-3" />
            {capabilitySummary(active)}
          </span>
        </DropdownMenuLabel>

        {others.length > 0 && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
              Switch environment
            </DropdownMenuLabel>
            {others.map((env) => (
              <DropdownMenuItem
                key={env.id}
                disabled={busy !== null}
                onSelect={(e) => {
                  // Keep the menu open while the switch runs; close on success.
                  e.preventDefault();
                  void switchTo(env.id);
                }}
              >
                <span className="flex-1">{env.name}</span>
                {busy === env.id ? (
                  <span className="text-xs text-muted-foreground">
                    Starting…
                  </span>
                ) : null}
              </DropdownMenuItem>
            ))}
          </>
        )}

        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={onManage}>
          <Settings2 className="h-4 w-4" />
          Manage environments
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
