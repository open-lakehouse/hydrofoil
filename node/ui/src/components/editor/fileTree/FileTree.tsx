// Volume file browser for the editor's left pane.
//
// Reuses the catalog tree's primitives (TreeRow / ListStates) and an expansion
// store keyed by path. Directories lazily fetch their contents via `useDirectory`
// (cursor-paginated) only once expanded; files invoke `onOpenFile` on click.
import {
  FileCode,
  FileText,
  FileType,
  Folder,
  NotebookPen,
  Plus,
} from "lucide-react";
import { useMemo } from "react";

import { ListStates, TreeRow } from "@/components/catalog/TreeRow";
import { Button } from "@/components/ui/button";
import { type EditorLanguage, languageOf } from "@/lib/editor/language";
import { useDirectory } from "@/lib/files/queries";
import type { FileEntry } from "@/lib/files/store";
import { useFileExpansion } from "./fileExpansion";

/** Icon for a file, chosen by its classified language. */
function FileIcon({ language }: { language: EditorLanguage }) {
  const cls = "h-4 w-4 text-muted-foreground";
  if (language === "sql") return <FileCode className={cls} />;
  if (language === "markdown") return <FileType className={cls} />;
  if (language === "notebook") return <NotebookPen className={cls} />;
  return <FileText className={cls} />;
}

export function FileTree({
  root,
  activePath,
  onOpenFile,
  onNewNotebook,
}: {
  /** Absolute directory path the tree is rooted at. */
  root: string;
  /** Path of the active tab, highlighted in the tree. */
  activePath?: string;
  onOpenFile: (path: string) => void;
  /** Open the "new notebook" dialog (header "New" button). Omitted = no button. */
  onNewNotebook?: () => void;
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center justify-between border-b px-3 py-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        <span className="flex items-center gap-2">
          <Folder className="h-4 w-4" />
          Files
        </span>
        {onNewNotebook && (
          <Button
            variant="ghost"
            size="sm"
            className="h-6 px-1.5 text-xs"
            onClick={onNewNotebook}
          >
            <Plus className="h-3.5 w-3.5" />
            New
          </Button>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-1">
        <DirectoryChildren
          path={root}
          depth={0}
          activePath={activePath}
          onOpenFile={onOpenFile}
        />
      </div>
    </div>
  );
}

/** The contents of one directory: subdirectories first, then files. */
function DirectoryChildren({
  path,
  depth,
  activePath,
  onOpenFile,
}: {
  path: string;
  depth: number;
  activePath?: string;
  onOpenFile: (path: string) => void;
}) {
  const query = useDirectory(path);
  const entries = useMemo(
    () => sortEntries(query.data?.pages.flatMap((p) => p.entries) ?? []),
    [query.data],
  );

  return (
    <ListStates
      depth={depth}
      isLoading={query.isLoading}
      error={query.error}
      isEmpty={entries.length === 0}
      hasNextPage={query.hasNextPage}
      isFetchingNextPage={query.isFetchingNextPage}
      onLoadMore={() => query.fetchNextPage()}
    >
      {entries.map((entry) =>
        entry.isDirectory ? (
          <DirectoryNode
            key={entry.path}
            entry={entry}
            depth={depth}
            activePath={activePath}
            onOpenFile={onOpenFile}
          />
        ) : (
          <TreeRow
            key={entry.path}
            depth={depth}
            icon={<FileIcon language={languageOf(entry.path)} />}
            label={entry.name}
            selected={entry.path === activePath}
            onSelect={() => onOpenFile(entry.path)}
          />
        ),
      )}
    </ListStates>
  );
}

function DirectoryNode({
  entry,
  depth,
  activePath,
  onOpenFile,
}: {
  entry: FileEntry;
  depth: number;
  activePath?: string;
  onOpenFile: (path: string) => void;
}) {
  const { isOpen, toggle } = useFileExpansion();
  const open = isOpen(entry.path);

  return (
    <div>
      <TreeRow
        depth={depth}
        icon={<Folder className="h-4 w-4 text-muted-foreground" />}
        label={entry.name}
        expandable
        open={open}
        onToggle={() => toggle(entry.path)}
      />
      {open && (
        <DirectoryChildren
          path={entry.path}
          depth={depth + 1}
          activePath={activePath}
          onOpenFile={onOpenFile}
        />
      )}
    </div>
  );
}

/** Directories first, then files; each group alphabetical (case-insensitive). */
function sortEntries(entries: FileEntry[]): FileEntry[] {
  return [...entries].sort((a, b) => {
    if (a.isDirectory !== b.isDirectory) return a.isDirectory ? -1 : 1;
    return a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
  });
}
