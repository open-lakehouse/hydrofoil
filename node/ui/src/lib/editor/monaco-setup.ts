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
// Base Monaco editor worker (the editorWorkerService — diff/links/etc.). Wired
// via Vite's `?worker` import, which works for this worker.
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";

// Register the SQL language contributions (tokenizer + ANTLR-backed language
// features). These call monaco's `registerLanguage` / `setupLanguageFeatures`
// at import time, but only touch the language registry, not workers.
import "monaco-sql-languages/esm/languages/pgsql/pgsql.contribution";
import "monaco-sql-languages/esm/languages/generic/generic.contribution";
import { LanguageIdEnum, setupLanguageFeatures } from "monaco-sql-languages";
import { registerSqlCompletion } from "./catalogCompletion";

// On the monaco-sql-languages worker: it creates its worker via
// `editor.createWebWorker({moduleId})`, whose ESM module loading is NOT reached
// through `MonacoEnvironment.getWorker` and doesn't resolve under Vite — the
// pgsql worker is never requested (verified) and its features fail with
// "Missing requestHandler: doValidation". `vite-plugin-monaco-editor` is the
// supported bridge, but it breaks the Vite 8 + monaco 0.55 build
// (resolveMonacoPath does a require.resolve without `.js`, which Node ESM
// rejects). So we DISABLE its worker-based completion + diagnostics and run
// completion ourselves on the main thread (see registerSqlCompletion). The
// pgsql *tokenizer* (highlighting) needs no worker and still works.

let done = false;

/**
 * Idempotently configure the Monaco loader + workers + SQL features. Safe to
 * call repeatedly (e.g. from a component mount under React StrictMode); only the
 * first call has any effect.
 */
export function ensureMonacoSetup(): void {
  if (done) return;
  done = true;

  // The base editor worker is required for core editor services; it loads fine
  // via `?worker`. (The SQL dialect workers are intentionally not wired — see
  // the note above.)
  self.MonacoEnvironment = {
    getWorker() {
      return new EditorWorker();
    },
  };

  // Use the bundled monaco rather than the CDN default.
  loader.config({ monaco });

  // Disable monaco-sql-languages' worker-based features (their worker can't load
  // under Vite); keep only the pgsql tokenizer. Completion is ours, main-thread.
  setupLanguageFeatures(LanguageIdEnum.PG, {
    completionItems: false,
    diagnostics: false,
  });
  registerSqlCompletion(monaco);
}
