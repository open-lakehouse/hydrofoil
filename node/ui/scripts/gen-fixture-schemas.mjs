// Generates the JSON Schemas used to VALIDATE the Storybook fixture data, from
// the same definitions the UI renders against. Counterpart to the
// @open-lakehouse/unity-catalog package's own gen-form-schemas.mjs (which
// generates the rjsf FORM schemas); this one targets the *domain* shapes
// (TagPolicy, FileMetadata, CatalogInfo, …).
//
// Two sources, one output dir (src/lib/fixtures/schemas/):
//
//   1. PORTAL (Tags + Files) — from the local protos via buf's
//      protoschema-jsonschema plugin (buf.gen.fixture-schemas.yaml), the same way
//      the form schemas are generated. The portal TS client is proto-generated,
//      so the proto is the faithful source.
//
//   2. UNITY CATALOG entities — extracted from the OpenAPI spec that ships in the
//      @open-lakehouse/unity-catalog package (openapi/unity-catalog.yaml). That
//      spec is the exact source that package's UC TS types are generated from (via
//      openapi-typescript), so it matches what the UI consumes — and needs no
//      network or git access. We resolve intra-spec `$ref`s into a self-contained
//      `$defs` block and stamp a draft 2020-12 `$schema`.
//
// Query/Ingest results are opaque Arrow IPC `bytes` (no JSON shape), so those
// fixtures are validated by round-tripping through apache-arrow in the test.
//
// Regenerate with `npm run gen:fixture-schemas` (or `just gen-fixture-schemas`).
// The portal half requires the `buf` CLI + BSR access; the UC half is offline.

import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parse as parseYaml } from "yaml";

const here = path.dirname(fileURLToPath(import.meta.url));
const uiDir = path.resolve(here, "..");
const tmpDir = path.join(uiDir, ".gen-fixture-jsonschema");
const outDir = path.join(uiDir, "src/lib/fixtures/schemas");
// The UC OpenAPI spec now ships inside the @open-lakehouse/unity-catalog package
// (in the sibling mangrove repo, consumed via a file: link), which exposes it as
// an `exports` subpath. Resolve it via Node's module resolution — robust to npm
// hoisting the link into any ancestor node_modules.
const require = createRequire(import.meta.url);
const openapiPath = require.resolve(
  "@open-lakehouse/unity-catalog/openapi/unity-catalog.yaml",
);

const JSON_SCHEMA_DIALECT = "https://json-schema.org/draft/2020-12/schema";

// ── Portal (proto -> buf protoschema-jsonschema) ─────────────────────────────

/** Output file name (without extension) -> fully-qualified portal proto message. */
const PORTAL_TARGETS = {
  "tag-policy": "portal.tags.v1.TagPolicy",
  "entity-tag-assignment": "portal.tags.v1.EntityTagAssignment",
  "file-metadata": "portal.files.v1.FileMetadata",
  "directory-entry": "portal.files.v1.DirectoryEntry",
  "directory-metadata": "portal.files.v1.DirectoryMetadata",
};

// Recursively normalize a generated proto schema node: strip per-subschema
// `$schema`/`$id`, drop camelCase `patternProperties` aliases (the APIs are
// snake_case), and collapse proto scalar `anyOf` unions to their nicest branch.
// Same logic as gen-form-schemas.mjs.
function clean(node) {
  if (Array.isArray(node)) return node.map(clean);
  if (!node || typeof node !== "object") return node;

  const out = {};
  for (const [key, value] of Object.entries(node)) {
    if (key === "$schema" || key === "$id" || key === "patternProperties")
      continue;
    out[key] = clean(value);
  }

  if (Array.isArray(out.anyOf)) {
    const branches = out.anyOf;
    const chosen =
      branches.find((b) => b && typeof b === "object" && "enum" in b) ??
      branches.find(
        (b) =>
          b &&
          typeof b === "object" &&
          (b.type === "number" || b.type === "integer"),
      ) ??
      branches[0];
    delete out.anyOf;
    if (chosen && typeof chosen === "object") {
      for (const [bk, bv] of Object.entries(chosen)) {
        if (!(bk in out)) out[bk] = bv;
      }
    }
  }

  return out;
}

