// Tab persistence for the editor workspace.
//
// Two stores, mirroring the catalog explorer's split (selection.ts +
// ExpansionContext):
//   - The open-tab set + order persists to sessionStorage — too noisy for the
//     URL, but worth surviving a reload.
//   - The ACTIVE tab persists to the `?path=` URL search param, so it's
//     deep-linkable and survives back/forward.
//
// On first editor-ready, we restore: re-open the persisted paths (which refetch
// content + build models), then activate the URL's `?path=` (falling back to the
// last persisted tab). Restore runs once.

import { useNavigate, useSearch } from "@tanstack/react-router";
import { useEffect, useRef } from "react";
import { useEditorSession } from "./EditorSessionContext";

const FROM = "/editor";
const STORAGE_KEY = "editor.openTabs";

function loadOpenPaths(): string[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.sessionStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw) as string[];
  } catch {
    // ignore malformed storage
  }
  return [];
}

function saveOpenPaths(paths: string[]) {
  try {
    window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify(paths));
  } catch {
    // storage may be unavailable
  }
}

/**
 * Wire tab persistence. Call once from the editor route component (it needs the
 * router context). Returns nothing; it's a side-effect hook.
 */
export function useTabPersistence() {
  const { tabs, activeId, editorReady, openFile, activate } =
    useEditorSession();
  const navigate = useNavigate({ from: FROM });
  const urlPath = useSearch({ from: FROM, select: (s) => s.path });
  const restoredRef = useRef(false);

  // Restore once, after Monaco is ready (openFile needs the editor mounted).
  useEffect(() => {
    if (restoredRef.current || !editorReady) return;
    restoredRef.current = true;

    const persisted = loadOpenPaths();
    const target = urlPath ?? persisted[persisted.length - 1];
    if (persisted.length === 0 && !target) return;

    void (async () => {
      // Open in persisted order; openFile is a no-op if already open.
      for (const path of persisted) {
        try {
          await openFile(path);
        } catch {
          // A persisted file may have been deleted/renamed — skip it.
        }
      }
      // Make sure the URL's active tab is open + focused.
      if (target) {
        try {
          await openFile(target);
          activate(target);
        } catch {
          // ignore a stale ?path=
        }
      }
    })();
  }, [editorReady, urlPath, openFile, activate]);

  // Persist the open-tab set/order whenever it changes (after restore).
  useEffect(() => {
    if (!restoredRef.current) return;
    saveOpenPaths(tabs.map((t) => t.id));
  }, [tabs]);

  // Keep `?path=` in sync with the active tab (after restore).
  useEffect(() => {
    if (!restoredRef.current) return;
    if (activeId === urlPath) return;
    navigate({
      search: (prev) => ({ ...prev, path: activeId ?? undefined }),
      replace: true,
    });
  }, [activeId, urlPath, navigate]);
}
