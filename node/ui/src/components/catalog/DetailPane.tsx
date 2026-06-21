import { Database, FolderTree, Pencil, Trash2, X } from "lucide-react";
import type { ReactNode } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CatalogDetail } from "./detail/CatalogDetail";
import { FunctionDetail } from "./detail/FunctionDetail";
import { ModelDetail } from "./detail/ModelDetail";
import { SchemaDetail } from "./detail/SchemaDetail";
import { TableDetail } from "./detail/TableDetail";
import { VolumeDetail } from "./detail/VolumeDetail";
import type { EditableKind } from "./dialog-types";
import { useCatalogDialogs } from "./dialogs";
import { kindIcon } from "./groups";
import { useCatalogSelection } from "./selection";
import { type SelectableKind, splitFullName } from "./types";

// Catalogs / schemas / volumes / models support PATCH; tables / functions don't.
const EDITABLE: ReadonlySet<SelectableKind> = new Set<EditableKind>([
  "catalog",
  "schema",
  "volume",
  "model",
]);

function detailIcon(kind: SelectableKind): ReactNode {
  if (kind === "catalog")
    return <Database className="h-4 w-4 text-muted-foreground" />;
  if (kind === "schema")
    return <FolderTree className="h-4 w-4 text-muted-foreground" />;
  return kindIcon(kind, "h-4 w-4 text-muted-foreground");
}

export function DetailPane() {
  const { selection, select } = useCatalogSelection();
  const dialogs = useCatalogDialogs();

  if (!selection) {
    return (
      <div className="flex min-h-0 items-center justify-center p-8 text-center text-sm text-muted-foreground">
        Select an object from the tree to see its details.
      </div>
    );
  }

  const { object } = splitFullName(selection.fullName);
  const editable = EDITABLE.has(selection.kind);

  return (
    <div className="flex min-h-0 flex-col overflow-auto">
      <div className="sticky top-0 z-10 flex items-center justify-between border-b bg-card px-6 py-3">
        <div className="flex min-w-0 items-center gap-2">
          {detailIcon(selection.kind)}
          <span className="truncate font-mono text-sm font-medium">
            {selection.fullName || object}
          </span>
          <Badge>{selection.kind}</Badge>
        </div>
        <div className="flex items-center gap-1">
          {editable && (
            <Button
              variant="ghost"
              size="sm"
              className="h-7"
              onClick={() =>
                dialogs.edit({
                  kind: selection.kind as EditableKind,
                  name: selection.fullName,
                })
              }
            >
              <Pencil className="h-3.5 w-3.5" />
              Edit
            </Button>
          )}
          <Button
            variant="ghost"
            size="sm"
            className="h-7 text-destructive hover:text-destructive"
            onClick={() =>
              dialogs.remove({ kind: selection.kind, name: selection.fullName })
            }
          >
            <Trash2 className="h-3.5 w-3.5" />
            Delete
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            aria-label="Close"
            onClick={() => select(undefined)}
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      </div>
      <div className="px-6 py-4">
        {selection.kind === "catalog" && (
          <CatalogDetail name={selection.fullName} />
        )}
        {selection.kind === "schema" && (
          <SchemaDetail fullName={selection.fullName} />
        )}
        {selection.kind === "table" && (
          <TableDetail fullName={selection.fullName} />
        )}
        {selection.kind === "volume" && (
          <VolumeDetail fullName={selection.fullName} />
        )}
        {selection.kind === "function" && (
          <FunctionDetail fullName={selection.fullName} />
        )}
        {selection.kind === "model" && (
          <ModelDetail fullName={selection.fullName} />
        )}
      </div>
    </div>
  );
}
