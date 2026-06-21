import type { ObjectKind } from "./types";

export type CreateRequest =
  | { kind: "catalog" }
  | { kind: "schema"; catalog: string }
  | { kind: "volume"; catalog: string; schema: string }
  | { kind: "model"; catalog: string; schema: string };

/** Entities that support PATCH (comment / rename). */
export type EditableKind = "catalog" | "schema" | "volume" | "model";

export interface EditRequest {
  kind: EditableKind;
  /** Catalog: name; everything else: fully-qualified name. */
  name: string;
  comment?: string;
}

/** Entities that support DELETE. */
export type DeletableKind = "catalog" | "schema" | ObjectKind;

export interface DeleteRequest {
  kind: DeletableKind;
  name: string;
}
