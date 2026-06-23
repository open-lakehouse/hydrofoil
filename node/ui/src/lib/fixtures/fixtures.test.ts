// Correctness gate for the curated Storybook fixtures. Three kinds of checks:
//
//   1. UNITY CATALOG fixtures are hand-authored plain JSON, so they get the most
//      value from schema validation: each is checked against the JSON Schema
//      extracted from the OpenAPI spec the TS client is generated from. This
//      catches field-name typos, bad enum values, wrong types — the easy ways
//      hand-written data drifts from the real shape.
//
//   2. PORTAL fixtures (Tags/Files) are constructed with protobuf `create`, so
//      they are already structurally valid messages. We additionally serialize
//      them with `toJson` to the canonical wire shape and validate against the
//      proto-derived schema (proving field names/enums line up). int64 fields
//      serialize to JSON strings per the proto3 JSON mapping, while the schema
//      types them as integers — we coerce those before validating.
//
//   3. ARROW fixtures have no JSON shape (opaque `bytes` on the wire), so we
//      prove them by round-tripping the IPC bytes back through `tableFromIPC`
//      and asserting the decoded row/column shape.
//
// Also asserts referential integrity across the dataset (tags/files reference
// entities that exist), since a believable showcase depends on it.

import { type DescMessage, type Message, toJson } from "@bufbuild/protobuf";
import Ajv2020, { type ValidateFunction } from "ajv/dist/2020";
import { tableFromIPC } from "apache-arrow";
import { describe, expect, it } from "vitest";
import {
  DirectoryEntrySchema,
  DirectoryMetadataSchema,
  FileMetadataSchema,
} from "@/gen/portal/files/v1/svc_pb";
import {
  EntityTagAssignmentSchema,
  TagPolicySchema,
} from "@/gen/portal/tags/v1/models_pb";

import * as arrow from "./arrow";
import {
  catalogs,
  credentials,
  externalLocations,
  functions,
  models,
  schemas,
  tables,
  volumes,
} from "./data/catalog";
import {
  directoryMetadata,
  fileMetadata,
  homeEntries,
  queryEntries,
  rawFilesEntries,
} from "./data/files";
import { tagAssignments, tagPolicies } from "./data/tags";

// JSON Schemas (checked in by scripts/gen-fixture-schemas.mjs).
import catalogInfo from "./schemas/catalog-info.json";
import credentialInfo from "./schemas/credential-info.json";
import directoryEntrySchema from "./schemas/directory-entry.json";
import directoryMetadataSchema from "./schemas/directory-metadata.json";
import entityTagAssignmentSchema from "./schemas/entity-tag-assignment.json";
import externalLocationInfo from "./schemas/external-location-info.json";
import fileMetadataSchemaJson from "./schemas/file-metadata.json";
import functionInfo from "./schemas/function-info.json";
import registeredModelInfo from "./schemas/registered-model-info.json";
import schemaInfo from "./schemas/schema-info.json";
import tableInfo from "./schemas/table-info.json";
import tagPolicySchema from "./schemas/tag-policy.json";
import volumeInfo from "./schemas/volume-info.json";

const ajv = new Ajv2020({ allErrors: true, strict: false });
// proto-derived schemas annotate int64/uint64 fields with these formats; register
// them as no-ops so Ajv validates the (coerced) values without logging warnings.
for (const fmt of ["int64", "uint64", "int32", "uint32"])
  ajv.addFormat(fmt, true);

function compile(schema: object): ValidateFunction {
  return ajv.compile(schema);
}

function expectValid(
  validate: ValidateFunction,
  value: unknown,
  label: string,
) {
  const ok = validate(value);
  if (!ok) {
    throw new Error(
      `${label} failed schema validation:\n${ajv.errorsText(validate.errors, {
        separator: "\n",
      })}`,
    );
  }
  expect(ok).toBe(true);
}

// ── 1. Unity Catalog fixtures vs OpenAPI-derived schemas ──────────────────────

describe("unity catalog fixtures", () => {
  const cases: [string, object, unknown[]][] = [
    ["catalogs", catalogInfo, catalogs],
    ["schemas", schemaInfo, schemas],
    ["tables", tableInfo, tables],
    ["volumes", volumeInfo, volumes],
    ["functions", functionInfo, functions],
    ["models", registeredModelInfo, models],
    ["credentials", credentialInfo, credentials],
    ["externalLocations", externalLocationInfo, externalLocations],
  ];

  for (const [name, schema, items] of cases) {
    it(`${name} validate against ${name} schema`, () => {
      const validate = compile(schema);
      items.forEach((item, i) => {
        expectValid(validate, item, `${name}[${i}]`);
      });
    });
  }
});

