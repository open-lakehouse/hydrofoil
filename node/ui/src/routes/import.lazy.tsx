// Import page — create a Unity Catalog managed Delta table from a local file.
//
// Phase 1 (desktop-only): pick a Parquet file, the host parses it and returns the
// inferred schema + a sample; preview the rows in the shared DataGrid, adjust the
// schema (column names / types / nullability), pick a target catalog + schema +
// table name, then create + ingest as a managed Delta table. The data is read by
// the host from the local path — see lib/ingest/client.ts + registry.ts.
//
// Developed in isolation as a dedicated page; a later phase can surface it from
// the catalog view.

import { ArrowResultStore, DataGrid } from "@open-lakehouse/data-grid";
import {
  Button,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@open-lakehouse/ui-kit";
import {
  invalidateTables,
  useCatalogs,
  useSchemas,
} from "@open-lakehouse/unity-catalog-client";
import { useQueryClient } from "@tanstack/react-query";
import { createLazyRoute } from "@tanstack/react-router";
import { Loader2, TableProperties, Upload } from "lucide-react";
import { useMemo, useState } from "react";
import { ingestTable, previewFile } from "@/lib/ingest/client";
import { ingestSupported, pickFile } from "@/lib/ingest/registry";
import {
  COLUMN_TYPES,
  type ColumnType,
  columnsFromSchemaIpc,
  type EditableColumn,
  schemaIpcFromColumns,
  validateColumns,
} from "@/lib/ingest/schema";

export const Route = createLazyRoute("/import")({
  component: ImportPage,
});

/** A previewed file ready to ingest: its path + the editable schema + a sample. */
interface Loaded {
  path: string;
  name: string;
  totalRows: number;
  /** A DataGrid-ready store holding the sample rows. */
  sampleStore: ArrowResultStore;
}

function ImportPage() {
  if (!ingestSupported()) {
    return (
      <div className="flex h-full items-center justify-center p-8 text-sm text-muted-foreground">
        Importing a file into a table is only available in the desktop app.
      </div>
    );
  }
  return <ImportFlow />;
}

function ImportFlow() {
  const queryClient = useQueryClient();

  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [columns, setColumns] = useState<EditableColumn[]>([]);
  const [previewing, setPreviewing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Target selection.
  const [catalog, setCatalog] = useState<string>("");
  const [schema, setSchema] = useState<string>("");
  const [table, setTable] = useState<string>("");

  // Ingest progress / result.
  const [ingesting, setIngesting] = useState(false);
  const [result, setResult] = useState<string | null>(null);

  async function onPick() {
    setError(null);
    try {
      const picked = await pickFile();
      if (!picked) return;
      setPreviewing(true);
      setResult(null);
      const preview = await previewFile(picked.path);
      const sampleStore = new ArrowResultStore();
      sampleStore.append(preview.sampleIpc);
      setLoaded({
        path: picked.path,
        name: picked.name,
        totalRows: preview.totalRows,
        sampleStore,
      });
      setColumns(columnsFromSchemaIpc(preview.schemaIpc));
      // Default the table name from the file's base name (sans extension).
      setTable(picked.name.replace(/\.[^.]+$/, ""));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPreviewing(false);
    }
  }

  async function onCreate() {
    setError(null);
    setResult(null);
    if (!loaded) return;
    const colError = validateColumns(columns);
    if (colError) return setError(colError);
    if (!catalog || !schema || !table.trim()) {
      return setError("pick a catalog and schema, and name the table");
    }
    setIngesting(true);
    try {
      const res = await ingestTable({
        catalog,
        schema,
        table: table.trim(),
        targetSchemaIpc: schemaIpcFromColumns(columns),
        sourcePath: loaded.path,
        createIfMissing: true,
      });
      await invalidateTables(queryClient, catalog, schema);
      setResult(
        `${res.created ? "Created" : "Appended to"} ${res.qualifiedName} — ${
          res.rowsWritten
        } row${res.rowsWritten === 1 ? "" : "s"} written.`,
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIngesting(false);
    }
  }

  return (
    <div className="mx-auto flex h-full max-w-5xl flex-col gap-6 p-6">
      <div>
        <h1 className="flex items-center gap-2 text-xl font-semibold tracking-tight">
          <TableProperties className="h-5 w-5" />
          Create table from file
        </h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Upload a Parquet file to create a managed Delta table in Unity
          Catalog.
        </p>
      </div>

      <div className="flex items-center gap-3">
        <Button onClick={() => void onPick()} disabled={previewing}>
          {previewing ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Upload className="h-4 w-4" />
          )}
          Choose Parquet file…
        </Button>
        {loaded && (
          <span className="text-sm text-muted-foreground">
            {loaded.name}
            {loaded.totalRows > 0 ? ` · ${loaded.totalRows} rows` : ""}
          </span>
        )}
      </div>

      {error && (
        <div className="rounded border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}
      {result && (
        <div className="rounded border border-emerald-500/40 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-700 dark:text-emerald-300">
          {result}
        </div>
      )}

      {loaded && (
        <div className="flex min-h-0 flex-1 flex-col gap-6">
          <SchemaEditor columns={columns} onChange={setColumns} />

          <section className="flex min-h-0 flex-1 flex-col">
            <h2 className="mb-2 text-sm font-semibold">Preview</h2>
            <div className="min-h-0 flex-1 rounded border">
              <DataGrid
                store={loaded.sampleStore}
                version={1}
                running={false}
              />
            </div>
          </section>

          <TargetForm
            catalog={catalog}
            schema={schema}
            table={table}
            onCatalog={(c) => {
              setCatalog(c);
              setSchema("");
            }}
            onSchema={setSchema}
            onTable={setTable}
          />

          <div className="flex items-center gap-3">
            <Button onClick={() => void onCreate()} disabled={ingesting}>
              {ingesting ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <TableProperties className="h-4 w-4" />
              )}
              Create table
            </Button>
            {ingesting && (
              <span className="text-sm text-muted-foreground">
                Writing managed Delta table…
              </span>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

/** Editable grid of the inferred columns: name, type, nullability. */
function SchemaEditor({
  columns,
  onChange,
}: {
  columns: EditableColumn[];
  onChange: (cols: EditableColumn[]) => void;
}) {
  const update = (i: number, patch: Partial<EditableColumn>) => {
    onChange(columns.map((c, idx) => (idx === i ? { ...c, ...patch } : c)));
  };

  return (
    <section>
      <h2 className="mb-2 text-sm font-semibold">Schema</h2>
      <div className="overflow-hidden rounded border">
        <table className="w-full text-sm">
          <thead className="bg-muted/50 text-left text-xs text-muted-foreground">
            <tr>
              <th className="px-3 py-2 font-medium">Column</th>
              <th className="px-3 py-2 font-medium">Type</th>
              <th className="px-3 py-2 font-medium">Nullable</th>
            </tr>
          </thead>
          <tbody>
            {columns.map((col, i) => (
              <tr key={col.id} className="border-t">
                <td className="px-3 py-1.5">
                  <Input
                    value={col.name}
                    onChange={(e) => update(i, { name: e.target.value })}
                    className="h-8"
                  />
                </td>
                <td className="px-3 py-1.5">
                  <Select
                    value={col.type}
                    onValueChange={(v) => update(i, { type: v as ColumnType })}
                  >
                    <SelectTrigger className="h-8 w-36">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {COLUMN_TYPES.map((t) => (
                        <SelectItem key={t} value={t}>
                          {t}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </td>
                <td className="px-3 py-1.5">
                  <input
                    type="checkbox"
                    checked={col.nullable}
                    onChange={(e) => update(i, { nullable: e.target.checked })}
                    aria-label={`${col.name} nullable`}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}

/** Target catalog + schema pickers (from live UC metadata) + table-name input. */
function TargetForm({
  catalog,
  schema,
  table,
  onCatalog,
  onSchema,
  onTable,
}: {
  catalog: string;
  schema: string;
  table: string;
  onCatalog: (c: string) => void;
  onSchema: (s: string) => void;
  onTable: (t: string) => void;
}) {
  const catalogs = useCatalogs();
  const schemas = useSchemas(catalog || undefined);

  const catalogNames = useMemo(
    () => (catalogs.data ?? []).map((c) => c.name).filter(Boolean) as string[],
    [catalogs.data],
  );
  const schemaNames = useMemo(
    () => (schemas.data ?? []).map((s) => s.name).filter(Boolean) as string[],
    [schemas.data],
  );

  return (
    <section className="grid grid-cols-3 gap-4">
      <div className="flex flex-col gap-1.5">
        <Label>Catalog</Label>
        <Select value={catalog} onValueChange={onCatalog}>
          <SelectTrigger>
            <SelectValue placeholder="Select catalog" />
          </SelectTrigger>
          <SelectContent>
            {catalogNames.map((c) => (
              <SelectItem key={c} value={c}>
                {c}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div className="flex flex-col gap-1.5">
        <Label>Schema</Label>
        <Select value={schema} onValueChange={onSchema} disabled={!catalog}>
          <SelectTrigger>
            <SelectValue placeholder="Select schema" />
          </SelectTrigger>
          <SelectContent>
            {schemaNames.map((s) => (
              <SelectItem key={s} value={s}>
                {s}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div className="flex flex-col gap-1.5">
        <Label>Table name</Label>
        <Input
          value={table}
          onChange={(e) => onTable(e.target.value)}
          placeholder="my_table"
        />
      </div>
    </section>
  );
}
