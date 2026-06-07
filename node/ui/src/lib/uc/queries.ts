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
import type {
  FunctionInfo,
  RegisteredModelInfo,
  VolumeInfo,
} from "@open-lakehouse/uc-client";
import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { $api, fetchClient } from "@/lib/api";

const PAGE_SIZE = 100;

/**
 * Three-level fully-qualified name (`catalog.schema.object`). Several OSS list
 * payloads omit `full_name` from the typed schema (the server populates it at
 * runtime), so we derive it deterministically from the namespace parts. This is
 * the single source of truth for both detail-cache keys and display.
 */
export function objectFullName(parts: {
  catalog_name?: string;
  schema_name?: string;
  name?: string;
}): string {
  return [parts.catalog_name, parts.schema_name, parts.name]
    .filter(Boolean)
    .join(".");
}

/** Back-compat alias; prefer `objectFullName`. */
export const tableFullName = objectFullName;

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

function volumesInit(catalogName: string, schemaName: string) {
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

function functionsInit(catalogName: string, schemaName: string) {
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

function modelsInit(catalogName: string, schemaName: string) {
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

export function volumeDetailQuery(fullName: string) {
  return $api.queryOptions("get", "/volumes/{name}", {
    params: { path: { name: fullName } },
  });
}

export function functionDetailQuery(fullName: string) {
  return $api.queryOptions("get", "/functions/{name}", {
    params: { path: { name: fullName } },
  });
}

export function modelDetailQuery(fullName: string) {
  return $api.queryOptions("get", "/models/{full_name}", {
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
      const fullName = objectFullName(table);
      if (fullName) {
        queryClient.setQueryData(tableDetailQuery(fullName).queryKey, table);
      }
    }
  }, [query.data, queryClient]);

  return query;
}

export function useVolumes(
  catalogName: string | undefined,
  schemaName: string | undefined,
) {
  const queryClient = useQueryClient();
  const query = $api.useInfiniteQuery(
    "get",
    "/volumes",
    volumesInit(catalogName ?? "", schemaName ?? ""),
    {
      enabled: !!catalogName && !!schemaName,
      pageParamName: "page_token",
      initialPageParam: "",
      getNextPageParam: (lastPage) => lastPage.next_page_token || undefined,
      select: (data) => data.pages.flatMap((page) => page.volumes ?? []),
    },
  );

  useEffect(() => {
    for (const volume of (query.data as VolumeInfo[] | undefined) ?? []) {
      const fullName = volume.full_name || objectFullName(volume);
      if (fullName) {
        queryClient.setQueryData(volumeDetailQuery(fullName).queryKey, volume);
      }
    }
  }, [query.data, queryClient]);

  return query;
}

export function useFunctions(
  catalogName: string | undefined,
  schemaName: string | undefined,
) {
  const queryClient = useQueryClient();
  const query = $api.useInfiniteQuery(
    "get",
    "/functions",
    functionsInit(catalogName ?? "", schemaName ?? ""),
    {
      enabled: !!catalogName && !!schemaName,
      pageParamName: "page_token",
      initialPageParam: "",
      getNextPageParam: (lastPage) => lastPage.next_page_token || undefined,
      select: (data) => data.pages.flatMap((page) => page.functions ?? []),
    },
  );

  useEffect(() => {
    for (const fn of (query.data as FunctionInfo[] | undefined) ?? []) {
      const fullName = fn.full_name || objectFullName(fn);
      if (fullName) {
        queryClient.setQueryData(functionDetailQuery(fullName).queryKey, fn);
      }
    }
  }, [query.data, queryClient]);

  return query;
}

export function useModels(
  catalogName: string | undefined,
  schemaName: string | undefined,
) {
  const queryClient = useQueryClient();
  const query = $api.useInfiniteQuery(
    "get",
    "/models",
    modelsInit(catalogName ?? "", schemaName ?? ""),
    {
      enabled: !!catalogName && !!schemaName,
      pageParamName: "page_token",
      initialPageParam: "",
      getNextPageParam: (lastPage) => lastPage.next_page_token || undefined,
      select: (data) =>
        data.pages.flatMap((page) => page.registered_models ?? []),
    },
  );

  useEffect(() => {
    for (const model of (query.data as RegisteredModelInfo[] | undefined) ??
      []) {
      const fullName = model.full_name || objectFullName(model);
      if (fullName) {
        queryClient.setQueryData(modelDetailQuery(fullName).queryKey, model);
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
  volumes: (catalogName: string, schemaName: string) =>
    ["get", "/volumes", volumesInit(catalogName, schemaName)] as const,
  functions: (catalogName: string, schemaName: string) =>
    ["get", "/functions", functionsInit(catalogName, schemaName)] as const,
  models: (catalogName: string, schemaName: string) =>
    ["get", "/models", modelsInit(catalogName, schemaName)] as const,
};
