import type {
  FunctionInfo,
  RegisteredModelInfo,
  TableInfo,
  VolumeInfo,
} from "@open-lakehouse/uc-client";
import {
  type UseInfiniteQueryResult,
  useQueryClient,
} from "@tanstack/react-query";
import { createLazyRoute } from "@tanstack/react-router";
import {
  Boxes,
  ChevronDown,
  ChevronRight,
  Columns3,
  Database,
  FolderTree,
  FunctionSquare,
  HardDrive,
  Plus,
  Table2,
  X,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import {
  CreateEntityDialog,
  type CreateRequest,
} from "@/components/CreateEntityDialog";
import { Button } from "@/components/ui/button";
import {
  objectFullName,
  prefetchSchemas,
  useCatalogs,
  useFunctions,
  useModels,
  useSchemas,
  useTables,
  useVolumes,
} from "@/lib/uc/queries";
import { cn } from "@/lib/utils";

export const Route = createLazyRoute("/catalog")({
  component: CatalogPage,
});

// ── Selection model ─────────────────────────────────────────────────────────

type ObjectKind = "table" | "volume" | "function" | "model";

type Selected =
  | { kind: "table"; fullName: string; data: TableInfo }
  | { kind: "volume"; fullName: string; data: VolumeInfo }
  | { kind: "function"; fullName: string; data: FunctionInfo }
  | { kind: "model"; fullName: string; data: RegisteredModelInfo };

interface TreeContext {
  selected: Selected | undefined;
  select: (next: Selected) => void;
  create: (request: CreateRequest) => void;
}

function CatalogPage() {
  const [selected, setSelected] = useState<Selected>();
  const [createRequest, setCreateRequest] = useState<CreateRequest>();
  const tree: TreeContext = {
    selected,
    select: setSelected,
    create: setCreateRequest,
  };

  return (
    <div className="flex h-[calc(100vh-3rem)] flex-col">
      <div className="border-b px-6 py-4">
        <h1 className="flex items-center gap-2 text-lg font-semibold">
          <Database className="h-5 w-5" />
          Unity Catalog
        </h1>
        <p className="text-sm text-muted-foreground">
          Browse catalogs, schemas, tables, volumes, functions, and models.
        </p>
      </div>

      <div className="grid min-h-0 flex-1 grid-cols-1 overflow-hidden md:grid-cols-[minmax(18rem,24rem)_minmax(0,1fr)]">
        <CatalogTree tree={tree} />
        <DetailPane
          selected={selected}
          onClose={() => setSelected(undefined)}
        />
      </div>

      {createRequest && (
        <CreateEntityDialog
          request={createRequest}
          onClose={() => setCreateRequest(undefined)}
        />
      )}
    </div>
  );
}

// ── Tree ────────────────────────────────────────────────────────────────────

function CatalogTree({ tree }: { tree: TreeContext }) {
  const queryClient = useQueryClient();
  const catalogs = useCatalogs();

  return (
    <div className="flex min-h-0 flex-col border-r">
      <div className="flex items-center justify-between border-b bg-card px-3 py-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        <span className="flex items-center gap-2">
          <Database className="h-4 w-4" />
          Catalogs
        </span>
        <Button
          variant="ghost"
          size="sm"
          className="h-6 px-1.5 text-xs"
          onClick={() => tree.create({ kind: "catalog" })}
        >
          <Plus className="h-3.5 w-3.5" />
          New
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-1">
        <ListStates
          isLoading={catalogs.isLoading}
          error={catalogs.error}
          isEmpty={(catalogs.data?.length ?? 0) === 0}
          hasNextPage={catalogs.hasNextPage}
          isFetchingNextPage={catalogs.isFetchingNextPage}
          onLoadMore={() => catalogs.fetchNextPage()}
        >
          {catalogs.data?.map((catalog) => (
            <CatalogNode
              key={catalog.name}
              name={catalog.name ?? ""}
              tree={tree}
              onPrefetch={() =>
                catalog.name && prefetchSchemas(queryClient, catalog.name)
              }
            />
          ))}
        </ListStates>
      </div>
    </div>
  );
}

function CatalogNode({
  name,
  tree,
  onPrefetch,
}: {
  name: string;
  tree: TreeContext;
  onPrefetch: () => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <TreeRow
        depth={0}
        icon={<Database className="h-4 w-4 text-muted-foreground" />}
        label={name}
        expandable
        open={open}
        onToggle={() => setOpen((o) => !o)}
        onMouseEnter={onPrefetch}
        action={
          <CreateAction
            title="New schema"
            onClick={() => tree.create({ kind: "schema", catalog: name })}
          />
        }
      />
      {open && <SchemaList catalog={name} tree={tree} />}
    </div>
  );
}

function SchemaList({ catalog, tree }: { catalog: string; tree: TreeContext }) {
  const schemas = useSchemas(catalog);
  return (
    <ListStates
      depth={1}
      isLoading={schemas.isLoading}
      error={schemas.error}
      isEmpty={(schemas.data?.length ?? 0) === 0}
      hasNextPage={schemas.hasNextPage}
      isFetchingNextPage={schemas.isFetchingNextPage}
      onLoadMore={() => schemas.fetchNextPage()}
    >
      {schemas.data?.map((schema) => (
        <SchemaNode
          key={schema.name}
          catalog={catalog}
          schema={schema.name ?? ""}
          tree={tree}
        />
      ))}
    </ListStates>
  );
}

function SchemaNode({
  catalog,
  schema,
  tree,
}: {
  catalog: string;
  schema: string;
  tree: TreeContext;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <TreeRow
        depth={1}
        icon={<FolderTree className="h-4 w-4 text-muted-foreground" />}
        label={schema}
        expandable
        open={open}
        onToggle={() => setOpen((o) => !o)}
      />
      {open &&
        GROUPS.map((group) => (
          <GroupNode
            key={group.kind}
            group={group}
            catalog={catalog}
            schema={schema}
            tree={tree}
          />
        ))}
    </div>
  );
}

interface GroupDef {
  kind: ObjectKind;
  title: string;
  icon: ReactNode;
  // Only the kinds with a low-complexity create form expose an inline "+".
  creatable?: "volume" | "model";
  useList: (
    catalog: string | undefined,
    schema: string | undefined,
  ) => UseInfiniteQueryResult<
    (TableInfo | VolumeInfo | FunctionInfo | RegisteredModelInfo)[],
    unknown
  >;
}

const GROUPS: GroupDef[] = [
  {
    kind: "table",
    title: "Tables",
    icon: <Table2 className="h-4 w-4 text-muted-foreground" />,
    useList: useTables as GroupDef["useList"],
  },
  {
    kind: "volume",
    title: "Volumes",
    icon: <HardDrive className="h-4 w-4 text-muted-foreground" />,
    creatable: "volume",
    useList: useVolumes as GroupDef["useList"],
  },
  {
    kind: "function",
    title: "Functions",
    icon: <FunctionSquare className="h-4 w-4 text-muted-foreground" />,
    useList: useFunctions as GroupDef["useList"],
  },
  {
    kind: "model",
    title: "Models",
    icon: <Boxes className="h-4 w-4 text-muted-foreground" />,
    creatable: "model",
    useList: useModels as GroupDef["useList"],
  },
];

function GroupNode({
  group,
  catalog,
  schema,
  tree,
}: {
  group: GroupDef;
  catalog: string;
  schema: string;
  tree: TreeContext;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <TreeRow
        depth={2}
        icon={group.icon}
        label={group.title}
        expandable
        open={open}
        onToggle={() => setOpen((o) => !o)}
        action={
          group.creatable ? (
            <CreateAction
              title={`New ${group.creatable}`}
              onClick={() =>
                tree.create({
                  kind: group.creatable as "volume" | "model",
                  catalog,
                  schema,
                })
              }
            />
          ) : undefined
        }
      />
      {open && (
        <ObjectList
          group={group}
          catalog={catalog}
          schema={schema}
          tree={tree}
        />
      )}
    </div>
  );
}

function ObjectList({
  group,
  catalog,
  schema,
  tree,
}: {
  group: GroupDef;
  catalog: string;
  schema: string;
  tree: TreeContext;
}) {
  const query = group.useList(catalog, schema);
  return (
    <ListStates
      depth={3}
      isLoading={query.isLoading}
      error={query.error}
      isEmpty={(query.data?.length ?? 0) === 0}
      hasNextPage={query.hasNextPage}
      isFetchingNextPage={query.isFetchingNextPage}
      onLoadMore={() => query.fetchNextPage()}
    >
      {query.data?.map((item) => {
        const fullName =
          ("full_name" in item && item.full_name) || objectFullName(item);
        const isSelected =
          tree.selected?.kind === group.kind &&
          tree.selected.fullName === fullName;
        return (
          <TreeRow
            key={fullName || item.name}
            depth={3}
            icon={group.icon}
            label={item.name ?? fullName}
            selected={isSelected}
            onClick={() =>
              tree.select({
                kind: group.kind,
                fullName,
                data: item,
              } as Selected)
            }
          />
        );
      })}
    </ListStates>
  );
}

// ── Shared tree presentation ─────────────────────────────────────────────────

function TreeRow({
  depth,
  icon,
  label,
  expandable,
  open,
  selected,
  action,
  onToggle,
  onClick,
  onMouseEnter,
}: {
  depth: number;
  icon: ReactNode;
  label?: string;
  expandable?: boolean;
  open?: boolean;
  selected?: boolean;
  action?: ReactNode;
  onToggle?: () => void;
  onClick?: () => void;
  onMouseEnter?: () => void;
}) {
  return (
    <div
      className={cn(
        "group flex items-center rounded pr-1 hover:bg-accent",
        selected && "bg-accent text-accent-foreground",
      )}
    >
      <button
        type="button"
        onClick={expandable ? onToggle : onClick}
        onMouseEnter={onMouseEnter}
        style={{ paddingLeft: `${depth * 0.875 + 0.5}rem` }}
        className="flex min-w-0 flex-1 items-center gap-1.5 px-2 py-1.5 text-left text-sm"
      >
        <span className="flex h-4 w-4 shrink-0 items-center justify-center text-muted-foreground">
          {expandable ? (
            open ? (
              <ChevronDown className="h-3.5 w-3.5" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5" />
            )
          ) : null}
        </span>
        {icon}
        <span className="min-w-0 flex-1 truncate font-medium">{label}</span>
      </button>
      {action && (
        <span className="opacity-0 transition-opacity group-hover:opacity-100">
          {action}
        </span>
      )}
    </div>
  );
}

function CreateAction({
  title,
  onClick,
}: {
  title: string;
  onClick: () => void;
}) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      title={title}
      aria-label={title}
      className="h-6 w-6 p-0"
      onClick={onClick}
    >
      <Plus className="h-3.5 w-3.5" />
    </Button>
  );
}

