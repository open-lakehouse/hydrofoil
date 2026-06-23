// Modal "create environment" dialog. Creating an environment immediately selects
// it (brings its services online) and hands the resulting active environment back
// to the caller — create + open is the common first-run path, and the management
// view wants the new environment to become the active one.
//
// Opened from the environment manager sidebar header (mirroring the catalog
// view's "New" affordance) instead of an inline form.

import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  type ActiveEnvironment,
  getEnvironmentHost,
} from "@/lib/client/environments";

export function CreateEnvironmentDialog({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  /** A new environment was created and brought online. */
  onCreated: (env: ActiveEnvironment) => void;
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
      const env = await host.create(trimmed);
      onCreated(await host.select(env.id));
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
