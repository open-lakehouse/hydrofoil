// Modal "create environment" dialog. Creating an environment leaves it IDLE — it
// is registered but its services are not started, so the user can review/adjust
// its configuration before deliberately starting it (Start / Launch from the
// detail pane). The new (stopped) environment is handed back so the manager can
// select its card.
//
// Opened from the environment manager sidebar header (mirroring the catalog
// view's "New" affordance) instead of an inline form.

import {
  Button,
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
} from "@open-lakehouse/ui-kit";
import { useState } from "react";
import {
  type Environment,
  getEnvironmentHost,
} from "@/lib/client/environments";

export function CreateEnvironmentDialog({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  /** A new (idle, not-started) environment was created. */
  onCreated: (env: Environment) => void;
}) {
  const host = getEnvironmentHost();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) return;
    setError(null);
    setBusy(true);
    try {
      // Create only — do NOT start. The user starts it deliberately afterwards.
      onCreated(await host.create(trimmed));
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <form onSubmit={submit}>
          <DialogHeader>
            <DialogTitle>New environment</DialogTitle>
          </DialogHeader>

          <div className="space-y-3 px-5 py-4">
            <div className="space-y-1">
              <Label htmlFor="env-name">Name</Label>
              <Input
                id="env-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="my-environment"
                disabled={busy}
                autoFocus
              />
            </div>
            {error ? <p className="text-sm text-destructive">{error}</p> : null}
          </div>

          <DialogFooter>
            <Button type="button" variant="ghost" size="sm" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" size="sm" disabled={busy || !name.trim()}>
              {busy ? "Creating…" : "Create"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