function ListStates({
  depth = 0,
  isLoading,
  error,
  isEmpty,
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
  children,
}: {
  depth?: number;
  isLoading: boolean;
  error: unknown;
  isEmpty: boolean;
  hasNextPage?: boolean;
  isFetchingNextPage?: boolean;
  onLoadMore?: () => void;
  children: ReactNode;
}) {
  const pad = { paddingLeft: `${depth * 0.875 + 1.75}rem` } as const;
  if (isLoading) {
    return (
      <div style={pad} className="py-1.5 text-sm text-muted-foreground">
        <span className="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-muted border-t-primary align-middle" />
      </div>
    );
  }
  if (error) {
    return (
      <div style={pad} className="py-1.5 text-sm text-destructive">
        Failed to load.
      </div>
    );
  }
  if (isEmpty) {
    return (
      <div style={pad} className="py-1.5 text-sm text-muted-foreground">
        Empty.
      </div>
    );
  }
  return (
    <>
      {children}
      {hasNextPage && (
        <Button
          variant="ghost"
          size="sm"
          style={pad}
          className="w-full justify-start text-xs"
          disabled={isFetchingNextPage}
          onClick={onLoadMore}
        >
          {isFetchingNextPage ? "Loading…" : "Load more"}
        </Button>
      )}
    </>
  );
}

