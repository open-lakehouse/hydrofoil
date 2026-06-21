import type { paths } from "@open-lakehouse/uc-client";
import createFetchClient from "openapi-fetch";
import createQueryClient from "openapi-react-query";
import { clientFetch } from "@/lib/client/registry";

// Single typed fetch client for the Unity Catalog REST API. The base URL is the
// Databricks-parallel root path that the Envoy gateway routes to the UC server
// (see environments/docker/envoy/envoy.yaml); the Vite dev proxy forwards /api
// to the gateway (see vite.config.ts). Override with VITE_API_URL if needed.
//
// The fetch is routed through `clientFetch` (lib/client/registry.ts) rather than
// the platform fetch directly. By default that IS the platform fetch, so web
// behavior is unchanged; a host (e.g. the Tauri desktop shell) can register an
// alternative to route selected requests elsewhere without the UI depending on it.
const fetchClient = createFetchClient<paths>({
  baseUrl: import.meta.env.VITE_API_URL ?? "/api/2.1/unity-catalog",
  fetch: clientFetch,
});

// `$api` wraps the fetch client with TanStack Query bindings. It auto-derives a
// query key of the form ["get", path, init] for every request. We treat that as
// the canonical key for a resource everywhere (reads, prefetch, invalidation),
// so keys never drift — see lib/uc/queries.ts for the conventions.
export const $api = createQueryClient(fetchClient);

export { fetchClient };
