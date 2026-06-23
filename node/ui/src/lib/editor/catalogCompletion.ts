// Catalog-aware SQL completion, run on the MAIN THREAD.
//
// monaco-sql-languages' own completion runs the parser in a web worker, which
// can't see our catalog. The worker DOES load fine under Vite (see monaco-setup.ts
// — we use it for diagnostics), but its completion only knows SQL grammar, not the
// live Unity Catalog. So we leave the package's worker-based *completion* disabled
// and provide our own here, sourcing real catalog names; async catalog fetches are
// natural on the main thread (a worker would have complicated them).
//
// Instead we register a plain Monaco CompletionItemProvider that parses with
// `dt-sql-parser`'s PostgreSQL grammar directly (the same parser the package
// uses, already a transitive dep) to get parse-aware context — what entity kind
// is expected and the partial dotted path — then sources real names from the
// pluggable catalog provider. Async catalog fetches are natural here (a worker
// would have complicated them).

import type { Suggestions } from "dt-sql-parser";
import { EntityContextType, PostgreSQL } from "dt-sql-parser";
import type * as Monaco from "monaco-editor";
import { getCatalogProvider } from "./catalogProvider";

// One parser instance is fine; getSuggestionAtCaretPosition is stateless per call.
const parser = new PostgreSQL();

// Catalog metadata changes infrequently; cache lookups for a minute so typing
// doesn't hammer the source. (Invalidation isn't critical for completion.)
const TTL_MS = 60_000;

interface Cached<T> {
  at: number;
  value: Promise<T>;
}
const cache = new Map<string, Cached<unknown>>();

function memo<T>(key: string, fn: () => Promise<T>): Promise<T> {
  const hit = cache.get(key) as Cached<T> | undefined;
  if (hit && performance.now() - hit.at < TTL_MS) return hit.value;
  const value = fn().catch((err) => {
    cache.delete(key);
    throw err;
  });
  cache.set(key, { at: performance.now(), value });
  return value;
}

const listCatalogs = () =>
  memo("catalogs", () => getCatalogProvider().catalogs());
const listSchemas = (catalog: string) =>
  memo(`schemas:${catalog}`, () => getCatalogProvider().schemas(catalog));
const listTables = (catalog: string, schema: string) =>
  memo(`tables:${catalog}.${schema}`, () =>
    getCatalogProvider().tables(catalog, schema),
  );
const listColumns = (fullTableName: string) =>
  memo(`columns:${fullTableName}`, () =>
    getCatalogProvider().columns(fullTableName),
  );

/**
 * The dotted-name prefix immediately before the cursor, derived from the line
 * text — NOT from the parser. dt-sql-parser returns an empty `syntax` once a
 * trailing dot is typed (`main.|`), so we can't get the path from `wordRanges`;
 * we read it off the text instead. Returns the segments BEFORE the partial word
 * being typed: `main.|` → ["main"]; `main.sa|` → ["main"]; `main.sales.|` →
 * ["main","sales"]; `foo|` (no dot) → []. Returns null if the cursor isn't in a
 * dotted identifier path at all.
 */
