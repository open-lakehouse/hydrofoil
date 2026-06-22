// Centralized, run-once Monaco bootstrap.
//
// Two things have to happen before any editor mounts, and both have to happen
// exactly once for the whole app (not per editor, not per tab):
//
//  1. Point `@monaco-editor/react`'s loader at the *bundled* `monaco-editor`
//     package rather than its default CDN download. The CDN path doesn't work
//     offline or inside the Tauri desktop shell, and it desyncs from the
//     `monaco-sql-languages` build (which is pinned to this monaco version).
//
//  2. Install `self.MonacoEnvironment.getWorker` so Monaco's language services
//     run in web workers. Under Vite this is done with `?worker` imports, which
//     Vite bundles into separate worker chunks. We wire the base editor worker
//     plus the `monaco-sql-languages` SQL workers (keyed by language id, which
//     is the `label` Monaco passes to `getWorker`).
//
// Import this module for its side effects once, early — `ensureMonacoSetup()`
// is idempotent and StrictMode-safe, so calling it from a component mount is
// fine too.

import { loader } from "@monaco-editor/react";
import * as monaco from "monaco-editor";
// Base Monaco editor worker (syntax, basic language features).
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import GenericSQLWorker from "monaco-sql-languages/esm/languages/generic/generic.worker?worker";
// SQL dialect workers from monaco-sql-languages. The query engine speaks
// PostgreSQL, so `pgsql` is the primary dialect; `genericsql` is the fallback
// parser.
import PgSQLWorker from "monaco-sql-languages/esm/languages/pgsql/pgsql.worker?worker";

// Register the SQL language contributions (tokenizer + ANTLR-backed language
// features). These call monaco's `registerLanguage` / `setupLanguageFeatures`
// at import time, so they must run after the loader has the monaco instance but
// they only touch the language registry, not workers.
import "monaco-sql-languages/esm/languages/pgsql/pgsql.contribution";
import "monaco-sql-languages/esm/languages/generic/generic.contribution";
import { LanguageIdEnum, setupLanguageFeatures } from "monaco-sql-languages";
import { registerSqlCompletion } from "./catalogCompletion";

let done = false;

/**
 * Idempotently configure the Monaco loader + workers. Safe to call repeatedly
 * (e.g. from a component mount under React StrictMode); only the first call has
 * any effect.
 */
export function ensureMonacoSetup(): void {
  if (done) return;
  done = true;

  // Worker factory. Monaco passes the *language id* as `label` for language
  // workers (see monaco-sql-languages workerManager), and `editorWorkerService`
  // / no label for the base worker.
  self.MonacoEnvironment = {
    getWorker(_workerId: string, label: string) {
      switch (label) {
        case "pgsql":
          return new PgSQLWorker();
        case "genericsql":
          return new GenericSQLWorker();
        default:
          return new EditorWorker();
      }
    },
  };

  // Use the bundled monaco rather than the CDN default.
  loader.config({ monaco });

  // Disable monaco-sql-languages' worker-based completion + diagnostics: their
  // worker (editor.createWebWorker with a module id) doesn't load under Vite, so
  // they fail with "Missing requestHandler". We keep only the pgsql tokenizer
  // (highlighting) and provide completion ourselves on the main thread.
  setupLanguageFeatures(LanguageIdEnum.PG, {
    completionItems: false,
    diagnostics: false,
  });
  registerSqlCompletion(monaco);
}
