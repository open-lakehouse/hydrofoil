// Shared expansion store for the file tree, keyed by directory path.
//
// Mirrors components/catalog/ExpansionContext.tsx: one Set of open paths held in
// context (not per-node useState) so expansion survives remounts, persists to
// sessionStorage, and can be driven programmatically (expand-to-path on deep
// link). Node ids ARE the absolute directory paths — they're already unique.
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useMemo,
  useState,
} from "react";

interface FileExpansionValue {
  isOpen: (path: string) => boolean;
  toggle: (path: string) => void;
  expand: (paths: string[]) => void;
}

const FileExpansionContext = createContext<FileExpansionValue | undefined>(
  undefined,
);

const STORAGE_KEY = "editor.tree.expanded";

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

function persist(paths: Set<string>) {
  try {
    window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify([...paths]));
  } catch {
    // storage may be unavailable (private mode etc.)
  }
}

export function FileExpansionProvider({ children }: { children: ReactNode }) {
  const [expanded, setExpanded] = useState<Set<string>>(loadInitial);

  const toggle = useCallback((path: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      persist(next);
      return next;
    });
  }, []);

  const expand = useCallback((paths: string[]) => {
    setExpanded((prev) => {
      if (paths.every((p) => prev.has(p))) return prev;
      const next = new Set(prev);
      for (const p of paths) next.add(p);
      persist(next);
      return next;
    });
  }, []);

  const value = useMemo<FileExpansionValue>(
    () => ({ isOpen: (p) => expanded.has(p), toggle, expand }),
    [expanded, toggle, expand],
  );

  return (
    <FileExpansionContext.Provider value={value}>
      {children}
    </FileExpansionContext.Provider>
  );
}

export function useFileExpansion(): FileExpansionValue {
  const ctx = useContext(FileExpansionContext);
  if (!ctx) {
    throw new Error(
      "useFileExpansion must be used within a FileExpansionProvider",
    );
  }
  return ctx;
}
