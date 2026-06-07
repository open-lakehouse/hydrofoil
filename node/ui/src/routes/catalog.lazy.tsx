import type { TableInfo } from "@open-lakehouse/uc-client";
import { useQueryClient } from "@tanstack/react-query";
import { createLazyRoute } from "@tanstack/react-router";
import {
  ChevronRight,
  Columns3,
  Database,
  FolderTree,
  Table2,
} from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  prefetchSchemas,
  tableFullName,
  useCatalogs,
  useSchemas,
  useTables,
} from "@/lib/uc/queries";
import { cn } from "@/lib/utils";

export const Route = createLazyRoute("/catalog")({
  component: CatalogPage,
});

function CatalogPage() {
  const queryClient = useQueryClient();
  const [catalog, setCatalog] = useState<string>();
  const [schema, setSchema] = useState<string>();
  const [table, setTable] = useState<TableInfo>();

  const catalogs = useCatalogs();
  const schemas = useSchemas(catalog);
  const tables = useTables(catalog, schema);

  return (
    <div className="flex h-[calc(100vh-3rem)] flex-col">
      <div className="border-b px-6 py-4">
        <h1 className="flex items-center gap-2 text-lg font-semibold">
          <Database className="h-5 w-5" />
          Unity Catalog
        </h1>
        <p className="text-sm text-muted-foreground">
          Browse catalogs, schemas, and tables.
        </p>
      </div>

      <div className="grid flex-1 grid-cols-1 overflow-hidden md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_minmax(0,1fr)]">
        <BrowserColumn
          icon={<Database className="h-4 w-4" />}
          title="Catalogs"
          isLoading={catalogs.isLoading}
          error={catalogs.error}
          isEmpty={(catalogs.data?.length ?? 0) === 0}
          hasNextPage={catalogs.hasNextPage}
          isFetchingNextPage={catalogs.isFetchingNextPage}
          onLoadMore={() => catalogs.fetchNextPage()}
        >
          {catalogs.data?.map((item) => (
            <Row
              key={item.name}
              label={item.name}
              sublabel={item.comment}
              selected={item.name === catalog}
              hasChildren
              onMouseEnter={() =>
                item.name && prefetchSchemas(queryClient, item.name)
              }
              onClick={() => {
                setCatalog(item.name);
                setSchema(undefined);
                setTable(undefined);
              }}
            />
          ))}
        </BrowserColumn>

        <BrowserColumn
          icon={<FolderTree className="h-4 w-4" />}
          title="Schemas"
          placeholder={!catalog ? "Select a catalog" : undefined}
          isLoading={schemas.isLoading}
          error={schemas.error}
          isEmpty={(schemas.data?.length ?? 0) === 0}
          hasNextPage={schemas.hasNextPage}
          isFetchingNextPage={schemas.isFetchingNextPage}
          onLoadMore={() => schemas.fetchNextPage()}
        >
          {schemas.data?.map((item) => (
            <Row
              key={item.name}
              label={item.name}
              sublabel={item.comment}
              selected={item.name === schema}
              hasChildren
              onClick={() => {
                setSchema(item.name);
                setTable(undefined);
              }}
            />
          ))}
        </BrowserColumn>

        <BrowserColumn
          icon={<Table2 className="h-4 w-4" />}
          title="Tables"
          placeholder={!schema ? "Select a schema" : undefined}
          isLoading={tables.isLoading}
          error={tables.error}
          isEmpty={(tables.data?.length ?? 0) === 0}
          hasNextPage={tables.hasNextPage}
          isFetchingNextPage={tables.isFetchingNextPage}
          onLoadMore={() => tables.fetchNextPage()}
        >
          {tables.data?.map((item) => (
            <Row
              key={tableFullName(item)}
              label={item.name}
              sublabel={item.comment}
              selected={!!table && tableFullName(item) === tableFullName(table)}
              onClick={() => setTable(item)}
            />
          ))}
        </BrowserColumn>
      </div>

      {table && (
        <TableDetail table={table} onClose={() => setTable(undefined)} />
      )}
    </div>
  );
}

