// Desktop notebook host: opens a `.py` file as a marimo notebook.
//
// The UI's editor calls the `NotebookHost` seam (registered in main.ts) when a
// notebook tab opens; this module maps those calls to the Tauri commands in
// `src-tauri/src/notebook.rs`, which copy a working copy, ensure the shared
// marimo sidecar, and return an `olservice://notebook/...` URL the editor
// embeds in an iframe. The UI itself stays Tauri-free.
import { invoke } from "@tauri-apps/api/core";
import type { NotebookHost, NotebookSession } from "@/lib/notebook/registry";

export const tauriNotebookHost: NotebookHost = {
  async openNotebook(volumePath: string): Promise<NotebookSession> {
    // Rust returns { url, session_id }; map to the seam's camelCase shape.
    const res = await invoke<{ url: string; session_id: string }>(
      "open_notebook",
      { path: volumePath },
    );
    return { url: res.url, sessionId: res.session_id };
  },
  async syncNotebook(sessionId: string): Promise<void> {
    await invoke("sync_notebook", { sessionId });
  },
  async closeNotebook(sessionId: string): Promise<void> {
    await invoke("close_notebook", { sessionId });
  },
};
