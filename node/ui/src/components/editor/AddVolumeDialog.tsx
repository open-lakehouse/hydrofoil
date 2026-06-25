import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useCatalogs, useSchemas, useVolumes } from "@/features/unity-catalog";
import { ucVolume, type Volume } from "@/lib/editor/volumes";

// A small catalog → schema → volume picker. On confirm, hands back the chosen
// volume as a `Volume`. Reuses the existing UC list hooks (useCatalogs /
// useSchemas / useVolumes), which are already cursor-paginated + cached.
export function AddVolumeDialog({
  open,
  onOpenChange,
  onAdd,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAdd: (volume: Volume) => void;
}) {
  const [catalog, setCatalog] = useState<string | undefined>();
  const [schema, setSchema] = useState<string | undefined>();
  const [volume, setVolume] = useState<string | undefined>();

  const catalogs = useCatalogs();
  const schemas = useSchemas(catalog);
  const volumes = useVolumes(catalog, schema);

  // Reset downstream selections when an upstream one changes.
  function pickCatalog(name: string) {
    setCatalog(name);
    setSchema(undefined);
    setVolume(undefined);
  }
  function pickSchema(name: string) {
    setSchema(name);
    setVolume(undefined);
  }

  function confirm() {
    if (!catalog || !schema || !volume) return;
    onAdd(ucVolume({ catalog, schema, volume }));
    onOpenChange(false);
    setCatalog(undefined);
    setSchema(undefined);
    setVolume(undefined);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add a volume</DialogTitle>
          <DialogDescription>
            Pick a Unity Catalog volume to open in the editor.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          <Field label="Catalog">
            <Select value={catalog} onValueChange={pickCatalog}>
              <SelectTrigger>
                <SelectValue placeholder="Select a catalog" />
              </SelectTrigger>
              <SelectContent>
                {(catalogs.data ?? []).map((c) => (
                  <SelectItem key={c.name} value={c.name ?? ""}>
                    {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>

          <Field label="Schema">
            <Select
              value={schema}
              onValueChange={pickSchema}
              disabled={!catalog}
            >
              <SelectTrigger>
                <SelectValue placeholder="Select a schema" />
              </SelectTrigger>
              <SelectContent>
                {(schemas.data ?? []).map((s) => (
                  <SelectItem key={s.name} value={s.name ?? ""}>
                    {s.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>

          <Field label="Volume">
            <Select value={volume} onValueChange={setVolume} disabled={!schema}>
              <SelectTrigger>
                <SelectValue placeholder="Select a volume" />
              </SelectTrigger>
              <SelectContent>
                {(volumes.data ?? []).map((v) => (
                  <SelectItem key={v.name} value={v.name ?? ""}>
                    {v.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
        </div>

        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            type="button"
          >
            Cancel
          </Button>
          <Button onClick={confirm} disabled={!volume} type="button">
            Add volume
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-1">
      <span className="text-xs font-medium">{label}</span>
      {children}
    </div>
  );
}
