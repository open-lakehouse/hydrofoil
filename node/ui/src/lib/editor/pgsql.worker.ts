// monaco-0.55 web-worker entry for monaco-sql-languages' pgsql parser.
//
// This mirrors monaco's OWN language workers (e.g.
// monaco-editor/esm/vs/language/css/css.worker.js) exactly: import `initialize`
// from monaco's `vs/common/initialize` and hand it a factory. `initialize`
// performs the 0.55 two-message handshake (ignore the first message, bootstrap
// `start()` on the second, which carries createData) and registers the returned
// worker as the foreign module backing `doValidation`.
//
// We do NOT use the package's own `pgsql.worker.js`: it targets monaco's removed
// `editor.worker.js#initialize` export and additionally its WorkerManager never
// passes a `worker`/`createWorker` to 0.55's `editor.createWebWorker`, so it falls
// back to the main thread with no request handler ("Missing requestHandler:
// doValidation"). We construct the worker the 0.55 way in monaco-setup.ts and use
// this blessed bootstrap here. See docs/monaco-sql-worker-tauri.md.
import { initialize } from "monaco-editor/esm/vs/common/initialize";
import type { ICreateData } from "monaco-sql-languages/esm/baseSQLWorker";
import { PgSQLWorker } from "monaco-sql-languages/esm/languages/pgsql/PgSQLWorker";

self.onmessage = () => {
  initialize(
    (ctx, createData: ICreateData) => new PgSQLWorker(ctx, createData),
  );
};
