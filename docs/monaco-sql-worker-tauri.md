# Monaco SQL language worker in Tauri — findings & fix plan

Status: **applied & verified** (2026-06-23). Fix landed in `monaco-setup.ts`,
`catalogCompletion.ts`, and a new `pgsql.worker.ts` + `monaco-worker.d.ts`; both `node/ui`
and `node/desktop` type-check and build, both emit the `pgsql.worker-*.js` chunk (~2.2 MB —
real parser), and **worker-backed SQL diagnostics confirmed working in the Tauri webview**.

## TL;DR (the actual root cause, confirmed at runtime)

The blocker is **not** Vite or Tauri. It is a version incompatibility:
**monaco-sql-languages@1.1.0 cannot create a web worker under monaco-editor 0.55.**

The package's `WorkerManager` calls the *low-level*
`monaco.editor.createWebWorker({ moduleId, label, createData })`. In monaco 0.55 that
low-level API reads only a pre-built `worker` (`Worker | Promise<Worker>`) and ignores
`moduleId`/`label`/`createData`. With no `worker`, monaco logs *"Could not create web
worker(s). Falling back to loading web worker code in main thread"*, runs the worker code on
the main thread **with no foreign module registered**, and every call rejects with
*"Missing requestHandler or method: doValidation"*. The `MonacoEnvironment.getWorker` path is
**never reached for "pgsql"** (confirmed: `getWorker` is only ever called with
`"editorWorkerService"`).

monaco's OWN language workers (css/json/ts) avoid this via the `vs/common/workers.js` wrapper,
which builds the `Worker`, runs the two-message handshake, and hands the worker to
`editor.createWebWorker({ worker })`. **Fix: we replicate that wrapper ourselves** and drive
the package's `DiagnosticsAdapter` with the worker we built — see "Root cause & fix" below.
We do NOT rely on `setupLanguageFeatures({ diagnostics: true })` (that's the broken path).

> **Earlier dead ends (kept for the record).** Two intermediate diagnoses were wrong:
> (1) "getWorker ignores the `label`" — true that it did, but irrelevant: pgsql never routes
> through getWorker at all. (2) "the package's `pgsql.worker.js` uses a removed `initialize`
> API" — also not the issue; `initialize` exists in `vs/common/initialize.js` and works (our
> worker entry now uses it, exactly like monaco's css.worker.js). The real failure is on the
> HOST side: the missing `worker` in `editor.createWebWorker`. The sections below the line
> describe the original (superseded) label hypothesis and are retained only as history.

---

### (Superseded) original hypothesis

The reason the `monaco-sql-languages` pgsql web worker "wouldn't load under Vite" is a
**worker-wiring bug in our own code**, not a Vite or Tauri limitation. Our
`MonacoEnvironment.getWorker` ignores the `label` argument and always returns the base
editor worker, so when Monaco asks for the `pgsql` worker it gets a worker that has no
`doValidation` handler — exactly the *"Missing requestHandler: doValidation"* error we saw.
The fix is to switch on `label` and return the package's pgsql worker (imported via Vite's
`?worker`). No `vite-plugin-monaco-editor`, no Tauri config change.

## How we got here (the current workaround)

- `node/ui/src/lib/editor/monaco-setup.ts` sets
  `setupLanguageFeatures(LanguageIdEnum.PG, { completionItems: false, diagnostics: false })`
  — i.e. all worker-backed pgsql features are disabled. Only the **tokenizer**
  (syntax highlighting, no worker) is kept.
- SQL completion was reimplemented **on the main thread** in
  `node/ui/src/lib/editor/catalogCompletion.ts` using `dt-sql-parser` + the pluggable
  catalog provider (`catalogProvider.ts`). This gives catalog-aware completion but **no
  diagnostics/validation at all**.
- The code comments (`monaco-setup.ts:35-44`, `catalogCompletion.ts:1-15`) state the cause
  as: *"`editor.createWebWorker({moduleId})` … is NOT reached through
  `MonacoEnvironment.getWorker` and doesn't resolve under Vite."*

## Root cause — the diagnosis in those comments is wrong

Reading the installed sources:

1. **`monaco-sql-languages@1.1.0`** — `node/node_modules/monaco-sql-languages/esm/workerManager.js:36-44`:
   ```js
   this._worker = editor.createWebWorker({
       moduleId: this._defaults.languageId,   // "pgsql"
       label:    this._defaults.languageId,   // "pgsql"
       createData: { languageId: this._defaults.languageId }
   });
   ```

