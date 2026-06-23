// A dependency-free `fetch` that serves the Unity Catalog REST API from the
// curated fixtures — registered via `registerFetch` so the UI's UC query layer
// (lib/api.ts -> lib/uc/queries.ts) renders real-looking data in Storybook with
// no backend. This mirrors how node/desktop's tauriFetch stands in for the
// network, but returns fixtures instead of proxying to a sidecar.
//
// It matches the exact paths/params the query layer issues (see
// lib/uc/queries.ts): cursor-paginated list endpoints returning an envelope
// keyed by the entity plural + `next_page_token`, and `/{name}` / `/{full_name}`
// detail endpoints. Everything is a single page (no real pagination needed for a
// fixture set this size).

import {
  catalogs,
  credentials,
  externalLocations,
  functions,
  models,
  schemas,
  tables,
  volumes,
} from "@/lib/fixtures";

const BASE = "/api/2.1/unity-catalog";

function json(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function notFound(detail: string): Response {
  return new Response(JSON.stringify({ message: detail }), {
    status: 404,
    headers: { "content-type": "application/json" },
  });
}

// A list endpoint: filter by the catalog/schema query params the UI sends, wrap
// in the entity-keyed envelope. `key` is the response field (e.g. "catalogs").
function listResponse<T extends Record<string, unknown>>(
  items: T[],
  key: string,
  params: URLSearchParams,
): Response {
  const catalog = params.get("catalog_name");
  const schema = params.get("schema_name");
  const filtered = items.filter(
    (it) =>
      (!catalog || it.catalog_name === catalog) &&
      (!schema || it.schema_name === schema),
  );
  return json({ [key]: filtered, next_page_token: undefined });
}

// Resolve a detail lookup by full name (catalog.schema.object) or simple name.
function detailResponse<T extends Record<string, unknown>>(
  items: T[],
  needle: string,
  label: string,
): Response {
  const match = items.find(
    (it) =>
      it.full_name === needle ||
      it.name === needle ||
      [it.catalog_name, it.schema_name, it.name].filter(Boolean).join(".") ===
        needle,
  );
  return match ? json(match) : notFound(`${label} not found: ${needle}`);
}

interface Route {
  /** Plural list key in the response envelope. */
  key: string;
  items: Record<string, unknown>[];
  label: string;
}

const ROUTES: Record<string, Route> = {
  catalogs: { key: "catalogs", items: catalogs, label: "catalog" },
  schemas: { key: "schemas", items: schemas, label: "schema" },
  tables: { key: "tables", items: tables, label: "table" },
  volumes: { key: "volumes", items: volumes, label: "volume" },
  functions: { key: "functions", items: functions, label: "function" },
  models: { key: "registered_models", items: models, label: "model" },
  credentials: { key: "credentials", items: credentials, label: "credential" },
  "external-locations": {
    key: "external_locations",
    items: externalLocations,
    label: "external location",
  },
};

/** A fixture-backed fetch for the UC REST API. Falls through to the platform
 *  fetch for anything outside the UC base path (e.g. asset requests). */
export const fixtureFetch: typeof globalThis.fetch = async (input, init) => {
  const url = new URL(
    typeof input === "string"
      ? input
      : input instanceof URL
        ? input.href
        : input.url,
    "http://storybook.local",
  );

  if (!url.pathname.startsWith(BASE)) {
    return globalThis.fetch(input as RequestInfo, init);
  }

  // Path after the UC base: e.g. "catalogs", "tables/main.sales.orders".
  const rest = url.pathname.slice(BASE.length).replace(/^\//, "");
  const [resource, ...tail] = rest.split("/");
  const route = ROUTES[resource];

  if (!route) return notFound(`no fixture route for /${rest}`);

  // Detail lookup: the remainder is the (URL-encoded) name / full name.
  if (tail.length > 0) {
    const needle = decodeURIComponent(tail.join("/"));
    return detailResponse(route.items, needle, route.label);
  }

  return listResponse(route.items, route.key, url.searchParams);
};
