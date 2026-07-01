// Editable schema model for the import page.
//
// The host returns the file's inferred schema as an Arrow IPC stream. We decode
// it into a flat, editable list of columns (name + a logical type the user can
// override + nullability), then re-encode the user-confirmed schema back to Arrow
// IPC for the IngestTable `targetSchemaIpc` field. The host's CREATE DDL maps each
// Arrow type to a SQL type, so the editable type set here mirrors that allowlist.

import {
  Bool,
  type DataType,
  DateDay,
  Field,
  Float64,
  Int32,
  Int64,
  Schema,
  Table,
  TimestampMicrosecond,
  tableFromIPC,
  tableToIPC,
  Utf8,
} from "apache-arrow";

/** The logical column types the user can choose. Each maps to one Arrow type
 *  (and, on the host, one SQL type in the CREATE DDL). */
export type ColumnType =
  | "string"
  | "int"
  | "bigint"
  | "double"
  | "boolean"
  | "date"
  | "timestamp";

export const COLUMN_TYPES: ColumnType[] = [
  "string",
  "int",
  "bigint",
  "double",
  "boolean",
  "date",
  "timestamp",
];

/** One editable column in the schema editor. */
export interface EditableColumn {
  name: string;
  type: ColumnType;
  nullable: boolean;
}

/** Map an Arrow field's type to the nearest editable column type, defaulting to
 *  `string` for anything outside the supported set (the user can re-pick). We
 *  match on the type's string form (`int32`, `float64`, `timestamp<...>`, …) —
 *  simpler and more stable across arrow versions than `typeId` constants. */
function inferColumnType(type: DataType): ColumnType {
  const name = type.toString().toLowerCase();
  if (name.startsWith("bool")) return "boolean";
  if (
    name.startsWith("int8") ||
    name.startsWith("int16") ||
    name.startsWith("int32")
  )
    return "int";
  if (name.startsWith("int") || name.startsWith("uint")) return "bigint";
  if (
    name.startsWith("float") ||
    name.startsWith("double") ||
    name.startsWith("decimal")
  )
    return "double";
  if (name.startsWith("date")) return "date";
  if (name.startsWith("timestamp")) return "timestamp";
  return "string";
}

/** Build the Arrow `DataType` for an editable column type. */
function arrowType(type: ColumnType): DataType {
  switch (type) {
    case "string":
      return new Utf8();
    case "int":
      return new Int32();
    case "bigint":
      return new Int64();
    case "double":
      return new Float64();
    case "boolean":
      return new Bool();
    case "date":
      return new DateDay();
    case "timestamp":
      return new TimestampMicrosecond();
  }
}

/** Decode a schema-only Arrow IPC stream into the editable column model. */
export function columnsFromSchemaIpc(schemaIpc: Uint8Array): EditableColumn[] {
  const table = tableFromIPC(schemaIpc);
  return table.schema.fields.map((f) => ({
    name: f.name,
    type: inferColumnType(f.type),
    nullable: f.nullable,
  }));
}

/** Encode the user-confirmed columns as a schema-only Arrow IPC stream. */
export function schemaIpcFromColumns(columns: EditableColumn[]): Uint8Array {
  const fields = columns.map(
    (c) => new Field(c.name, arrowType(c.type), c.nullable),
  );
  const schema = new Schema(fields);
  return tableToIPC(new Table(schema), "stream");
}

/** Validate the editable columns before ingest: non-empty, unique names. */
export function validateColumns(columns: EditableColumn[]): string | null {
  if (columns.length === 0) return "the file has no columns";
  const seen = new Set<string>();
  for (const c of columns) {
    const name = c.name.trim();
    if (!name) return "every column must have a name";
    if (seen.has(name)) return `duplicate column name: ${name}`;
    seen.add(name);
  }
  return null;
}