2. **Monaco 0.55** — `node_modules/monaco-editor/esm/vs/base/browser/webWorkerFactory.js:29-49`:
   ```js
   function getWorker(descriptor, id) {
       const label = descriptor.label || 'anonymous' + id;
       const monacoEnvironment = getMonacoEnvironment();
       if (monacoEnvironment) {
           if (typeof monacoEnvironment.getWorker === 'function') {
               return monacoEnvironment.getWorker('workerMain.js', label); // ← routes HERE
           }
           if (typeof monacoEnvironment.getWorkerUrl === 'function') { ... }
       }
       ...
   }
   ```
   `editor.createWebWorker` **does** route through `MonacoEnvironment.getWorker`. The
   `moduleId` arg Monaco passes is hardcoded `'workerMain.js'`; the **`label`** (here
   `"pgsql"`) is the disambiguator.

3. **Our wiring** — `node/ui/src/lib/editor/monaco-setup.ts:60-64`:
   ```ts
   self.MonacoEnvironment = {
     getWorker() {           // ← ignores (moduleId, label)
       return new EditorWorker();   // ← always the BASE editor worker
     },
   };
   ```
   So Monaco's request for the `pgsql` worker returned a **base editor worker**, which has
   no `doValidation` request handler → *"Missing requestHandler: doValidation"*. The pgsql
   worker file was never imported, so Vite never bundled a chunk for it — which is why it
   "was never requested." That is a consequence of the bug, not Vite failing to resolve it.

The base `EditorWorker` is wired the same way and **works today**, proving module workers run
fine in both the Vite browser dev server and the Tauri webview. The pgsql worker is the
identical mechanism with a different `label`.

## Second root cause — monaco-sql-languages 1.1.0 vs monaco-editor 0.55 worker API

After fixing the label switch, the worker loaded but every call still threw
`Missing requestHandler or method: doValidation` (from
`monaco-editor/esm/vs/editor/common/services/editorWebWorker.js:284`, the `$fmr` foreign-method
dispatcher — it rejects because `this._foreignModule` was never set).

The package's worker entry (`monaco-sql-languages/esm/languages/pgsql/pgsql.worker.js`) targets
an **old** monaco worker API:
```js
import * as EditorWorker from 'monaco-editor/esm/vs/editor/editor.worker.js';
self.onmessage = () => {
    EditorWorker.initialize((ctx, createData) => new PgSQLWorker(ctx, createData));
};
```
But in monaco 0.55, `editor.worker.js` re-exports `initialize` from `common/initialize.js`,
which is **an empty file** — so `EditorWorker.initialize` is `undefined`. The foreign module is
never registered → `doValidation` is missing. (This is a real version incompatibility; the
package hasn't kept up with monaco's worker-bootstrap rewrite.)

monaco 0.55's actual protocol is `start(createClient)` from
`monaco-editor/esm/vs/editor/editor.worker.start.js`: it runs the host handshake and sets the
object `createClient(ctx)` returns as the foreign module. `ctx` is `{ host, getMirrorModels }`
(no `createData`). `BaseSQLWorker` only needs `ctx.getMirrorModels()` (its `getTextDocument()`),
so the bridge is tiny.

**Fix:** ship our own worker entry, `node/ui/src/lib/editor/pgsql.worker.ts`:
```ts
import { start } from "monaco-editor/esm/vs/editor/editor.worker.start";
import { PgSQLWorker } from "monaco-sql-languages/esm/languages/pgsql/PgSQLWorker";
start((ctx) => new PgSQLWorker(ctx, { languageId: "pgsql" }));
```
and point the `?worker` import in `monaco-setup.ts` at `./pgsql.worker` instead of the package's
broken `pgsql.worker.js`. `editor.worker.start` ships no `.d.ts`, so a one-line ambient module
declaration lives in `node/ui/src/lib/editor/monaco-worker.d.ts`.

Sanity check that we got the *real* worker: the emitted `pgsql.worker-*.js` chunk is ~2.2 MB —
it bundles `dt-sql-parser`'s PostgreSQL grammar. (The old stub would have been tiny.)

## Tauri angle (the new aspect we were asked to check)

No Tauri constraint blocks this:

- `node/desktop/src-tauri/tauri.conf.json` sets `"app.security.csp": null` → **no CSP**, so
  no `worker-src`/`script-src`/`child-src` restriction on workers. Tauri **v2**.
