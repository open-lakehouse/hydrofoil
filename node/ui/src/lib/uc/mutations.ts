// Unity Catalog invalidation map.
//
// The three-level namespace is a hierarchy: catalog -> schema -> table. When a
// resource changes, the lists that contain it must be refetched. Mutations
// (create/update/delete) are not implemented yet for the read-first navigation
// surface, but the invalidation strategy is defined here so wiring them up later
// is mechanical: a mutation's `onSettled` just calls the matching helper.
//
// We match by PREDICATE on the canonical ["get", path, init] key rather than by
// exact key, so a single call invalidates every page / param variant of a list
// (e.g. all `maxResults`/`pageToken` combinations).

import type { QueryClient, QueryKey } from "@tanstack/react-query";
import { useQueryClient } from "@tanstack/react-query";
import { $api } from "@/lib/api";
import { catalogDetailQuery, tableDetailQuery } from "./queries";

interface ListInit {
  params?: { query?: { catalog_name?: string; schema_name?: string } };
}

function listQuery(key: QueryKey, path: string): ListInit | undefined {
  if (!Array.isArray(key) || key[0] !== "get" || key[1] !== path) {
    return undefined;
  }
  return key[2] as ListInit | undefined;
}

/** Invalidate the top-level catalog list. */
export function invalidateCatalogs(queryClient: QueryClient) {
  return queryClient.invalidateQueries({
    predicate: (q) => !!listQuery(q.queryKey, "/catalogs"),
  });
}

/** Invalidate every schema list for a catalog (all pages/params). */
export function invalidateSchemas(
  queryClient: QueryClient,
  catalogName: string,
) {
  return queryClient.invalidateQueries({
    predicate: (q) =>
      listQuery(q.queryKey, "/schemas")?.params?.query?.catalog_name ===
      catalogName,
  });
}

/** Invalidate a schema-scoped list (tables/volumes/functions/models). */
function invalidateSchemaList(
  queryClient: QueryClient,
  path: string,
  catalogName: string,
  schemaName: string,
) {
  return queryClient.invalidateQueries({
    predicate: (q) => {
      const query = listQuery(q.queryKey, path)?.params?.query;
      return (
        query?.catalog_name === catalogName && query?.schema_name === schemaName
      );
    },
  });
}

/** Invalidate every table list for a schema (all pages/params). */
export function invalidateTables(
  queryClient: QueryClient,
  catalogName: string,
  schemaName: string,
) {
  return invalidateSchemaList(queryClient, "/tables", catalogName, schemaName);
}

/** Invalidate every volume list for a schema. */
export function invalidateVolumes(
  queryClient: QueryClient,
  catalogName: string,
  schemaName: string,
) {
  return invalidateSchemaList(queryClient, "/volumes", catalogName, schemaName);
}

/** Invalidate every function list for a schema. */
export function invalidateFunctions(
  queryClient: QueryClient,
  catalogName: string,
  schemaName: string,
) {
  return invalidateSchemaList(
    queryClient,
    "/functions",
    catalogName,
    schemaName,
  );
}

/** Invalidate every registered-model list for a schema. */
export function invalidateModels(
  queryClient: QueryClient,
  catalogName: string,
  schemaName: string,
) {
  return invalidateSchemaList(queryClient, "/models", catalogName, schemaName);
}

/** Drop a single catalog's detail cache. */
export function removeCatalogDetail(queryClient: QueryClient, name: string) {
  queryClient.removeQueries({ queryKey: catalogDetailQuery(name).queryKey });
}

/** Drop a single table's detail cache. */
export function removeTableDetail(queryClient: QueryClient, fullName: string) {
  queryClient.removeQueries({ queryKey: tableDetailQuery(fullName).queryKey });
}

/**
 * Remove all cached descendants of a catalog (its schema lists and any table
 * lists under it). Use after deleting a catalog so stale child data can't be
 * served from cache.
 */
export function removeCatalogDescendants(
  queryClient: QueryClient,
  catalogName: string,
) {
  queryClient.removeQueries({
    predicate: (q) => {
      for (const path of [
        "/schemas",
        "/tables",
        "/volumes",
        "/functions",
        "/models",
      ]) {
        if (
          listQuery(q.queryKey, path)?.params?.query?.catalog_name ===
          catalogName
        ) {
          return true;
        }
      }
      return false;
    },
  });
}

// ── Create mutations ────────────────────────────────────────────────────────
//
// Each hook wraps `$api.useMutation("post", path)` and, on success, invalidates
// the parent list so the tree refetches in place. Mutations never hand-write
// keys; they reuse the same predicate-based invalidators as everything else.

/** Create a catalog, then refresh the catalog list. */
export function useCreateCatalog() {
  const queryClient = useQueryClient();
  return $api.useMutation("post", "/catalogs", {
    onSuccess: () => invalidateCatalogs(queryClient),
  });
}

/** Create a schema, then refresh its parent catalog's schema list. */
export function useCreateSchema() {
  const queryClient = useQueryClient();
  return $api.useMutation("post", "/schemas", {
    onSuccess: (data) => {
      if (data?.catalog_name) invalidateSchemas(queryClient, data.catalog_name);
    },
  });
}

/** Create a volume, then refresh its parent schema's volume list. */
export function useCreateVolume() {
  const queryClient = useQueryClient();
  return $api.useMutation("post", "/volumes", {
    onSuccess: (data) => {
      if (data?.catalog_name && data?.schema_name) {
        invalidateVolumes(queryClient, data.catalog_name, data.schema_name);
      }
    },
  });
}

/** Create a registered model, then refresh its parent schema's model list. */
export function useCreateRegisteredModel() {
  const queryClient = useQueryClient();
  return $api.useMutation("post", "/models", {
    onSuccess: (data) => {
      if (data?.catalog_name && data?.schema_name) {
        invalidateModels(queryClient, data.catalog_name, data.schema_name);
      }
    },
  });
}
