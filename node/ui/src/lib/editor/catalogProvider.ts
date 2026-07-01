// Catalog metadata source for SQL completion.
//
// Pluggable so the completion service doesn't care where names come from. Two
// implementations:
//   - `unityCatalogProvider` — the real one, backed by the UC REST client.
//   - `fixtureCatalogProvider` — hardcoded sample data, used until the Unity
//     Catalog sidecar is wired into the desktop host (the desktop dev session
//     currently runs without UC).
//
// Swap via `setCatalogProvider`. Lookups are memoized with a short TTL by the
// completion layer, so providers can be naive.

import { fetchClient } from "@open-lakehouse/unity-catalog";

export interface CatalogColumn {
  name: string;
  type: string;
}

/** The metadata the SQL completion service needs. */
export interface CatalogProvider {
  catalogs(): Promise<string[]>;
  schemas(catalog: string): Promise<string[]>;
  tables(catalog: string, schema: string): Promise<string[]>;
  /** Columns for a fully-qualified `catalog.schema.table`. */
  columns(fullTableName: string): Promise<CatalogColumn[]>;
}

const PAGE = 1000;

/** Real provider: Unity Catalog over the existing REST client. */
export const unityCatalogProvider: CatalogProvider = {
  async catalogs() {
    const { data } = await fetchClient.GET("/catalogs", {
      params: { query: { max_results: PAGE } },
    });
    return (data?.catalogs ?? [])
      .map((c) => c.name)
      .filter((n): n is string => !!n);
  },
  async schemas(catalog) {
    const { data } = await fetchClient.GET("/schemas", {
      params: { query: { catalog_name: catalog, max_results: PAGE } },
    });
    return (data?.schemas ?? [])
      .map((s) => s.name)
      .filter((n): n is string => !!n);
  },
  async tables(catalog, schema) {
    const { data } = await fetchClient.GET("/tables", {
      params: {
        query: {
          catalog_name: catalog,
          schema_name: schema,
          max_results: PAGE,
        },
      },
    });
    return (data?.tables ?? [])
      .map((t) => t.name)
      .filter((n): n is string => !!n);
  },
  async columns(fullTableName) {
    const { data } = await fetchClient.GET("/tables/{full_name}", {
      params: { path: { full_name: fullTableName } },
    });
    return (data?.columns ?? [])
      .filter((c) => !!c.name)
      .map((c) => ({ name: c.name as string, type: c.type_text ?? "" }));
  },
};

// ── Fixture provider (temporary, until the UC sidecar is wired) ──────────────

interface FixtureTable {
  columns: CatalogColumn[];
}
// catalog → schema → table → columns
const FIXTURE: Record<string, Record<string, Record<string, FixtureTable>>> = {
  main: {
    default: {
      users: {
        columns: [
          { name: "id", type: "bigint" },
          { name: "email", type: "string" },
          { name: "created_at", type: "timestamp" },
          { name: "events", type: "bigint" },
        ],
      },
      events: {
        columns: [
          { name: "id", type: "bigint" },
          { name: "user_id", type: "bigint" },
          { name: "name", type: "string" },
          { name: "ts", type: "timestamp" },
        ],
      },
    },
    analytics: {
      daily_active: {
        columns: [
          { name: "date", type: "date" },
          { name: "count", type: "bigint" },
        ],
      },
    },
  },
  samples: {
    nyctaxi: {
      trips: {
        columns: [
          { name: "pickup_zip", type: "int" },
          { name: "dropoff_zip", type: "int" },
          { name: "fare_amount", type: "double" },
          { name: "trip_distance", type: "double" },
        ],
      },
    },
  },
};

export const fixtureCatalogProvider: CatalogProvider = {
  async catalogs() {
    return Object.keys(FIXTURE);
  },
  async schemas(catalog) {
    return Object.keys(FIXTURE[catalog] ?? {});
  },
  async tables(catalog, schema) {
    return Object.keys(FIXTURE[catalog]?.[schema] ?? {});
  },
  async columns(fullTableName) {
    const [c, s, t] = fullTableName.split(".");
    return FIXTURE[c]?.[s]?.[t]?.columns ?? [];
  },
};

// TEMPORARY: default to the fixture until the UC sidecar is wired into the
// desktop host. Switch to `unityCatalogProvider` (or call setCatalogProvider)
// once /catalogs is reachable.
let current: CatalogProvider = fixtureCatalogProvider;

export function setCatalogProvider(provider: CatalogProvider): void {
  current = provider;
}

export function getCatalogProvider(): CatalogProvider {
  return current;
}