// ── 2. Portal fixtures: serialize to wire JSON, then validate ─────────────────

// Proto3 JSON maps int64 to a string; the proto-derived schema types those
// fields as integers. Coerce numeric-looking strings to numbers so validation
// reflects the logical shape. (Recurses through arrays/objects.)
function coerceInt64Strings(node: unknown): unknown {
  if (Array.isArray(node)) return node.map(coerceInt64Strings);
  if (node && typeof node === "object") {
    return Object.fromEntries(
      Object.entries(node).map(([k, v]) => [k, coerceInt64Strings(v)]),
    );
  }
  if (typeof node === "string" && /^-?\d+$/.test(node)) return Number(node);
  return node;
}

function toWireJson<T extends Message>(schema: DescMessage, msg: T): unknown {
  // useProtoFieldName -> snake_case keys, matching the proto-bundle schema (which
  // sets additionalProperties:false and uses proto field names).
  return coerceInt64Strings(toJson(schema, msg, { useProtoFieldName: true }));
}

describe("portal fixtures", () => {
  it("tag policies validate", () => {
    const validate = compile(tagPolicySchema);
    tagPolicies.forEach((p, i) => {
      expectValid(validate, toWireJson(TagPolicySchema, p), `tagPolicy[${i}]`);
    });
  });

  it("tag assignments validate", () => {
    const validate = compile(entityTagAssignmentSchema);
    tagAssignments.forEach((a, i) => {
      expectValid(
        validate,
        toWireJson(EntityTagAssignmentSchema, a),
        `tagAssignment[${i}]`,
      );
    });
  });

  it("directory entries validate", () => {
    const validate = compile(directoryEntrySchema);
    const all = [...homeEntries, ...queryEntries, ...rawFilesEntries];
    all.forEach((e, i) => {
      expectValid(
        validate,
        toWireJson(DirectoryEntrySchema, e),
        `dirEntry[${i}]`,
      );
    });
  });

  it("file + directory metadata validate", () => {
    expectValid(
      compile(fileMetadataSchemaJson),
      toWireJson(FileMetadataSchema, fileMetadata),
      "fileMetadata",
    );
    expectValid(
      compile(directoryMetadataSchema),
      toWireJson(DirectoryMetadataSchema, directoryMetadata),
      "directoryMetadata",
    );
  });
});

// ── 3. Arrow fixtures: round-trip the IPC bytes ───────────────────────────────

describe("arrow fixtures", () => {
  const cases: [string, Uint8Array, number, number][] = [
    ["topCustomers", arrow.topCustomersIpc, 5, 4],
    ["empty", arrow.emptyIpc, 0, 3],
    ["trips", arrow.tripsIpc, 6, 6],
  ];

  for (const [name, ipc, rows, cols] of cases) {
    it(`${name} round-trips to ${rows}x${cols}`, () => {
      const table = tableFromIPC(ipc);
      expect(table.numRows).toBe(rows);
      expect(table.schema.fields.length).toBe(cols);
    });
  }

  it("builds a populated ArrowResultStore", () => {
    const store = arrow.storeFromIpc(arrow.topCustomersIpc);
    expect(store.rowCount).toBe(5);
    expect(store.columnCount).toBe(4);
    expect(store.getCell(0, 1)).toBe("Ada Lovelace");
  });
});

// ── Referential integrity ─────────────────────────────────────────────────────

describe("referential integrity", () => {
  const catalogNames = new Set(catalogs.map((c) => c.name));
  const schemaFullNames = new Set(schemas.map((s) => s.full_name));
  const policyKeys = new Set(tagPolicies.map((p) => p.tagKey));

  it("every schema's catalog exists", () => {
    for (const s of schemas)
      expect(catalogNames.has(s.catalog_name)).toBe(true);
  });

  it("every table's schema exists", () => {
    for (const t of tables)
      expect(schemaFullNames.has(`${t.catalog_name}.${t.schema_name}`)).toBe(
        true,
      );
  });

  it("every tag assignment references a defined policy", () => {
    for (const a of tagAssignments) expect(policyKeys.has(a.tagKey)).toBe(true);
  });

  it("restricted-value assignments use an allowed value", () => {
    for (const a of tagAssignments) {
      const policy = tagPolicies.find((p) => p.tagKey === a.tagKey);
      if (policy && policy.values.length > 0 && a.tagValue) {
        expect(policy.values.map((v) => v.name)).toContain(a.tagValue);
      }
    }
  });
});
