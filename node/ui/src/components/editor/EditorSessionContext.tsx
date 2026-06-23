// Editor session orchestration — the hub that ties together the tab reducer, the
// Monaco model registry, and autosave, and exposes the imperative API the rest
// of the editor UI calls (openFile / activate / close / reorder).
//
// Split of concerns (deliberate, mirrors the app's data layer):
//   - reducer state (tabs, order, active id, per-tab save status) lives here in
//     React and drives the tab strip;
//   - the live Monaco model + view state + saved-version baseline live in the
//     model registry (lib/editor/models.ts), a non-React singleton;
//   - autosave timers + the version-pinned flush live in lib/editor/autosave.ts.
//
// MonacoHost registers the captured `monaco` + `editor` here on mount; opening a
// file needs `monaco` to create the model.

import type * as Monaco from "monaco-editor";
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from "react";
import { type Autosave, createAutosave } from "@/lib/editor/autosave";
import { languageOf } from "@/lib/editor/language";
import {
  disposeAll,
  disposeModel,
  ensureModel,
  getEntry,
} from "@/lib/editor/models";
import type { RunController } from "@/lib/editor/runController";
import {
  initialSessionState,
  type OpenTab,
  sessionReducer,
  type TabId,
} from "@/lib/editor/sessionReducer";
import { connectFileStore } from "@/lib/files/store";
import {
  type NotebookController,
  NotebookSessionRegistry,
} from "@/lib/notebook/sessionRegistry";
import { ResultSessionRegistry } from "@/lib/query/resultSessions";

const CONTENT_TYPE_BY_LANG: Record<string, string> = {
  sql: "text/plain",
  markdown: "text/markdown",
  notebook: "text/x-python",
  plaintext: "text/plain",
};

interface EditorSessionValue {
  tabs: OpenTab[];
  activeId: TabId | null;
  /** True once the Monaco editor has mounted (openFile needs it). */
  editorReady: boolean;
  /** Open (or focus, if already open) a file in a tab. */
  openFile: (path: string) => Promise<void>;
  activate: (id: TabId) => void;
  close: (id: TabId) => Promise<void>;
  reorder: (from: number, to: number) => void;
  /** Force-save a tab (e.g. before running its query). */
  flush: (path: string) => Promise<void>;
  /** The run controller for a tab (per-tab SQL results, survives switches). */
  runController: (path: TabId) => RunController;
  /** The notebook controller for a notebook tab (per-tab marimo session,
   *  survives switches). Null if no notebook host is registered. */
  notebookController: (path: TabId) => NotebookController | null;
  /** Flush then run the active SQL tab's current buffer text. */
  runActive: () => Promise<void>;
  /** Set by MonacoHost once the editor has mounted. */
  attachMonaco: (
    monaco: typeof Monaco,
    editor: Monaco.editor.IStandaloneCodeEditor,
  ) => void;
}

const EditorSessionContext = createContext<EditorSessionValue | undefined>(
  undefined,
);

