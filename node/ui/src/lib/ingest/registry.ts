// Pluggable file-ingest seam — lets a host environment (the Tauri desktop shell)
// supply a local-file picker + drag-drop path resolution WITHOUT the UI taking on
// a Tauri dependency. The import page depends only on the types and functions
// here; the desktop host registers its implementation before the UI bootstraps
// (mirrors `lib/client/registry.ts` and `lib/query/runner.ts`).
//
// Phase 1 is desktop-only: previewing + ingesting a file goes through the host's
// in-process engine, which reads the file by its local path. A web build (which
// has no filesystem path) would register a picker that returns bytes and parse
// in-browser instead — out of scope for now.

/** A picked local file: its absolute path (the host engine reads it by path). */
export interface PickedFile {
  /** Absolute filesystem path on the machine running the host engine. */
  path: string;
  /** Base name for display (e.g. `events.parquet`). */
  name: string;
}

/**
 * Opens a native file picker and resolves to the chosen file, or `null` when the
 * user cancels. Host-provided; the default (web) throws because there is no local
 * filesystem path to hand the engine.
 */
export type FilePicker = () => Promise<PickedFile | null>;

const webPicker: FilePicker = () => {
  throw new Error(
    "file ingest is only available in the desktop app (no local filesystem on web)",
  );
};

let current: FilePicker = webPicker;

/** Install a host file picker. Hosts call this once, before the UI bootstraps. */
export function registerFilePicker(picker: FilePicker): void {
  current = picker;
}

/** Whether a host has registered a real file picker (gates the import UI). */
export function ingestSupported(): boolean {
  return current !== webPicker;
}

/** The picker currently in effect (late-binding, like the other registries). */
export const pickFile: FilePicker = () => current();