function portalBundleToSchema(bundle) {
  const defs = bundle.$defs ?? {};
  const rootRef = typeof bundle.$ref === "string" ? bundle.$ref : "";
  const rootKey = rootRef.replace("#/$defs/", "");
  const root = defs[rootKey];
  if (!root) {
    throw new Error(`Could not resolve bundle root $ref: ${rootRef}`);
  }
  const schema = clean({ ...root, $defs: defs });
  schema.$schema = JSON_SCHEMA_DIALECT;
  return schema;
}

function genPortal() {
  console.log("buf generate (protoschema-jsonschema, portal)…");
  execFileSync(
    "buf",
    ["generate", "--template", "buf.gen.fixture-schemas.yaml"],
    { cwd: uiDir, stdio: "inherit" },
  );

  for (const [file, type] of Object.entries(PORTAL_TARGETS)) {
    const bundlePath = path.join(tmpDir, `${type}.schema.bundle.json`);
    if (!fs.existsSync(bundlePath)) {
      throw new Error(`Expected bundle not generated: ${bundlePath}`);
    }
    const bundle = JSON.parse(fs.readFileSync(bundlePath, "utf8"));
    writeSchema(file, portalBundleToSchema(bundle));
  }

  fs.rmSync(tmpDir, { recursive: true, force: true });
}

// ── Unity Catalog (OpenAPI components -> self-contained JSON Schema) ──────────

/** Output file name -> OpenAPI component schema name (`#/components/schemas/X`). */
const UC_TARGETS = {
  "catalog-info": "CatalogInfo",
  "schema-info": "SchemaInfo",
  "table-info": "TableInfo",
  "column-info": "ColumnInfo",
  "volume-info": "VolumeInfo",
  "function-info": "FunctionInfo",
  "registered-model-info": "RegisteredModelInfo",
  "credential-info": "CredentialInfo",
  "external-location-info": "ExternalLocationInfo",
};

// Walk a node and collect every component schema it (transitively) `$ref`s, so
// we can bundle exactly the needed definitions under `$defs`. Rewrites each
// `#/components/schemas/X` ref to `#/$defs/X` in place (on a deep clone).
function rewriteRefsAndCollect(node, deps) {
  if (Array.isArray(node)) return node.map((n) => rewriteRefsAndCollect(n, deps));
  if (!node || typeof node !== "object") return node;

  const out = {};
  for (const [key, value] of Object.entries(node)) {
    if (key === "$ref" && typeof value === "string") {
      const name = value.replace("#/components/schemas/", "");
      deps.add(name);
      out.$ref = `#/$defs/${name}`;
    } else {
      out[key] = rewriteRefsAndCollect(value, deps);
    }
  }
  return out;
}

function genUnityCatalog() {
  console.log(`reading OpenAPI spec ${path.relative(process.cwd(), openapiPath)}…`);
  const spec = parseYaml(fs.readFileSync(openapiPath, "utf8"));
  const components = spec?.components?.schemas ?? {};

  for (const [file, name] of Object.entries(UC_TARGETS)) {
    const root = components[name];
    if (!root) {
      throw new Error(`OpenAPI component not found: ${name}`);
    }

    // Closure over transitive $refs starting from the root.
    const deps = new Set();
    const rootSchema = rewriteRefsAndCollect(root, deps);
    const $defs = {};
    const queue = [...deps];
    while (queue.length) {
      const dep = queue.shift();
      if ($defs[dep]) continue;
      const comp = components[dep];
      if (!comp) throw new Error(`Unresolved $ref: ${dep} (from ${name})`);
      const before = deps.size;
      $defs[dep] = rewriteRefsAndCollect(comp, deps);
      // Newly discovered refs get queued.
      if (deps.size > before) {
        for (const d of deps) if (!$defs[d] && !queue.includes(d)) queue.push(d);
      }
    }

    const schema = { $schema: JSON_SCHEMA_DIALECT, ...rootSchema };
    if (Object.keys($defs).length) schema.$defs = $defs;
    writeSchema(file, schema);
  }
}

// ── Shared ───────────────────────────────────────────────────────────────────

function writeSchema(file, schema) {
  const dest = path.join(outDir, `${file}.json`);
  fs.writeFileSync(dest, `${JSON.stringify(schema, null, 2)}\n`);
  console.log(`wrote ${path.relative(process.cwd(), dest)}`);
}

fs.mkdirSync(outDir, { recursive: true });
genPortal();
genUnityCatalog();
