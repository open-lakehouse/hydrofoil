import { keepPreviousData, QueryClient } from "@tanstack/react-query";

// Central QueryClient for the whole app.
//
// These defaults are tuned for the Unity Catalog navigation surface, whose
// metadata changes infrequently (contrast: the workflows control room polls
// every 30s because it watches live plan execution). Here we lean on the cache
// and refresh on explicit action/invalidation rather than on a timer:
//   - staleTime 60s     -> serve cached metadata, avoid refetch storms while browsing.
//   - gcTime 5m         -> keep unmounted catalog/schema/table data around for back-nav.
//   - no refetchInterval -> refresh is invalidation-driven (see lib/uc/mutations.ts).
//   - refetchOnWindowFocus off -> tab switches don't trigger refetch waves.
//   - placeholderData keepPreviousData -> paginating/filtering keeps the old list
//     visible instead of flashing to empty.
// Per-query overrides are still allowed where a specific view needs fresher data.
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 60_000,
        gcTime: 5 * 60_000,
        retry: 2,
        retryDelay: (attempt) => Math.min(1000 * 2 ** attempt, 30_000),
        refetchOnWindowFocus: false,
        refetchOnReconnect: true,
        placeholderData: keepPreviousData,
      },
    },
  });
}
