import Editor, { type OnMount } from "@monaco-editor/react";
import type { editor } from "monaco-editor";
import { useRef } from "react";
import { ensureMonacoSetup } from "@/lib/editor/monaco-setup";

// Run the loader/worker bootstrap at module import — before any <Editor> mounts.
ensureMonacoSetup();

// SPIKE: a minimal single-model host to validate Monaco + monaco-sql-languages
// workers under Vite (dev + build). The full host (imperative model swapping,
// per-tab view state) is built on top of this in a later step.
export function MonacoHost() {
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);

  const onMount: OnMount = (ed) => {
    editorRef.current = ed;
    ed.focus();
  };

  return (
    <Editor
      // `pgsql` is registered by monaco-sql-languages' pgsql.contribution.
      defaultLanguage="pgsql"
      defaultValue={"SELECT *\nFROM main.default.\nWHERE 1 = 1;"}
      theme="vs-dark"
      onMount={onMount}
      options={{
        minimap: { enabled: false },
        fontSize: 13,
        scrollBeyondLastLine: false,
        automaticLayout: true,
      }}
    />
  );
}
