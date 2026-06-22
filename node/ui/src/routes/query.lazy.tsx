import { createLazyRoute } from "@tanstack/react-router";
import { Loader2, Play, X } from "lucide-react";
import { useState } from "react";
import { DataGrid } from "@/components/data-grid/data-grid";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { useRunQuery } from "@/lib/query/useRunQuery";

export const Route = createLazyRoute("/query")({
  component: QueryPage,
});

function QueryPage() {
  const [sql, setSql] = useState("SELECT 1 AS x");
  const { store, version, running, error, run, cancel } = useRunQuery();

  function submit() {
    if (sql.trim()) void run(sql);
  }

  const rowCount = store?.rowCount ?? 0;

  return (
    <div className="flex h-full flex-col gap-4 p-6">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">SQL</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Run a query through Hydrofoil. Results stream in as they're produced.
        </p>
      </div>

      <Textarea
        value={sql}
        onChange={(e) => setSql(e.target.value)}
        // Cmd/Ctrl+Enter submits, matching common SQL editors.
        onKeyDown={(e) => {
          if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
            e.preventDefault();
            submit();
          }
        }}
        rows={6}
        spellCheck={false}
        className="font-mono text-sm"
        placeholder="SELECT * FROM catalog.schema.table LIMIT 100"
      />

      <div className="flex items-center gap-2">
        <Button onClick={submit} disabled={running || !sql.trim()}>
          {running ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Play className="h-4 w-4" />
          )}
          Run
        </Button>
        {running && (
          <Button variant="outline" onClick={cancel}>
            <X className="h-4 w-4" />
            Cancel
          </Button>
        )}
        {store && (
          <span className="text-sm text-muted-foreground">
            {rowCount} row{rowCount === 1 ? "" : "s"}
            {running ? "…" : ""}
          </span>
        )}
      </div>

      {error && (
        <div className="rounded border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive">
          {error}
        </div>
      )}

      {store && store.columnCount > 0 && (
        <DataGrid store={store} version={version} running={running} />
      )}
    </div>
  );
}
