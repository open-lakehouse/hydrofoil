// Sidebar section for the metastore-level storage securables: external
// locations and storage credentials. Mirrors the catalog tree's row/selection
// behavior but for flat (non-namespaced) lists.
import type {
  CredentialInfo,
  ExternalLocationInfo,
} from "@open-lakehouse/uc-client";
import type { UseInfiniteQueryResult } from "@tanstack/react-query";
import {
  Globe,
  HardDrive,
  KeyRound,
  type LucideIcon,
  Pencil,
  Trash2,
} from "lucide-react";
import { useState } from "react";
import { useCatalogDialogs } from "@/components/catalog/dialogs";
import { RowMenu } from "@/components/catalog/RowMenu";
import { useCatalogSelection } from "@/components/catalog/selection";
import {
  CreateAction,
  ListStates,
  TreeRow,
} from "@/components/catalog/TreeRow";
import type { StorageKind } from "@/components/catalog/types";
import { useCredentials, useExternalLocations } from "@/lib/uc/queries";

type StorageItem = CredentialInfo | ExternalLocationInfo;
type StorageList = UseInfiniteQueryResult<StorageItem[], unknown>;

interface StorageGroupDef {
  kind: StorageKind;
  title: string;
  createLabel: string;
  Icon: LucideIcon;
  useList: () => StorageList;
}

const GROUPS: StorageGroupDef[] = [
  {
    kind: "external_location",
    title: "External Locations",
    createLabel: "New external location",
    Icon: Globe,
    useList: useExternalLocations as () => StorageList,
  },
  {
    kind: "credential",
    title: "Credentials",
    createLabel: "New credential",
    Icon: KeyRound,
    useList: useCredentials as () => StorageList,
  },
];

export function StorageTree() {
  return (
    <div className="flex max-h-[45%] min-h-0 flex-col border-t">
      <div className="flex items-center gap-2 border-b px-3 py-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        <HardDrive className="h-4 w-4" />
        External data
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-1">
        {GROUPS.map((group) => (
          <StorageGroup key={group.kind} group={group} />
        ))}
      </div>
    </div>
  );
}

function StorageGroup({ group }: { group: StorageGroupDef }) {
  const [open, setOpen] = useState(true);
  const dialogs = useCatalogDialogs();

  return (
    <div>
      <TreeRow
        depth={0}
        icon={<group.Icon className="h-4 w-4 text-muted-foreground" />}
        label={group.title}
        expandable
        open={open}
        onToggle={() => setOpen((v) => !v)}
        action={
          <CreateAction
            title={group.createLabel}
            onClick={() => dialogs.create({ kind: group.kind })}
          />
        }
      />
      {open && <StorageList group={group} />}
    </div>
  );
}

function StorageList({ group }: { group: StorageGroupDef }) {
  const query = group.useList();
  const { selection, select } = useCatalogSelection();
  const dialogs = useCatalogDialogs();

  return (
    <ListStates
      depth={1}
      isLoading={query.isLoading}
      error={query.error}
      isEmpty={(query.data?.length ?? 0) === 0}
      hasNextPage={query.hasNextPage}
      isFetchingNextPage={query.isFetchingNextPage}
      onLoadMore={() => query.fetchNextPage()}
    >
      {query.data?.map((item) => {
        const name = item.name ?? "";
        const selected =
          selection?.kind === group.kind && selection.fullName === name;
        return (
          <TreeRow
            key={name}
            depth={1}
            icon={<group.Icon className="h-4 w-4 text-muted-foreground" />}
            label={name}
            selected={selected}
            onSelect={() => select({ kind: group.kind, fullName: name })}
            action={
              <RowMenu
                label={`${name} actions`}
                items={[
                  {
                    label: "Edit",
                    icon: <Pencil />,
                    onSelect: () => dialogs.edit({ kind: group.kind, name }),
                  },
                  {
                    label: "Delete",
                    icon: <Trash2 />,
                    variant: "destructive",
                    separatorBefore: true,
                    onSelect: () => dialogs.remove({ kind: group.kind, name }),
                  },
                ]}
              />
            }
          />
        );
      })}
    </ListStates>
  );
}
