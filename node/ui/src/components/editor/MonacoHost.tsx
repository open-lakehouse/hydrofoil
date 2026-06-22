import Editor, { type OnMount } from "@monaco-editor/react";
import type * as Monaco from "monaco-editor";
import { useCallback, useEffect, useRef } from "react";
import { getEntry, saveViewState } from "@/lib/editor/models";
import { ensureMonacoSetup } from "@/lib/editor/monaco-setup";
import { useEditorSession } from "./EditorSessionContext";

// Run the loader/worker bootstrap at module import — before any <Editor> mounts.
ensureMonacoSetup();

// A single persistent Monaco editor whose model is swapped on tab change. We do
// NOT mount an <Editor> per tab (that tears down/recreates the editor on every
// switch — slow, leaky, loses cursor/scroll). Instead one editor displays the
// active tab's model; switching saves the outgoing view state, sets the new
// model, and restores its view state.
export function MonacoHost() {
  const { activeId, attachMonaco, runActive } = useEditorSession();
  const monacoRef = useRef<typeof Monaco | null>(null);
  const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
  // The path whose model is currently set on the editor (to save its view state
  // before swapping).
  const currentPathRef = useRef<string | null>(null);
  const mountedRef = useRef(false);
  // Latest runActive in a ref so the Cmd+Enter command (registered once) always
  // calls the current one without re-registering.
  const runActiveRef = useRef(runActive);
  runActiveRef.current = runActive;

  // Swap the displayed model to `nextPath`'s. Refs-only, so it's stable.
  const applyModel = useCallback((nextPath: string | null) => {
    const editor = editorRef.current;
    if (!editor) return;

    // Save the outgoing tab's view state.
    const prev = currentPathRef.current;
    if (prev && prev !== nextPath) {
      saveViewState(prev, editor.saveViewState());
    }

    if (!nextPath) {
      editor.setModel(null);
      currentPathRef.current = null;
      return;
    }

    const entry = getEntry(nextPath);
    if (!entry || entry.model.isDisposed()) {
      // Model not ready yet (open is async); the next activeId effect re-runs.
      return;
    }
    editor.setModel(entry.model);
    if (entry.viewState) editor.restoreViewState(entry.viewState);
    currentPathRef.current = nextPath;
    editor.focus();
  }, []);

  const onMount: OnMount = (editor, monaco) => {
    editorRef.current = editor;
    monacoRef.current = monaco;
    mountedRef.current = true;
    attachMonaco(monaco, editor);
    // Cmd/Ctrl+Enter runs the active query (flush-then-run, no-op for non-SQL).
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => {
      void runActiveRef.current();
    });
    // Apply the active model now that the editor exists.
    applyModel(activeId);
  };

  // Swap the displayed model whenever the active tab changes.
  useEffect(() => {
    if (mountedRef.current) applyModel(activeId);
  }, [activeId, applyModel]);

  return (
    <div className="relative h-full">
      <Editor
        // Content is driven imperatively via setModel; these are just defaults
        // for the brief moment before the first model is applied.
        defaultLanguage="pgsql"
        theme="vs-dark"
        onMount={onMount}
        options={{
          minimap: { enabled: false },
          fontSize: 13,
          scrollBeyondLastLine: false,
          automaticLayout: true,
        }}
      />
      {activeId === null && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-sm text-muted-foreground">
          Open a file from the tree to start editing.
        </div>
      )}
    </div>
  );
}