- In dev, `build.devUrl = http://localhost:3003`; the webview loads the Vite dev server (a
  normal HTTP origin) and Vite serves `?worker` chunks on demand. The base editor worker
  already loads here — the pgsql worker will too.
- The desktop app reuses the UI wholesale (`node/desktop/src/main.ts` imports `@/main`;
  `@` → `../ui/src`), so the same `monaco-setup.ts` runs in Tauri.
- If a CSP is ever introduced, it must include `worker-src 'self' blob:` and
  `script-src 'self'` for module workers — but today there is none to worry about.

## The fix

### 1. Wire per-`label` workers — `node/ui/src/lib/editor/monaco-setup.ts`

```ts
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
// NB: our own entry, not the package's pgsql.worker.js — see "Second root cause".
import PgSqlWorker from "./pgsql.worker?worker";

self.MonacoEnvironment = {
  getWorker(_moduleId, label) {
    if (label === "pgsql") return new PgSqlWorker();
    return new EditorWorker();
  },
};
```

`?worker` makes Vite bundle the entry as a worker chunk exactly like the base worker.
`vite-plugin-monaco-editor` (and its Vite-8-incompatible `require.resolve` bug noted in the
old comment) is **not needed**. We point at `./pgsql.worker` (our `start()`-based entry)
rather than the package's `pgsql.worker.js`, which is broken under monaco 0.55 (see the
"Second root cause" section).

### 2. Re-enable diagnostics — `node/ui/src/lib/editor/monaco-setup.ts`

Change `setupLanguageFeatures(LanguageIdEnum.PG, …)` to `{ diagnostics: true }`. **Keep
`completionItems: false`** and keep our `registerSqlCompletion` main-thread provider — it
does real Unity Catalog metadata lookups the bundled worker can't, and async catalog fetches
are natural on the main thread. Net effect: we *gain* SQL validation (currently absent)
without losing catalog-aware completion.

(Alternative, not recommended: `completionItems: true` to also get the package's keyword
completion, but then reconcile against `registerSqlCompletion` to avoid duplicates and we
lose catalog awareness.)

Update the now-incorrect comment blocks in `monaco-setup.ts:35-44` and
`catalogCompletion.ts:1-15` to reflect the corrected diagnosis.

### 3. Worker chunks in the packaged desktop build — resolved

We initially observed `node/desktop/dist/assets/` with **no worker chunks**. After wiring the
`?worker` import (step 1), `npx vite build` in `node/desktop` emits the full set —
`editor.worker-*.js`, `pgsql.worker-*.js`, etc. — identical to `node/ui`'s build. The earlier
empty-worker observation was a stale/partial build, not a code-split problem; no
`vite.config.ts` change was needed.

## Files

- `node/ui/src/lib/editor/monaco-setup.ts` — getWorker `label` switch; `diagnostics: true`; fix comments. **(primary)**
- `node/ui/src/lib/editor/catalogCompletion.ts` — keep main-thread completion; fix stale comment.
- `node/desktop/vite.config.ts`, `node/desktop/src/main.ts`, `node/ui/src/routes/editor.lazy.tsx` — packaged-build worker emission (step 3).
- Reference only (installed deps, do not edit): `monaco-sql-languages/esm/workerManager.js`, `…/esm/languages/pgsql/pgsql.worker.js`, `monaco-editor/esm/vs/base/browser/webWorkerFactory.js`.

## Verification

1. **Browser dev (`node/ui`, port 3002):** open the SQL editor.
   - Network tab: confirm a `pgsql.worker` chunk is requested (it never was before).
   - Type bad SQL (`SELEC * FORM t`) → red squiggle diagnostics appear. No
     "Missing requestHandler: doValidation" in console.
   - Confirm `main.` → schema completion still works (main-thread provider not regressed).
2. **Tauri dev (`node/desktop`, `npm run tauri:dev`):** repeat the diagnostics + completion
   checks inside the desktop webview (devUrl `:3003`). This is the "does Tauri allow it"
   proof — expect identical behavior to the browser.
3. **Packaged build:** `npm run build` in `node/desktop`, then
   `ls node/desktop/dist/assets | grep worker` → confirm base + pgsql worker chunks emit.
   Optionally `tauri build` and smoke-test diagnostics in the packaged app (workers over the
   Tauri asset protocol).
4. **Types:** `tsc --noEmit` in `node/ui` and `node/desktop`.