// ── Detail pane ───────────────────────────────────────────────────────────────

function DetailPane({
  selected,
  onClose,
}: {
  selected: Selected | undefined;
  onClose: () => void;
}) {
  if (!selected) {
    return (
      <div className="flex min-h-0 items-center justify-center p-8 text-center text-sm text-muted-foreground">
        Select an object from the tree to see its details.
      </div>
    );
  }

  const icon = GROUPS.find((g) => g.kind === selected.kind)?.icon;

  return (
    <div className="flex min-h-0 flex-col overflow-auto">
      <div className="flex items-center justify-between border-b bg-card px-6 py-3">
        <div className="flex items-center gap-2">
          {icon}
          <span className="font-mono text-sm font-medium">
            {selected.fullName || selected.data.name}
          </span>
          <span className="rounded bg-muted px-1.5 py-0.5 text-xs uppercase tracking-wide text-muted-foreground">
            {selected.kind}
          </span>
        </div>
        <Button variant="ghost" size="sm" onClick={onClose}>
          <X className="h-4 w-4" />
        </Button>
      </div>
      <div className="px-6 py-4">
        {selected.kind === "table" && <TableDetail table={selected.data} />}
        {selected.kind === "volume" && <VolumeDetail volume={selected.data} />}
        {selected.kind === "function" && <FunctionDetail fn={selected.data} />}
        {selected.kind === "model" && <ModelDetail model={selected.data} />}
      </div>
    </div>
  );
}

