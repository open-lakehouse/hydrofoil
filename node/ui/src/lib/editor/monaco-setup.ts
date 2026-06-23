// Centralized, run-once Monaco bootstrap.
//
// Three things have to happen before any editor mounts, exactly once for the whole
// app (not per editor, not per tab):
//
//  1. Point `@monaco-editor/react`'s loader at the *bundled* `monaco-editor`
//     package rather than its default CDN download. The CDN path doesn't work
//     offline or inside the Tauri desktop shell, and it desyncs from the
//     `monaco-sql-languages` build (which is pinned to this monaco version).
//
//  2. Install `self.MonacoEnvironment.getWorker` for the base editor worker
//     (diff/links/etc.), wired via Vite's `?worker` import.
//
//  3. Wire pgsql DIAGNOSTICS (validation) to a web worker ourselves — see
//     `setupPgSqlDiagnostics`. We cannot use `monaco-sql-languages`'
//     `setupLanguageFeatures({ diagnostics: true })`: its WorkerManager calls the
//     low-level `editor.createWebWorker({ moduleId, label, createData })`, but
//     monaco 0.55's low-level API ignores those and reads only `worker` — a
//     pre-built Worker. With no `worker`, monaco logs "Could not create web
//     worker(s). Falling back to ... main thread", runs the worker code on the
//     main thread WITHOUT a request handler, and every call rejects with
//     "Missing requestHandler: doValidation". (The MonacoEnvironment.getWorker
//     path is never reached for "pgsql" — confirmed at runtime.) monaco's OWN
//     language workers (css/json/ts) avoid this by passing `worker`/`createWorker`
//     via the `vs/common/workers.js` wrapper; we replicate that wrapper here.
//     See docs/monaco-sql-worker-tauri.md.
//
// Import this module for its side effects once, early — `ensureMonacoSetup()`
// is idempotent and StrictMode-safe, so calling it from a component mount is fine.

import { loader } from "@monaco-editor/react";
import * as monaco from "monaco-editor";
// Base Monaco editor worker (the editorWorkerService — diff/links/etc.). Wired
// via Vite's `?worker` import, which bundles it as a separate worker chunk.
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
// Our monaco-0.55-compatible pgsql worker entry (the package's own is broken under
// 0.55 — see pgsql.worker.ts). `?worker` bundles it into its own chunk.
import PgSqlWorker from "./pgsql.worker?worker";

// Register the SQL language contributions (tokenizer + ANTLR-backed language
// features). These call monaco's `registerLanguage` / `setupLanguageFeatures`
// at import time, but only touch the language registry, not workers.
import "monaco-sql-languages/esm/languages/pgsql/pgsql.contribution";
import "monaco-sql-languages/esm/languages/generic/generic.contribution";
import { LanguageIdEnum, setupLanguageFeatures } from "monaco-sql-languages";
// The package's DiagnosticsAdapter wires model-change → worker.doValidation →
// editor markers. We reuse it but feed it a worker WE created the 0.55 way.
import type { BaseSQLWorker } from "monaco-sql-languages/esm/baseSQLWorker";
import { DiagnosticsAdapter } from "monaco-sql-languages/esm/languageFeatures";
import {
  LanguageServiceDefaultsImpl,
  modeConfigurationDefault,
} from "monaco-sql-languages/esm/monaco.contribution";
import { registerSqlCompletion } from "./catalogCompletion";

let done = false;

/**
 * Idempotently configure the Monaco loader + workers + SQL features. Safe to
 * call repeatedly (e.g. from a component mount under React StrictMode); only the
 * first call has any effect.
 */
export function ensureMonacoSetup(): void {
  if (done) return;
  done = true;

  // Base editor worker only. (The pgsql worker is created directly in
  // setupPgSqlDiagnostics — monaco-sql-languages never routes through here.)
  self.MonacoEnvironment = {
    getWorker() {
      return new EditorWorker();
    },
  };

  // Use the bundled monaco rather than the CDN default.
  loader.config({ monaco });

  // Disable the package's worker features entirely: its diagnostics WorkerManager
  // can't build a worker under monaco 0.55 (see header), and completion is ours
  // (catalog-aware, main-thread). We keep only the pgsql tokenizer + diagnostics
  // we wire by hand below.
  setupLanguageFeatures(LanguageIdEnum.PG, {
    completionItems: false,
    diagnostics: false,
  });
  setupPgSqlDiagnostics();
  registerSqlCompletion(monaco);
}

/**
 * Wire pgsql validation to a web worker using monaco 0.55's real worker protocol,
 * then drive the package's DiagnosticsAdapter with it.
 *
 * This mirrors `monaco-editor/esm/vs/common/workers.js` `createWebWorker`: build a
 * Worker, perform the two-message handshake (`postMessage("ignore")` then the
 * createData), and hand the worker to the low-level `editor.createWebWorker({ worker })`.
 */
function setupPgSqlDiagnostics(): void {
  const languageId = LanguageIdEnum.PG; // "pgsql"

  const worker = new PgSqlWorker();
  // 0.55 handshake: first message is ignored by the worker, the second carries
  // createData and bootstraps it (see pgsql.worker.ts).
  worker.postMessage("ignore");
  worker.postMessage({ languageId });

  // Foreign-module proxy: methods like doValidation are forwarded to the worker.
  // Typed as the package's BaseSQLWorker so it slots into WorkerAccessor.
  const client = monaco.editor.createWebWorker<BaseSQLWorker>({ worker });

  // The DiagnosticsAdapter expects a `(...uris) => Promise<workerProxy>` getter
  // and syncs the open models onto the worker before each call.
  const getWorker = (...resources: monaco.Uri[]) =>
    client.withSyncedResources(resources.filter(Boolean));

  // Defaults the adapter reads: the `onDidChange` event it subscribes to and
  // `preprocessCode` (optional). Diagnostics on; everything else default.
  const defaults = new LanguageServiceDefaultsImpl(
    languageId,
    { ...modeConfigurationDefault, diagnostics: true },
    undefined,
  );

  // Side-effect: registers model listeners that validate on change.
  // eslint-disable-next-line no-new
  new DiagnosticsAdapter(languageId, getWorker, defaults);
}
