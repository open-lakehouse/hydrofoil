// The capabilities card on the environment Overview tab. A checklist of the
// service modules a user can enable (lineage/headwaters, model tracking/mlflow,
// object storage/azurite) plus a separate Observability opt-in; toggling persists
// via the host and takes effect on the next start.
//
// All of these currently require Docker (the modules run as containers, or — for
// observability — emit to the shared Docker-run collector), so when the host
// reports Docker unavailable the whole card is disabled and a banner (rendered by
// EnvironmentDetail) explains how to install it. Editing is idle-only: a running
// environment isn't hot-reconfigured.

import { Boxes, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { type EnvModule, getEnvironmentHost } from "@/lib/client/environments";
import { cn } from "@/lib/utils";

export function CapabilitiesCard({
  environmentId,
  editable,
  dockerAvailable,
}: {
  environmentId: string;
  /** Idle-only: a running environment's capabilities are read-only. */
  editable: boolean;
  /** When false, all capabilities are disabled (they need Docker). */
  dockerAvailable: boolean;
}) {
  const host = getEnvironmentHost();
  const [available, setAvailable] = useState<EnvModule[] | null>(null);
  const [enabled, setEnabled] = useState<Set<string>>(new Set());
  const [observability, setObservability] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([
      host.availableModules(),
      host.environmentModules(environmentId),
      host.environmentObservability(environmentId),
    ])
      .then(([all, on, obs]) => {
        if (cancelled) return;
        setAvailable(all);
        setEnabled(new Set(on));
        setObservability(obs);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [host, environmentId]);

  const toggle = async (id: string) => {
    if (!editable || saving) return;
    // Optimistic: flip locally, persist the full set, roll back on failure.
    const next = new Set(enabled);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    const prev = enabled;
    setEnabled(next);
    setSaving(true);
    setError(null);
    try {
      await host.setEnvironmentModules(environmentId, [...next]);
    } catch (e) {
      setEnabled(prev);
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const toggleObservability = async () => {
    if (!editable || saving) return;
    const next = !observability;
    const prev = observability;
    setObservability(next);
    setSaving(true);
    setError(null);
    try {
      await host.setEnvironmentObservability(environmentId, next);
    } catch (e) {
      setObservability(prev);
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const disabled = !editable || !dockerAvailable || saving;

  return (
    <div className="space-y-2 rounded-md border p-3">
      <div className="flex items-center gap-2">
        <Boxes className="h-4 w-4 text-muted-foreground" />
        <span className="text-sm font-medium">Capabilities</span>
        {saving ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
        ) : null}
      </div>
      <p className="text-xs text-muted-foreground">
        {editable
          ? "Enable the services this environment runs. Applies on next start."
          : "Stop the environment to change its capabilities."}
      </p>

      {!available ? (
        <div className="flex items-center gap-2 py-1 text-sm text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading…
        </div>
      ) : (
        <ul className="space-y-1">
          {available.map((m) => {
            const on = enabled.has(m.id);
            return (
              <li key={m.id}>
                <label
                  className={cn(
                    "flex items-center gap-2 rounded px-1.5 py-1 text-sm",
                    disabled
                      ? "cursor-not-allowed opacity-60"
                      : "cursor-pointer hover:bg-accent/50",
                  )}
                >
                  <input
                    type="checkbox"
                    checked={on}
                    disabled={disabled}
                    onChange={() => toggle(m.id)}
                    className="h-3.5 w-3.5 accent-primary"
                  />
                  <span>{m.label}</span>
                </label>
              </li>
            );
          })}
          <li>
            <label
              className={cn(
                "flex items-center gap-2 rounded px-1.5 py-1 text-sm",
                disabled
                  ? "cursor-not-allowed opacity-60"
                  : "cursor-pointer hover:bg-accent/50",
              )}
            >
              <input
                type="checkbox"
                checked={observability}
                disabled={disabled}
                onChange={() => toggleObservability()}
                className="h-3.5 w-3.5 accent-primary"
              />
              <span>Observability</span>
            </label>
          </li>
        </ul>
      )}

      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </div>
  );
}