export function EditorSessionProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(sessionReducer, initialSessionState);
  const [editorReady, setEditorReady] = useState(false);

  const monacoRef = useRef<typeof Monaco | null>(null);
  const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
  // The content-change listener disposable, per open path.
  const listenersRef = useRef<Map<string, Monaco.IDisposable>>(new Map());
  const autosaveRef = useRef<Autosave | null>(null);
  // Etags by path, read by autosave's write-if-match (kept in a ref so the
  // autosave instance is stable across renders).
  const etagsRef = useRef<Map<string, string>>(new Map());
  // The environment-scoped result-session registry owns the per-tab run
  // controllers (each tab's results survive tab switches). It is created here so
  // it shares this provider's lifetime — the gate remounts the provider on an
  // environment switch (key=env.id), which disposes the old registry and its
  // in-flight streams, scoping results to the environment.
  const sessionsRef = useRef<ResultSessionRegistry | null>(null);
  if (sessionsRef.current === null) {
    sessionsRef.current = new ResultSessionRegistry();
  }
  const sessions = sessionsRef.current;
  // The notebook-session registry, scoped to this provider's lifetime exactly
  // like the result-session registry (disposed on environment switch, which
  // remounts the provider). Owns the per-tab marimo sessions.
  const notebooksRef = useRef<NotebookSessionRegistry | null>(null);
  if (notebooksRef.current === null) {
    notebooksRef.current = new NotebookSessionRegistry();
  }
  const notebooks = notebooksRef.current;
  // Mirror of activeId for stable callbacks (runActive) that shouldn't re-create
  // on every activation.
  const activeIdRef = useRef<TabId | null>(null);
  activeIdRef.current = state.activeId;
  // Mirror of the tab list for the stable openFile callback (notebook dedupe —
  // notebooks have no Monaco model, so getEntry can't serve as the open-check).
  const tabsRef = useRef<OpenTab[]>(state.tabs);
  tabsRef.current = state.tabs;

  // Build the autosave instance once; its callbacks dispatch into the reducer.
  if (autosaveRef.current === null) {
    autosaveRef.current = createAutosave({
      onStatus: (path, saveStatus, error) =>
        dispatch({ type: "SET_STATUS", id: path, saveStatus, error }),
      onEtag: (path, etag) => {
        etagsRef.current.set(path, etag);
        dispatch({ type: "SET_ETAG", id: path, etag });
      },
      getEtag: (path) => etagsRef.current.get(path),
      contentType: (path) => CONTENT_TYPE_BY_LANG[languageOf(path)],
    });
  }

  const attachMonaco = useCallback(
    (monaco: typeof Monaco, editor: Monaco.editor.IStandaloneCodeEditor) => {
      monacoRef.current = monaco;
      editorRef.current = editor;
      setEditorReady(true);
    },
    [],
  );

  const openFile = useCallback(async (path: string) => {
    const language = languageOf(path);

    // Notebook tabs embed a marimo iframe — no Monaco model, no byte read here
    // (the host copies a working copy and serves it). Dedupe on the tab list
    // (notebooks never create a model, so getEntry can't be the open-check) and
    // let the NotebookController own the async session lifecycle.
    if (language === "notebook") {
      if (tabsRef.current.some((t) => t.id === path)) {
        dispatch({ type: "ACTIVATE_TAB", id: path });
        return;
      }
      const name = path.replace(/\/+$/, "").split("/").pop() ?? path;
      dispatch({
        type: "OPEN_TAB",
        tab: { id: path, path, name, language: "notebook" },
      });
      return;
    }

    // Already open → just activate (no refetch, no duplicate model).
    if (getEntry(path)) {
      dispatch({ type: "ACTIVATE_TAB", id: path });
      return;
    }
    const monaco = monacoRef.current;
    if (!monaco) return; // editor not mounted yet

    const { bytes, stat } = await connectFileStore.readFile(path);
    const text = new TextDecoder().decode(bytes);
    const entry = ensureModel(monaco, path, text);
    etagsRef.current.set(path, stat.etag);

    // Mark dirty on edits; the autosave instance derives clean/dirty/saving.
    const listener = entry.model.onDidChangeContent(() =>
      autosaveRef.current?.noteEdit(path),
    );
    listenersRef.current.get(path)?.dispose();
    listenersRef.current.set(path, listener);

    const name = path.replace(/\/+$/, "").split("/").pop() ?? path;
    dispatch({
      type: "OPEN_TAB",
      tab: {
        id: path,
        path,
        name,
        language: languageOf(path),
        etag: stat.etag,
      },
    });
  }, []);

  const activate = useCallback(
    (id: TabId) => dispatch({ type: "ACTIVATE_TAB", id }),
    [],
  );

  const close = useCallback(
    async (id: TabId) => {
      // Notebook tabs have no Monaco model / autosave; closing means a final
      // sync of the working copy back to the volume, then releasing the host
      // session (which discards the working copy).
      if (languageOf(id) === "notebook") {
        const ctrl = notebooks.controller(id);
        if (ctrl) await ctrl.sync().catch(() => {});
        notebooks.release(id);
        dispatch({ type: "CLOSE_TAB", id });
        return;
      }

      // Best-effort flush before discarding the buffer (dirty-confirm UI lands in
      // the autosave/dirty step; for now an unsaved close still persists).
      await autosaveRef.current?.flush(id);
      autosaveRef.current?.cancel(id);
      listenersRef.current.get(id)?.dispose();
      listenersRef.current.delete(id);
      etagsRef.current.delete(id);
      sessions.release(id);
      disposeModel(id);
      dispatch({ type: "CLOSE_TAB", id });
    },
    [sessions, notebooks],
  );

  const reorder = useCallback(
    (from: number, to: number) => dispatch({ type: "REORDER_TABS", from, to }),
    [],
  );

  const flush = useCallback(
    (path: string) => autosaveRef.current?.flush(path) ?? Promise.resolve(),
    [],
  );

  const runController = useCallback(
    (path: TabId): RunController => sessions.controller(path),
    [sessions],
  );

  const notebookController = useCallback(
    (path: TabId): NotebookController | null => notebooks.controller(path),
    [notebooks],
  );

  // Save-on-run: flush the buffer, then execute its current text. We run what's
  // in the model (the live buffer), so the flush is for persistence, not to
  // decide what executes.
  const runActive = useCallback(async () => {
    const path = activeIdRef.current;
    if (!path) return;
    const entry = getEntry(path);
    if (!entry || entry.model.isDisposed()) return;
    const sql = entry.model.getValue();
    if (!sql.trim()) return;
    await autosaveRef.current?.flush(path);
    await runController(path).run(sql);
  }, [runController]);

  // Flush dirty buffers on tab close / browser unload; tear down on unmount.
  useEffect(() => {
    const onBeforeUnload = (e: BeforeUnloadEvent) => {
      const hasUnsaved = state.tabs.some(
        (t) => t.saveStatus === "dirty" || t.saveStatus === "saving",
      );
      if (hasUnsaved) {
        void autosaveRef.current?.flushAll();
        e.preventDefault();
      }
    };
    window.addEventListener("beforeunload", onBeforeUnload);
    return () => window.removeEventListener("beforeunload", onBeforeUnload);
  }, [state.tabs]);

  // On provider unmount, flush then dispose every model + listener. The session
  // registry (and its in-flight streams) is disposed here too — on an
  // environment switch this provider remounts, tearing the old environment's
  // results down.
  const autosave = autosaveRef.current;
  useEffect(() => {
    const listeners = listenersRef.current;
    return () => {
      void autosave?.flushAll().finally(() => {
        autosave?.dispose();
        for (const d of listeners.values()) d.dispose();
        listeners.clear();
        sessions.dispose();
        notebooks.dispose();
        disposeAll();
      });
    };
  }, [autosave, sessions, notebooks]);

  const value = useMemo<EditorSessionValue>(
    () => ({
      tabs: state.tabs,
      activeId: state.activeId,
      editorReady,
      openFile,
      activate,
      close,
      reorder,
      flush,
      runController,
      notebookController,
      runActive,
      attachMonaco,
    }),
    [
      state.tabs,
      state.activeId,
      editorReady,
      openFile,
      activate,
      close,
      reorder,
      flush,
      runController,
      notebookController,
      runActive,
      attachMonaco,
    ],
  );

  return (
    <EditorSessionContext.Provider value={value}>
      {children}
    </EditorSessionContext.Provider>
  );
}

export function useEditorSession(): EditorSessionValue {
  const ctx = useContext(EditorSessionContext);
  if (!ctx) {
    throw new Error(
      "useEditorSession must be used within an EditorSessionProvider",
    );
  }
  return ctx;
}