function BrowserColumn({
  icon,
  title,
  placeholder,
  isLoading,
  error,
  isEmpty,
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  placeholder?: string;
  isLoading: boolean;
  error: unknown;
  isEmpty: boolean;
  hasNextPage?: boolean;
  isFetchingNextPage?: boolean;
  onLoadMore?: () => void;
  children: React.ReactNode;
}) {
  return (
    <div className="flex min-h-0 flex-col border-r last:border-r-0">
      <div className="flex items-center gap-2 border-b bg-card px-3 py-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        {icon}
        {title}
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-1">
        {placeholder ? (
          <Empty>{placeholder}</Empty>
        ) : isLoading ? (
          <Empty>
            <span className="inline-block h-4 w-4 animate-spin rounded-full border-2 border-muted border-t-primary align-middle" />
          </Empty>
        ) : error ? (
          <Empty>
            <span className="text-destructive">Failed to load.</span>
          </Empty>
        ) : isEmpty ? (
          <Empty>Nothing here.</Empty>
        ) : (
          <>
            {children}
            {hasNextPage && (
              <Button
                variant="ghost"
                size="sm"
                className="mt-1 w-full justify-center text-xs"
                disabled={isFetchingNextPage}
                onClick={onLoadMore}
              >
                {isFetchingNextPage ? "Loading…" : "Load more"}
              </Button>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function Row({
  label,
  sublabel,
  selected,
  hasChildren,
  onClick,
  onMouseEnter,
}: {
  label?: string;
  sublabel?: string;
  selected?: boolean;
  hasChildren?: boolean;
  onClick?: () => void;
  onMouseEnter?: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      onMouseEnter={onMouseEnter}
      className={cn(
        "flex w-full items-center justify-between gap-2 rounded px-2 py-1.5 text-left text-sm hover:bg-accent",
        selected && "bg-accent text-accent-foreground",
      )}
    >
      <span className="min-w-0">
        <span className="block truncate font-medium">{label}</span>
        {sublabel && (
          <span className="block truncate text-xs text-muted-foreground">
            {sublabel}
          </span>
        )}
      </span>
      {hasChildren && (
        <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
      )}
    </button>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex h-full items-center justify-center p-4 text-center text-sm text-muted-foreground">
      {children}
    </div>
  );
}

function TableDetail({
  table,
  onClose,
}: {
  table: TableInfo;
  onClose: () => void;
}) {
  return (
    <div className="max-h-[40vh] overflow-auto border-t bg-card">
      <div className="flex items-center justify-between border-b px-6 py-3">
        <div className="flex items-center gap-2">
          <Table2 className="h-4 w-4" />
          <span className="font-mono text-sm font-medium">
            {tableFullName(table) || table.name}
          </span>
        </div>
        <Button variant="ghost" size="sm" onClick={onClose}>
          Close
        </Button>
      </div>

      <dl className="grid grid-cols-2 gap-x-6 gap-y-1 px-6 py-3 text-sm sm:grid-cols-3">
        <Meta label="Owner" value={table.owner} />
        <Meta label="Storage location" value={table.storage_location} />
        <Meta label="Comment" value={table.comment} />
      </dl>

      <div className="px-6 pb-4">
        <div className="mb-2 flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          <Columns3 className="h-4 w-4" />
          Columns
        </div>
        {table.columns && table.columns.length > 0 ? (
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b text-left text-xs text-muted-foreground">
                <th className="py-1 pr-4 font-medium">Name</th>
                <th className="py-1 pr-4 font-medium">Type</th>
                <th className="py-1 font-medium">Comment</th>
              </tr>
            </thead>
            <tbody>
              {table.columns.map((col) => (
                <tr key={col.name} className="border-b last:border-b-0">
                  <td className="py-1 pr-4 font-mono">{col.name}</td>
                  <td className="py-1 pr-4 text-muted-foreground">
                    {col.type_text ?? "—"}
                  </td>
                  <td className="py-1 text-muted-foreground">
                    {col.comment ?? ""}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <p className="text-sm text-muted-foreground">No column metadata.</p>
        )}
      </div>
    </div>
  );
}

function Meta({ label, value }: { label: string; value?: string }) {
  return (
    <div className="min-w-0">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="truncate" title={value}>
        {value || "—"}
      </dd>
    </div>
  );
}
