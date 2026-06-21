import type { VolumeType } from "@open-lakehouse/uc-client";
import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import { useMemo, useState } from "react";
import { toast } from "sonner";

import type { CreateRequest } from "@/components/catalog/dialog-types";
import { SchemaForm } from "@/components/forms/SchemaForm";
import { cloneSchema, formSchemas } from "@/components/forms/schemas";
import { StorageLocationPicker } from "@/components/storage/StorageLocationPicker";
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
  if (request.kind === "catalog" || request.kind === "schema") {
    return <NamespaceCreateDialog request={request} onClose={onClose} />;
  }
  return <LeafCreateDialog request={request} onClose={onClose} />;
}

const NAMESPACE_FORM_ID = "namespace-create-form";

type LooseSchema = Record<string, unknown>;

interface NamespaceFormData {
  name?: string;
  catalog_name?: string;
  comment?: string;
}

/** Reduce a catalog/schema create schema to the name + comment fields. */
function tailorNamespaceSchema(base: RJSFSchema): RJSFSchema {
  const schema = cloneSchema(base);
  const props = (schema.properties ?? {}) as Record<string, LooseSchema>;
  // The key-value `properties` map is an object field; drop it (not surfaced).
  delete props.properties;
  return schema;
}

// Hidden fields are still part of the schema (so injected values validate) but
// not rendered; storage is handled by the dedicated picker below the form.
const NAMESPACE_HIDDEN_UI: UiSchema = {
  catalog_name: { "ui:widget": "hidden" },
  storage_root: { "ui:widget": "hidden" },
  storage_location: { "ui:widget": "hidden" },
  provider_name: { "ui:widget": "hidden" },
  share_name: { "ui:widget": "hidden" },
  comment: { "ui:placeholder": "Description (optional)" },
  name: { "ui:placeholder": "my_object", "ui:autofocus": true },
  "ui:order": ["name", "comment", "*"],
};

function NamespaceCreateDialog({
  request,
  onClose,
}: {
  request: Extract<CreateRequest, { kind: "catalog" | "schema" }>;
  onClose: () => void;
}) {
  const createCatalog = useCreateCatalog();
  const createSchema = useCreateSchema();

  const [formData, setFormData] = useState<NamespaceFormData>(() =>
    request.kind === "schema" ? { catalog_name: request.catalog } : {},
  );
  const [storageRoot, setStorageRoot] = useState<string>();

  const schema = useMemo(
    () =>
      tailorNamespaceSchema(
        request.kind === "catalog"
          ? formSchemas.createCatalog
          : formSchemas.createSchema,
      ),
    [request.kind],
  );

  const pending = createCatalog.isPending || createSchema.isPending;

  function submit(data: NamespaceFormData) {
    const handlers = {
      onSuccess: () => {
        toast.success(`Created ${request.kind} "${data.name}"`);
        onClose();
      },
      onError: (error: unknown) => toast.error(parseUcError(error)),
    };

    if (request.kind === "catalog") {
      createCatalog.mutate(
        {
          body: {
            name: data.name ?? "",
            comment: data.comment || undefined,
            storage_root: storageRoot,
          },
        },
        handlers,
      );
      return;
    }

    createSchema.mutate(
      {
        body: {
          name: data.name ?? "",
          catalog_name: request.catalog,
          comment: data.comment || undefined,
          storage_root: storageRoot,
        },
      },
      handlers,
    );
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{TITLES[request.kind]}</DialogTitle>
        </DialogHeader>

        <div className="space-y-3 px-5 py-4">
          {request.kind === "schema" && (
            <p className="text-xs text-muted-foreground">
              In <span className="font-mono">{request.catalog}</span>
            </p>
          )}

          <SchemaForm<NamespaceFormData>
            id={NAMESPACE_FORM_ID}
            schema={schema}
            uiSchema={NAMESPACE_HIDDEN_UI}
            formData={formData}
            disabled={pending}
            onChange={setFormData}
            onSubmit={submit}
          />

          <StorageLocationPicker onChange={setStorageRoot} />
        </div>

        <DialogFooter>
          <Button type="button" variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button
            type="submit"
            form={NAMESPACE_FORM_ID}
            size="sm"
            disabled={pending}
          >
            {pending ? "Creating…" : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function LeafCreateDialog({
  request,
  onClose,
}: {
  request: Extract<CreateRequest, { kind: "volume" | "model" }>;
  onClose: () => void;
}) {
  const createVolume = useCreateVolume();
  const createModel = useCreateRegisteredModel();

  const [name, setName] = useState("");
  const [comment, setComment] = useState("");
  const [volumeType, setVolumeType] = useState<VolumeType>("MANAGED");
  const [storageLocation, setStorageLocation] = useState("");

  const pending = createVolume.isPending || createModel.isPending;

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

    if (request.kind === "volume") {
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

  const parent = `${request.catalog}.${request.schema}`;

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <form onSubmit={submit}>
          <DialogHeader>
            <DialogTitle>{TITLES[request.kind]}</DialogTitle>
          </DialogHeader>

          <div className="space-y-3 px-5 py-4">
            <p className="text-xs text-muted-foreground">
              In <span className="font-mono">{parent}</span>
            </p>

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
