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
import { toast } from "sonner";
import { connectFileStore } from "@/lib/files/store";
import {
  ENGINE_LABELS,
  type NotebookEngine,
  notebookTemplate,
} from "@/lib/notebook/templates";

const ENGINES: NotebookEngine[] = ["spark", "duckdb", "polars"];

/** Join a directory root and a basename into an absolute path. */
function joinPath(root: string, name: string): string {
  return `${root.replace(/\/+$/, "")}/${name}`;
}

/**
 * Create a new marimo notebook in the active volume: ask for a name + engine,
 * write a templated `.py` (refusing to clobber an existing file), then open it
 * as a notebook tab via `onCreated`.
 */
export function NewNotebookDialog({
  root,
  onClose,
  onCreated,
}: {
  /** The active volume root the notebook is created under. */
  root: string;
  onClose: () => void;
  /** Called with the new file's path after a successful write. */
  onCreated: (path: string) => void;
}) {
  const [name, setName] = useState("");
  const [engine, setEngine] = useState<NotebookEngine>("spark");
  const [pending, setPending] = useState(false);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed || pending) return;

    const fileName = trimmed.endsWith(".py") ? trimmed : `${trimmed}.py`;
    const path = joinPath(root, fileName);
    setPending(true);
    try {
      // Refuse to overwrite an existing file.
      let exists = false;
      try {
        await connectFileStore.stat(path);
        exists = true;
      } catch {
        // Not found (or unreadable) — treat as creatable.
      }
      if (exists) {
        toast.error(`A file named "${fileName}" already exists`);
        return;
      }

      const bytes = new TextEncoder().encode(notebookTemplate(engine));
      await connectFileStore.writeFile(path, bytes, {
        contentType: "text/x-python",
      });
      toast.success(`Created notebook "${fileName}"`);
      onCreated(path);
      onClose();
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Failed to create notebook",
      );
    } finally {
      setPending(false);
    }
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <form onSubmit={submit}>
          <DialogHeader>
            <DialogTitle>New notebook</DialogTitle>
          </DialogHeader>

          <div className="space-y-3 px-5 py-4">
            <p className="text-xs text-muted-foreground">
              In <span className="font-mono">{root}</span>
            </p>

            <div className="space-y-1">
              <Label htmlFor="notebook-name">Name</Label>
              <Input
                id="notebook-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="analysis"
                autoFocus
              />
            </div>

            <div className="space-y-1">
              <Label htmlFor="notebook-engine">Query engine</Label>
              <select
                id="notebook-engine"
                value={engine}
                onChange={(e) => setEngine(e.target.value as NotebookEngine)}
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                {ENGINES.map((e) => (
                  <option key={e} value={e}>
                    {ENGINE_LABELS[e]}
                  </option>
                ))}
              </select>
            </div>
          </div>

          <DialogFooter>
            <Button type="button" variant="ghost" size="sm" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" size="sm" disabled={pending || !name.trim()}>
              {pending ? "Creating…" : "Create"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
