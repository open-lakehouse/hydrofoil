# Regenerating the Storybook fixtures

The fixtures in this directory are **hand-curated for realism**, not script-generated —
so the "generator" is the prompt below. Paste it to an agent (or follow it yourself) to
re-author the dataset after the data shapes change, or to extend the world. The agent
must finish with the validation loop green; the fixtures test (`fixtures.test.ts`) is the
correctness gate.

## What lives here

- `data/catalog.ts` — Unity Catalog world: catalogs, schemas, tables (+ columns), volumes,
  functions, models, credentials, external locations. Typed against `@open-lakehouse/uc-client`.
- `data/tags.ts` — governed tag policies + entity tag assignments (portal Tags). Built with
  protobuf `create(...)` against `@/gen/portal/tags/v1`.
- `data/files.ts` — a volume file tree + file/dir metadata (portal Files). Built with
  `create(...)` against `@/gen/portal/files/v1`.
- `arrow.ts` — real Arrow tables serialized to IPC bytes for query/result surfaces.
- `schemas/*.json` — generated JSON Schemas the fixtures validate against. **Do not hand-edit**;
  regenerate with `npm run gen:fixture-schemas` (see below).
- `fixtures.test.ts` — the validation gate.
- `index.ts` — barrel + a mock `ActiveEnvironment`.

---

## Paste-ready prompt

> **Task:** (Re)author the curated Storybook fixtures under
> `node/ui/src/lib/fixtures/`. The goal is a **believable, internally-consistent
> lakehouse** that reads well in a component showcase — real-sounding catalog /
> schema / table / column names and plausible types and values, not random or
> placeholder data. Keep it compact (a handful of each entity), not exhaustive.
>
> **Produce/refresh these domains** (counts are a guide, adjust for realism):
> - 2 catalogs (e.g. a production `main` and a read-only `samples`).
> - 3 schemas across those catalogs.
> - ~4 tables, each with 4–6 realistic columns (mix MANAGED/EXTERNAL, DELTA/PARQUET).
> - 2 volumes (one MANAGED, one EXTERNAL).
> - 1 function, 1 registered model.
> - 1 credential, 1 external location (metastore-level).
> - 3 tag policies (include one with no allowed values) + ~5 assignments across entity types
>   (catalogs/schemas/tables/columns).
> - A small file tree (a couple of directories + files) under the Home volume and one UC volume,
>   plus standalone file + directory metadata.
> - 3 Arrow result sets: a small mixed-type table, an empty (zero-row) table, and a wider table.
>
> **Authoritative shapes — match these exactly:**
> - Unity Catalog entities: the generated types in `@open-lakehouse/uc-client`
>   (`CatalogInfo`, `SchemaInfo`, `TableInfo`, `ColumnInfo`, `VolumeInfo`, `FunctionInfo`,
>   `RegisteredModelInfo`, `CredentialInfo`, `ExternalLocationInfo`) — and the JSON Schemas in
>   `./schemas/*-info.json`, which carry the enum constraints (e.g. `table_type`,
>   `data_source_format`, `type_name`, `volume_type`, `parameter_style`). Read the enums before
>   inventing values.
> - Tags/Files: the generated message types in `@/gen/portal/tags/v1/models_pb` and
>   `@/gen/portal/files/v1/svc_pb` — build with `create(Schema, {...})`. Use camelCase fields
>   (e.g. `tagKey`, `entityName`, `fileSize`, `lastModified`) and `bigint` for 64-bit fields.
> - Field names served to the UI must line up with the query layer: see
>   `node/ui/src/lib/uc/queries.ts` (snake_case params/paths and the list-envelope keys the
>   fixture-fetch returns).
>
> **Referential integrity (required):**
> - Every schema's `catalog_name` resolves to a catalog defined here; every table/volume/function/
>   model's `catalog_name` + `schema_name` resolve to a schema defined here; `full_name` is the
>   dot-joined three-level name.
> - Every tag assignment's `tagKey` references a defined policy; when the policy restricts values,
>   the assignment's `tagValue` is one of them; every `entityName` references an entity defined here.
> - Arrow result columns should plausibly correspond to a table in the catalog world.
>
> **Determinism:** use fixed epoch-ms timestamp constants (never `Date.now()` / `new Date()`), so
> fixtures are stable across runs and snapshots.
>
> **Validation loop — run before declaring done, iterate until all green:**
> 1. `npm run gen:fixture-schemas --prefix node/ui` — regenerate the JSON Schemas. On a clean
>    dataset this should produce **no unexpected schema diff**.
> 2. `npm run test --prefix node/ui` — the Ajv `fixtures.test.ts` must pass (it validates every
>    UC fixture against its schema, serializes each portal fixture to wire JSON and validates it,
>    round-trips the Arrow IPC, and checks referential integrity).
> 3. `npm run build --prefix node/ui` — `tsc --noEmit` must pass (fixtures are typed against the
>    real generated types).
> 4. `npm run lint --prefix node` — biome clean.
>
> Done = steps 2–4 pass and step 1 shows no unintended schema churn. If validation fails, the
> fixture is wrong (or a shape genuinely changed) — fix the fixture, don't weaken the test.
