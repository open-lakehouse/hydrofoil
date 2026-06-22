// Results panel for the active SQL tab.
//
// Subscribes to the active tab's RunController (per-tab results survive tab
// switches) and renders the shared, virtualized Arrow DataGrid. The Run button
// flushes the buffer then executes its text (save-on-run); Cancel aborts the
// in-flight stream.
import { Loader2, Play, X } from "lucide-react";
import { useSyncExternalStore } from "react";
import { DataGrid } from "@/components/data-grid/data-grid";
import { Button } from "@/components/ui/button";
import type { TabId } from "@/lib/editor/sessionReducer";
import { useEditorSession } from "./EditorSessionContext";

export function ResultsPane({ activePath }: { activePath: TabId }) {
  const { runController, runActive } = useEditorSession();
  const controller = runController(activePath);

  // Re-render whenever this tab's controller updates (chunk appended, done…).
  const snapshot = useSyncExternalStore(controller.subscribe, controller.get);
  const { store, version, running, error } = snapshot;

  const rowCount = store?.rowCount ?? 0;
  const hasColumns = !!store && store.columnCount > 0;

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-2 border-b px-3 py-1.5">
        <Button
          size="sm"
          onClick={() => void runActive()}
          disabled={running}
          className="h-7"
        >
          {running ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Play className="h-3.5 w-3.5" />
          )}
          Run
        </Button>
        {running && (
          <Button
            size="sm"
            variant="outline"
            onClick={() => controller.cancel()}
            className="h-7"
          >
            <X className="h-3.5 w-3.5" />
            Cancel
          </Button>
        )}
        {store && (
          <span className="text-xs text-muted-foreground">
            {rowCount} row{rowCount === 1 ? "" : "s"}
            {running ? "…" : ""}
          </span>
        )}
        <span className="ml-auto text-[11px] text-muted-foreground">
          ⌘⏎ to run
        </span>
      </div>

      {error && (
        <div className="border-b border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}

      <div className="min-h-0 flex-1">
        {hasColumns ? (
          <DataGrid store={store} version={version} running={running} />
        ) : (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            {running ? "Running…" : "Run the query to see results."}
          </div>
        )}
      </div>
    </div>
  );
}
