// Catalog dialog orchestration.
//
// Tree rows and the detail pane trigger create/edit/delete flows through this
// context instead of threading callbacks down the tree. The provider owns the
// transient dialog request state and renders the matching dialog.
import {
  createContext,
  type ReactNode,
  useContext,
  useMemo,
  useState,
} from "react";

import { CreateEntityDialog } from "@/components/CreateEntityDialog";
import { DeleteEntityDialog } from "@/components/DeleteEntityDialog";
import { EditEntityDialog } from "@/components/EditEntityDialog";

import type { CreateRequest, DeleteRequest, EditRequest } from "./dialog-types";

export type {
  CreateRequest,
  DeletableKind,
  DeleteRequest,
  EditableKind,
  EditRequest,
} from "./dialog-types";

interface CatalogDialogsValue {
  create: (req: CreateRequest) => void;
  edit: (req: EditRequest) => void;
  remove: (req: DeleteRequest) => void;
}

const CatalogDialogsContext = createContext<CatalogDialogsValue | undefined>(
  undefined,
);

export function CatalogDialogsProvider({ children }: { children: ReactNode }) {
  const [createReq, setCreateReq] = useState<CreateRequest>();
  const [editReq, setEditReq] = useState<EditRequest>();
  const [deleteReq, setDeleteReq] = useState<DeleteRequest>();

  const value = useMemo<CatalogDialogsValue>(
    () => ({
      create: setCreateReq,
      edit: setEditReq,
      remove: setDeleteReq,
    }),
    [],
  );

  return (
    <CatalogDialogsContext.Provider value={value}>
      {children}
      {createReq && (
        <CreateEntityDialog
          request={createReq}
          onClose={() => setCreateReq(undefined)}
        />
      )}
      {editReq && (
        <EditEntityDialog
          request={editReq}
          onClose={() => setEditReq(undefined)}
        />
      )}
      {deleteReq && (
        <DeleteEntityDialog
          request={deleteReq}
          onClose={() => setDeleteReq(undefined)}
        />
      )}
    </CatalogDialogsContext.Provider>
  );
}

export function useCatalogDialogs(): CatalogDialogsValue {
  const ctx = useContext(CatalogDialogsContext);
  if (!ctx) {
    throw new Error(
      "useCatalogDialogs must be used within a CatalogDialogsProvider",
    );
  }
  return ctx;
}
