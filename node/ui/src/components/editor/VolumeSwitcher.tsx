import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@open-lakehouse/ui-kit";
import { Plus } from "lucide-react";
import { useState } from "react";
import type { Volume } from "@/lib/editor/volumes";
import { AddVolumeDialog } from "./AddVolumeDialog";

// Picks the editor's active volume. Lists the known volumes (Home, when the host
// provides it, plus any added UC volumes) and an "Add volume…" action that opens
// the catalog→schema→volume picker. Controlled: the editor route owns the active
// volume + the list (so it can mirror the active one to the URL).
const ADD_SENTINEL = "__add__";

export function VolumeSwitcher({
  volumes,
  activeRoot,
  onSelect,
  onAdd,
}: {
  volumes: Volume[];
  activeRoot: string | undefined;
  onSelect: (root: string) => void;
  onAdd: (volume: Volume) => void;
}) {
  const [dialogOpen, setDialogOpen] = useState(false);

  function onValueChange(value: string) {
    if (value === ADD_SENTINEL) {
      setDialogOpen(true);
      return;
    }
    onSelect(value);
  }

  return (
    <div className="border-b p-2">
      <Select value={activeRoot} onValueChange={onValueChange}>
        <SelectTrigger className="h-8 text-xs">
          <SelectValue placeholder="Select a volume" />
        </SelectTrigger>
        <SelectContent>
          {volumes.map((v) => (
            <SelectItem key={v.id} value={v.root} className="text-xs">
              {v.label}
            </SelectItem>
          ))}
          <SelectItem value={ADD_SENTINEL} className="text-xs">
            <span className="flex items-center gap-1.5">
              <Plus className="h-3.5 w-3.5" />
              Add volume…
            </span>
          </SelectItem>
        </SelectContent>
      </Select>

      <AddVolumeDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        onAdd={onAdd}
      />
    </div>
  );
}
