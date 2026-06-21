// Shared tree-expansion store.
//
// Expansion lives in one place (keyed by stable node ids) instead of per-node
// `useState`, so it (a) survives navigation/remounts, (b) is persisted to
// sessionStorage, and (c) can be driven programmatically — e.g. expand-to-path
// when arriving via a deep link. Node ids are built with the helpers below.
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useMemo,
  useState,
} from "react";

import type { ObjectKind } from "./types";

export const nodeId = {
  catalog: (catalog: string) => `catalog:${catalog}`,
  schema: (catalog: string, schema: string) => `schema:${catalog}.${schema}`,
  group: (catalog: string, schema: string, kind: ObjectKind) =>
    `group:${catalog}.${schema}:${kind}`,
};

interface ExpansionValue {
  isOpen: (id: string) => boolean;
  toggle: (id: string) => void;
  expand: (ids: string[]) => void;
}

const ExpansionContext = createContext<ExpansionValue | undefined>(undefined);

const STORAGE_KEY = "catalog.expanded";

function loadInitial(): Set<string> {
  if (typeof window === "undefined") return new Set();
  try {
    const raw = window.sessionStorage.getItem(STORAGE_KEY);
    if (raw) return new Set(JSON.parse(raw) as string[]);
  } catch {
    // ignore malformed storage
  }
  return new Set();
}

function persist(ids: Set<string>) {
  try {
    window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify([...ids]));
  } catch {
    // storage may be unavailable (private mode etc.)
  }
}

export function ExpansionProvider({ children }: { children: ReactNode }) {
  const [expanded, setExpanded] = useState<Set<string>>(loadInitial);

  const toggle = useCallback((id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      persist(next);
      return next;
    });
  }, []);

  const expand = useCallback((ids: string[]) => {
    setExpanded((prev) => {
      if (ids.every((id) => prev.has(id))) return prev;
      const next = new Set(prev);
      for (const id of ids) next.add(id);
      persist(next);
      return next;
    });
  }, []);

  const value = useMemo<ExpansionValue>(
    () => ({ isOpen: (id) => expanded.has(id), toggle, expand }),
    [expanded, toggle, expand],
  );

  return (
    <ExpansionContext.Provider value={value}>
      {children}
    </ExpansionContext.Provider>
  );
}

export function useExpansion(): ExpansionValue {
  const ctx = useContext(ExpansionContext);
  if (!ctx) {
    throw new Error("useExpansion must be used within an ExpansionProvider");
  }
  return ctx;
}