function dottedPrefix(lineUpToCursor: string): string[] | null {
  // Grab the trailing run of `word.word.word` (with the optional partial word).
  const m = lineUpToCursor.match(/([A-Za-z_][\w]*\.)+[\w]*$/);
  if (!m) return null;
  const segments = m[0].split(".");
  // The last segment is the partial word being completed; drop it.
  return segments.slice(0, -1).map((s) => s.replace(/[`"]/g, ""));
}

/** Fully-qualified `catalog.schema.table` names referenced in the statement. */
function tablesInStatement(suggestions: Suggestions): string[] {
  const tables = new Set<string>();
  for (const s of suggestions.syntax) {
    if (s.syntaxContextType === EntityContextType.TABLE) {
      const parts = s.wordRanges.map((w) => w.text.replace(/[.`"]/g, ""));
      if (parts.length === 3) tables.add(parts.join("."));
    }
  }
  return [...tables];
}

interface RawItem {
  label: string;
  kind: Monaco.languages.CompletionItemKind;
  detail: string;
  sort: string;
}

/**
 * Build completion items. When the cursor is inside a dotted path (`prefix` has
 * ≥1 segment), we narrow purely by prefix length — this is the reliable path,
 * since the parser yields no `syntax` after a trailing dot. Otherwise we use the
 * parser's `syntaxContextType` to decide whether catalogs, columns, etc. are
 * expected.
 */
async function buildItems(
  monaco: typeof Monaco,
  suggestions: Suggestions | null,
  prefix: string[],
): Promise<RawItem[]> {
  const Kind = monaco.languages.CompletionItemKind;
  const out: RawItem[] = [];
  const seen = new Set<string>();
  const push = (label: string, kind: number, detail: string, sort: string) => {
    const key = `${kind}:${label}`;
    if (!seen.has(key)) {
      seen.add(key);
      out.push({ label, kind, detail, sort });
    }
  };

  try {
    if (prefix.length >= 3) {
      // catalog.schema.table. → columns of that table.
      const table = prefix.slice(0, 3).join(".");
      for (const col of await listColumns(table))
        push(
          col.name,
          Kind.Field,
          col.type ? `${col.type} · ${table}` : table,
          "1",
        );
    } else if (prefix.length === 2) {
      // catalog.schema. → tables.
      for (const t of await listTables(prefix[0], prefix[1]))
        push(t, Kind.Field, `table in ${prefix[0]}.${prefix[1]}`, "1");
    } else if (prefix.length === 1) {
      // catalog. → schemas.
      for (const s of await listSchemas(prefix[0]))
        push(s, Kind.Folder, `schema in ${prefix[0]}`, "1");
    } else {
      // No dotted prefix: use the parser's expected-entity context.
      const types = new Set(
        suggestions?.syntax.map((s) => s.syntaxContextType) ?? [],
      );
      if (
        types.has(EntityContextType.TABLE) ||
        types.has(EntityContextType.VIEW) ||
        types.has(EntityContextType.CATALOG) ||
        types.has(EntityContextType.DATABASE)
      ) {
        for (const c of await listCatalogs())
          push(c, Kind.Module, "catalog", "2");
      }
      if (suggestions && types.has(EntityContextType.COLUMN)) {
        for (const table of tablesInStatement(suggestions)) {
          for (const col of await listColumns(table))
            push(
              col.name,
              Kind.Field,
              col.type ? `${col.type} · ${table}` : table,
              "1",
            );
        }
      }
    }
  } catch {
    // A failed catalog lookup must not break keyword completion.
  }

  // ANTLR-derived keywords valid at the cursor, sorted after catalog entities.
  // (Suppressed when narrowing a dotted path — keywords aren't valid mid-name.)
  if (prefix.length === 0 && suggestions) {
    for (const kw of suggestions.keywords)
      push(kw, monaco.languages.CompletionItemKind.Keyword, "keyword", "9");
  }

  return out;
}

/**
 * Register the pgsql completion provider on the main thread. Call once, after
 * the pgsql language is registered (monaco-sql-languages contribution).
 */
export function registerSqlCompletion(
  monaco: typeof Monaco,
): Monaco.IDisposable {
  return monaco.languages.registerCompletionItemProvider("pgsql", {
    // `.` continues a dotted name path; the rest is identifier characters.
    triggerCharacters: ["."],
    async provideCompletionItems(model, position) {
      const code = model.getValue();
      const suggestions = parser.getSuggestionAtCaretPosition(code, {
        lineNumber: position.lineNumber,
        column: position.column,
      });

      // The dotted prefix comes from the line text (the parser drops it after a
      // trailing dot), and decides catalog→schema→table→column narrowing.
      const lineUpToCursor = model.getValueInRange({
        startLineNumber: position.lineNumber,
        startColumn: 1,
        endLineNumber: position.lineNumber,
        endColumn: position.column,
      });
      const prefix = dottedPrefix(lineUpToCursor) ?? [];

      // With nothing parsed and no dotted prefix, there's nothing to offer.
      if (!suggestions && prefix.length === 0) return { suggestions: [] };

      // Replace the word under the cursor (so completing a partial name works).
      const word = model.getWordUntilPosition(position);
      const range: Monaco.IRange = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      };

      const items = await buildItems(monaco, suggestions, prefix);
      return {
        suggestions: items.map((it) => ({
          label: it.label,
          kind: it.kind,
          detail: it.detail,
          insertText: it.label,
          sortText: it.sort + it.label,
          range,
        })),
      };
    },
  });
}
