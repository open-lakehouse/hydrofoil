// Unity Catalog query layer.
//
// Conventions (follow these; they are what make caching/refresh/invalidation
// predictable):
//   1. NEVER hand-write query keys. `openapi-react-query` ($api) derives the key
//      ["get", path, init] from the `init` you pass. We funnel every read through
//      the shared `init` builders below so the hook, prefetch, seeding, and
//      invalidation all reference the exact same key for a resource.
//   2. Lists are cursor-paginated -> `$api.useInfiniteQuery` with
//      `pageParamName: "page_token"` (auto-injects the cursor) and
//      `getNextPageParam` reading `next_page_token`.
//   3. List responses embed full objects, so on success we seed each item's
//      DETAIL cache (`$api.queryOptions(...).queryKey`). Drilling into a row is
//      then instant with no refetch.
//   4. Refresh is invalidation-driven (see ./mutations.ts), not polled.
//
// NOTE: the OSS Unity Catalog REST API uses snake_case for both query params
// (catalog_name, schema_name, max_results, page_token) and response fields
// (next_page_token, full_name, ...). These names are taken straight from the
// generated client; keep them in sync with the spec.
import type { TableInfo } from "@open-lakehouse/uc-client";
import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { $api, fetchClient } from "@/lib/api";

const PAGE_SIZE = 100;

/**
 * Three-level fully-qualified table name. The OSS `TableInfo` schema omits the
 * `full_name` field (the server populates it at runtime, but it isn't typed), so
 * we derive it deterministically from the namespace parts.
 */
export function tableFullName(table: TableInfo): string {
  return [table.catalog_name, table.schema_name, table.name]
    .filter(Boolean)
    .join(".");
}

// ── Shared init builders (single source of truth for query keys) ────────────

const catalogsInit = {
  params: { query: { max_results: PAGE_SIZE } },
} as const;

function schemasInit(catalogName: string) {
  return {
    params: { query: { catalog_name: catalogName, max_results: PAGE_SIZE } },
  } as const;
}

function tablesInit(catalogName: string, schemaName: string) {
  return {
    params: {
      query: {
        catalog_name: catalogName,
        schema_name: schemaName,
        max_results: PAGE_SIZE,
      },
    },
  } as const;
}

// ── Detail queries (shared queryOptions: used by reads, prefetch, seeding) ──

export function catalogDetailQuery(name: string) {
  return $api.queryOptions("get", "/catalogs/{name}", {
    params: { path: { name } },
  });
}

export function tableDetailQuery(fullName: string) {
  return $api.queryOptions("get", "/tables/{full_name}", {
    params: { path: { full_name: fullName } },
  });
}

// ── List hooks (infinite/cursor pagination + list->detail seeding) ──────────

export function useCatalogs() {
  const queryClient = useQueryClient();
  const query = $api.useInfiniteQuery("get", "/catalogs", catalogsInit, {
    pageParamName: "page_token",
    initialPageParam: "",
    getNextPageParam: (lastPage) => lastPage.next_page_token || undefined,
    select: (data) => data.pages.flatMap((page) => page.catalogs ?? []),
  });

  useEffect(() => {
    for (const catalog of query.data ?? []) {
      if (catalog.name) {
        queryClient.setQueryData(
          catalogDetailQuery(catalog.name).queryKey,
          catalog,
        );
      }
    }
  }, [query.data, queryClient]);

  return query;
}

export function useSchemas(catalogName: string | undefined) {
  return $api.useInfiniteQuery(
    "get",
    "/schemas",
    schemasInit(catalogName ?? ""),
    {
      enabled: !!catalogName,
      pageParamName: "page_token",
      initialPageParam: "",
      getNextPageParam: (lastPage) => lastPage.next_page_token || undefined,
      select: (data) => data.pages.flatMap((page) => page.schemas ?? []),
    },
  );
}

export function useTables(
  catalogName: string | undefined,
  schemaName: string | undefined,
) {
  const queryClient = useQueryClient();
  const query = $api.useInfiniteQuery(
    "get",
    "/tables",
    tablesInit(catalogName ?? "", schemaName ?? ""),
    {
      enabled: !!catalogName && !!schemaName,
      pageParamName: "page_token",
      initialPageParam: "",
      getNextPageParam: (lastPage) => lastPage.next_page_token || undefined,
      select: (data) => data.pages.flatMap((page) => page.tables ?? []),
    },
  );

  useEffect(() => {
    for (const table of query.data ?? []) {
      const fullName = tableFullName(table);
      if (fullName) {
        queryClient.setQueryData(tableDetailQuery(fullName).queryKey, table);
      }
    }
  }, [query.data, queryClient]);

  return query;
}

// ── Prefetch-on-intent helpers ──────────────────────────────────────────────
//
// These mirror the hook `init` exactly, so the cache they warm is the SAME
// entry the hook later reads. Call from route loaders or row hover handlers.

export function prefetchCatalogs(queryClient: QueryClient) {
  return queryClient.ensureInfiniteQueryData({
    queryKey: ["get", "/catalogs", catalogsInit],
    queryFn: async () => {
      const { data, error } = await fetchClient.GET("/catalogs", {
        params: { query: { max_results: PAGE_SIZE } },
      });
      if (error) throw error;
      return data;
    },
    initialPageParam: "",
    getNextPageParam: (lastPage: { next_page_token?: string }) =>
      lastPage.next_page_token || undefined,
  });
}

export function prefetchSchemas(queryClient: QueryClient, catalogName: string) {
  return queryClient.ensureInfiniteQueryData({
    queryKey: ["get", "/schemas", schemasInit(catalogName)],
    queryFn: async ({ pageParam }) => {
      const { data, error } = await fetchClient.GET("/schemas", {
        params: {
          query: {
            catalog_name: catalogName,
            max_results: PAGE_SIZE,
            page_token: (pageParam as string) || undefined,
          },
        },
      });
      if (error) throw error;
      return data;
    },
    initialPageParam: "",
    getNextPageParam: (lastPage: { next_page_token?: string }) =>
      lastPage.next_page_token || undefined,
  });
}

export const ucListKeys = {
  catalogs: () => ["get", "/catalogs", catalogsInit] as const,
  schemas: (catalogName: string) =>
    ["get", "/schemas", schemasInit(catalogName)] as const,
  tables: (catalogName: string, schemaName: string) =>
    ["get", "/tables", tablesInit(catalogName, schemaName)] as const,
};
