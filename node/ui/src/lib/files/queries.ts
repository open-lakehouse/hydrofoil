// TanStack Query hooks for the editor's file browser.
//
// Unlike the Unity Catalog layer (which goes through openapi-react-query's
// `$api`), the FileStore is a hand-rolled ConnectRPC wrapper, so we use a plain
// `useInfiniteQuery` with explicit keys. Listings are cursor-paginated via the
// Files API's `next_page_token`; refresh is invalidation-driven, not polled.

import {
  type QueryClient,
  useInfiniteQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useCallback } from "react";
import { connectFileStore, type ListPage } from "./store";

const PAGE_SIZE = 200;

/** Stable query key for a directory listing. */
export function dirKey(path: string) {
  return ["files", "list", path] as const;
}

/**
 * Infinite (cursor-paginated) listing of a directory's immediate contents.
 * Pass `enabled: false` to defer the fetch until a tree node is expanded.
 */
export function useDirectory(path: string, enabled = true) {
  return useInfiniteQuery({
    queryKey: dirKey(path),
    enabled,
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam }) =>
      connectFileStore.listDirectory(path, {
        maxResults: PAGE_SIZE,
        pageToken: pageParam,
      }),
    getNextPageParam: (last: ListPage) => last.nextPageToken,
  });
}

/** Invalidate a single directory's listing (e.g. after create/delete/rename). */
export function useInvalidateDirectory() {
  const qc = useQueryClient();
  return useCallback(
    (path: string) => qc.invalidateQueries({ queryKey: dirKey(path) }),
    [qc],
  );
}

/** Invalidate a directory listing imperatively (outside a component). */
export function invalidateDirectory(qc: QueryClient, path: string) {
  return qc.invalidateQueries({ queryKey: dirKey(path) });
}
