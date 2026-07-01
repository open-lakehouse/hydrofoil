// Choose where an environment's credential-encryption key (KEK) lives. The KEK
// envelope-encrypts storage credentials at rest in the environment's Unity
// Catalog. Today the desktop host stores it in the OS keychain (the fully-wired
// option); a remote key store is shown as a forthcoming choice but is not yet
// selectable. Configuration is only meaningful while the environment is idle —
// the caller gates on that.

import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@open-lakehouse/ui-kit";
import { useState } from "react";
import {
  getEnvironmentHost,
  type KeyProvider,
  type KeyStatus,
} from "@/lib/client/environments";

export function ConfigureKeyDialog({
  environmentId,
  open,
  onOpenChange,
  onConfigured,
}: {
  environmentId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The new key status after a successful configuration. */
  onConfigured: (status: KeyStatus) => void;
}) {
  const host = getEnvironmentHost();
  const [provider, setProvider] = useState<KeyProvider>("keychain");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The remote provider is scaffolded but not wired end-to-end yet, so we don't
  // let the user commit to it (it would persist a choice with no backend).
  const canConfirm = provider === "keychain" && !busy;

  async function confirm() {
    setError(null);
    setBusy(true);
    try {
      const status = await host.configureKey(environmentId, provider);
      onConfigured(status);
      onOpenChange(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Configure encryption key</DialogTitle>
          <DialogDescription>
            Credentials this environment stores are encrypted at rest with a key
            you control. Choose where that key lives.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3 px-5 py-4">
          <div className="space-y-1">
            <span className="text-xs font-medium">Key store</span>
            <Select
              value={provider}
              onValueChange={(v) => setProvider(v as KeyProvider)}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="keychain">OS keychain</SelectItem>
                <SelectItem value="remote">Remote key store</SelectItem>
              </SelectContent>
            </Select>
          </div>

          {provider === "keychain" ? (
            <p className="text-xs text-muted-foreground">
              A fresh key is generated and stored in your operating system's
              keychain. It never touches disk.
            </p>
          ) : (
            <p className="text-xs text-muted-foreground">
              Remote key management (e.g. a cloud KMS) is coming soon and can't
              be selected yet.
            </p>
          )}

          {error ? <p className="text-sm text-destructive">{error}</p> : null}
        </div>

        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            type="button"
          >
            Cancel
          </Button>
          <Button onClick={confirm} disabled={!canConfirm} type="button">
            {busy ? "Configuring…" : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
