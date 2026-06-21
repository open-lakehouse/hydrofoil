import type { VolumeType } from "@open-lakehouse/uc-client";
import { useState } from "react";
import { toast } from "sonner";

import type { CreateRequest } from "@/components/catalog/dialog-types";
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
import { parseUcError } from "@/lib/uc/errors";
import {
  useCreateCatalog,
  useCreateRegisteredModel,
  useCreateSchema,
  useCreateVolume,
} from "@/lib/uc/mutations";

export type { CreateRequest };

const TITLES: Record<CreateRequest["kind"], string> = {
  catalog: "New catalog",
  schema: "New schema",
  volume: "New volume",
  model: "New registered model",
};

export function CreateEntityDialog({
  request,
  onClose,
}: {
  request: CreateRequest;
  onClose: () => void;
}) {
  const createCatalog = useCreateCatalog();
  const createSchema = useCreateSchema();
  const createVolume = useCreateVolume();
  const createModel = useCreateRegisteredModel();

  const [name, setName] = useState("");
  const [comment, setComment] = useState("");
  const [volumeType, setVolumeType] = useState<VolumeType>("MANAGED");
  const [storageLocation, setStorageLocation] = useState("");

  const pending =
    createCatalog.isPending ||
    createSchema.isPending ||
    createVolume.isPending ||
    createModel.isPending;

  function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!name.trim()) return;

    const handlers = {
      onSuccess: () => {
        toast.success(`Created ${request.kind} "${name}"`);
        onClose();
      },
      onError: (error: unknown) => toast.error(parseUcError(error)),
    };

    if (request.kind === "catalog") {
      createCatalog.mutate(
        { body: { name, comment: comment || undefined } },
        handlers,
      );
    } else if (request.kind === "schema") {
      createSchema.mutate(
        {
          body: {
            name,
            catalog_name: request.catalog,
            comment: comment || undefined,
          },
        },
        handlers,
      );
    } else if (request.kind === "volume") {
      createVolume.mutate(
        {
          body: {
            name,
            catalog_name: request.catalog,
            schema_name: request.schema,
            volume_type: volumeType,
            comment: comment || undefined,
            storage_location:
              volumeType === "EXTERNAL" ? storageLocation : undefined,
          },
        },
        handlers,
      );
    } else {
      createModel.mutate(
        {
          body: {
            name,
            catalog_name: request.catalog,
            schema_name: request.schema,
            comment: comment || undefined,
          },
        },
        handlers,
      );
    }
  }

  const parent =
    request.kind === "schema"
      ? request.catalog
      : request.kind === "volume" || request.kind === "model"
        ? `${request.catalog}.${request.schema}`
        : undefined;

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <form onSubmit={submit}>
          <DialogHeader>
            <DialogTitle>{TITLES[request.kind]}</DialogTitle>
          </DialogHeader>

          <div className="space-y-3 px-5 py-4">
            {parent && (
              <p className="text-xs text-muted-foreground">
                In <span className="font-mono">{parent}</span>
              </p>
            )}

            <div className="space-y-1">
              <Label htmlFor="entity-name">Name</Label>
              <Input
                id="entity-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="my_object"
                autoFocus
              />
            </div>

            {request.kind === "volume" && (
              <>
                <div className="space-y-1">
                  <Label htmlFor="volume-type">Volume type</Label>
                  <select
                    id="volume-type"
                    value={volumeType}
                    onChange={(e) =>
                      setVolumeType(e.target.value as VolumeType)
                    }
                    className="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    <option value="MANAGED">MANAGED</option>
                    <option value="EXTERNAL">EXTERNAL</option>
                  </select>
                </div>
                {volumeType === "EXTERNAL" && (
                  <div className="space-y-1">
                    <Label htmlFor="storage-location">Storage location</Label>
                    <Input
                      id="storage-location"
                      value={storageLocation}
                      onChange={(e) => setStorageLocation(e.target.value)}
                      placeholder="s3://bucket/path"
                    />
                  </div>
                )}
              </>
            )}

            <div className="space-y-1">
              <Label htmlFor="entity-comment">Comment (optional)</Label>
              <Input
                id="entity-comment"
                value={comment}
                onChange={(e) => setComment(e.target.value)}
                placeholder="Description"
              />
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
