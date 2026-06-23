// Path → editor language classification.
//
// One source of truth for "what kind of file is this", used by the file tree
// (icon), the tab (label/behavior), and MonacoHost (the model's language id).
// `sql` maps to the `pgsql` Monaco language registered by monaco-sql-languages
// (the query engine speaks PostgreSQL); markdown gets a preview; `notebook`
// (a `.py` file on a notebook-capable host) opens as an embedded marimo notebook
// instead of a Monaco buffer; everything else is plain text.

import { notebookSupported } from "@/lib/notebook/registry";

export type EditorLanguage = "sql" | "markdown" | "notebook" | "plaintext";

/** Monaco language id for a given EditorLanguage. Total over the union even
 *  though `notebook` tabs never create a Monaco model (they embed an iframe);
 *  the entry keeps lookups in models.ts type-safe. */
export const MONACO_LANGUAGE_ID: Record<EditorLanguage, string> = {
  sql: "pgsql",
  markdown: "markdown",
  notebook: "python",
  plaintext: "plaintext",
};

const BY_EXTENSION: Record<string, EditorLanguage> = {
  sql: "sql",
  md: "markdown",
  markdown: "markdown",
};

/** The lowercased extension (without the dot), or "" if none. */
export function extensionOf(path: string): string {
  const name = path.replace(/\/+$/, "").split("/").pop() ?? "";
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

/** Classify a path into an EditorLanguage (plaintext for unknown types).
 *
 *  `.py` resolves to `notebook` only when the host registered notebook support
 *  (the desktop shell); on web it falls through to plaintext so Python files
 *  still open in the Monaco text editor. */
export function languageOf(path: string): EditorLanguage {
  const ext = extensionOf(path);
  if (ext === "py" && notebookSupported()) {
    return "notebook";
  }
  return BY_EXTENSION[ext] ?? "plaintext";
}
