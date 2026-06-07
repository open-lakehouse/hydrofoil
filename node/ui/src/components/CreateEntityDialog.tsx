import type { VolumeType } from "@open-lakehouse/uc-client";
import { X } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  useCreateCatalog,
  useCreateRegisteredModel,
  useCreateSchema,
  useCreateVolume,
} from "@/lib/uc/mutations";

// What the dialog should create, with any parent namespace context prefilled.
export type CreateRequest =
  | { kind: "catalog" }
  | { kind: "schema"; catalog: string }
  | { kind: "volume"; catalog: string; schema: string }
  | { kind: "model"; catalog: string; schema: string };

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

  const error =
    createCatalog.error ||
    createSchema.error ||
    createVolume.error ||
    createModel.error;

  function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!name.trim()) return;
    const done = { onSuccess: () => onClose() };

    if (request.kind === "catalog") {
      createCatalog.mutate(
        { body: { name, comment: comment || undefined } },
        done,
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
        done,
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
        done,
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
        done,
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
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      role="dialog"
      aria-modal="true"
    >
      <form
        onSubmit={submit}
        className="w-full max-w-md rounded-lg border bg-card shadow-lg"
      >
        <div className="flex items-center justify-between border-b px-5 py-3">
          <h2 className="text-sm font-semibold">{TITLES[request.kind]}</h2>
          <Button type="button" variant="ghost" size="sm" onClick={onClose}>
            <X className="h-4 w-4" />
          </Button>
        </div>

        <div className="space-y-3 px-5 py-4">
          {parent && (
            <p className="text-xs text-muted-foreground">
              In <span className="font-mono">{parent}</span>
            </p>
          )}

          <Field label="Name">
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="w-full rounded border bg-background px-2 py-1.5 text-sm"
              placeholder="my_object"
            />
          </Field>

          {request.kind === "volume" && (
            <>
              <Field label="Volume type">
                <select
                  value={volumeType}
                  onChange={(e) => setVolumeType(e.target.value as VolumeType)}
                  className="w-full rounded border bg-background px-2 py-1.5 text-sm"
                >
                  <option value="MANAGED">MANAGED</option>
                  <option value="EXTERNAL">EXTERNAL</option>
                </select>
              </Field>
              {volumeType === "EXTERNAL" && (
                <Field label="Storage location">
                  <input
                    value={storageLocation}
                    onChange={(e) => setStorageLocation(e.target.value)}
                    className="w-full rounded border bg-background px-2 py-1.5 text-sm"
                    placeholder="s3://bucket/path"
                  />
                </Field>
              )}
            </>
          )}

          <Field label="Comment (optional)">
            <input
              value={comment}
              onChange={(e) => setComment(e.target.value)}
              className="w-full rounded border bg-background px-2 py-1.5 text-sm"
              placeholder="Description"
            />
          </Field>

          {error ? (
            <p className="text-sm text-destructive">
              {(error as { message?: string })?.message ?? "Request failed."}
            </p>
          ) : null}
        </div>

        <div className="flex justify-end gap-2 border-t px-5 py-3">
          <Button type="button" variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" size="sm" disabled={pending || !name.trim()}>
            {pending ? "Creating…" : "Create"}
          </Button>
        </div>
      </form>
    </div>
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
    <div className="block space-y-1">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      {children}
    </div>
  );
}
