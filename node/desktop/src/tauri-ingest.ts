// Desktop file picker for the import flow.
//
// The UI's import page reads a local file through the host engine *by path* (the
// in-process DataFusion reader parses it). The UI itself stays Tauri-free; this
// module supplies the `FilePicker` seam (registered in main.ts) using Tauri's
// native open dialog, returning the chosen absolute path.
import { open } from "@tauri-apps/plugin-dialog";
import type { FilePicker } from "@/lib/ingest/registry";

export const tauriFilePicker: FilePicker = async () => {
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Parquet", extensions: ["parquet"] }],
  });
  // `open` returns the path string (or null on cancel); with multiple:false it is
  // never an array.
  if (typeof selected !== "string") return null;
  const name = selected.split(/[/\\]/).pop() ?? selected;
  return { path: selected, name };
};