function TableDetail({ table }: { table: TableInfo }) {
  return (
    <>
      <dl className="grid grid-cols-2 gap-x-6 gap-y-1 text-sm sm:grid-cols-3">
        <Meta label="Owner" value={table.owner} />
        <Meta label="Type" value={table.table_type} />
        <Meta label="Format" value={table.data_source_format} />
        <Meta label="Storage location" value={table.storage_location} />
        <Meta label="Comment" value={table.comment} />
      </dl>

      <div className="mt-4">
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
    </>
  );
}

function VolumeDetail({ volume }: { volume: VolumeInfo }) {
  return (
    <dl className="grid grid-cols-2 gap-x-6 gap-y-1 text-sm sm:grid-cols-3">
      <Meta label="Owner" value={volume.owner} />
      <Meta label="Volume type" value={volume.volume_type} />
      <Meta label="Storage location" value={volume.storage_location} />
      <Meta label="Comment" value={volume.comment} />
    </dl>
  );
}

function FunctionDetail({ fn }: { fn: FunctionInfo }) {
  return (
    <>
      <dl className="grid grid-cols-2 gap-x-6 gap-y-1 text-sm sm:grid-cols-3">
        <Meta label="Owner" value={fn.owner} />
        <Meta label="Return type" value={fn.full_data_type ?? fn.data_type} />
        <Meta label="Routine body" value={fn.routine_body} />
        <Meta label="SQL data access" value={fn.sql_data_access} />
        <Meta label="Comment" value={fn.comment} />
      </dl>
      {fn.routine_definition && (
        <div className="mt-4">
          <div className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Definition
          </div>
          <pre className="overflow-auto rounded bg-muted p-3 text-xs">
            {fn.routine_definition}
          </pre>
        </div>
      )}
    </>
  );
}

function ModelDetail({ model }: { model: RegisteredModelInfo }) {
  return (
    <dl className="grid grid-cols-2 gap-x-6 gap-y-1 text-sm sm:grid-cols-3">
      <Meta label="Owner" value={model.owner} />
      <Meta label="Storage location" value={model.storage_location} />
      <Meta label="Comment" value={model.comment} />
    </dl>
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
