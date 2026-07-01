// Read-only config viewer for the environment manager's Config tab. A teaching /
// inspection surface: it lists the config artifacts the host generates for the
// environment's selected capabilities (the generated Docker Compose, the static
// service fragments, the Envoy + collector configs) and renders the selected one
// in a read-only Monaco editor.
//
// Artifacts are produced on demand by the host (generated compose is built from
// the capabilities, not read off disk), so this works before the environment has
// ever been started.

import Editor from "@monaco-editor/react";
import { cn } from "@open-lakehouse/ui-kit";
import { FileCode, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import {
  type ConfigArtifact,
  getEnvironmentHost,
} from "@/lib/client/environments";
import { ensureMonacoSetup } from "@/lib/editor/monaco-setup";

// Bootstrap Monaco's loader/workers before the first <Editor> mounts (same hook
// the SQL editor uses).
ensureMonacoSetup();

export function ConfigViewer({ environmentId }: { environmentId: string }) {
  const host = getEnvironmentHost();
  const [artifacts, setArtifacts] = useState<ConfigArtifact[] | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setArtifacts(null);
    setError(null);
    host
      .configArtifacts(environmentId)
      .then((list) => {
        if (cancelled) return;
        setArtifacts(list);
        setSelectedId(list[0]?.id ?? null);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [host, environmentId]);

  if (error) {
    return (
      <p className="p-4 text-sm text-destructive">
        Couldn't load config artifacts: {error}
      </p>
    );
  }
  if (!artifacts) {
    return (
      <div className="flex items-center gap-2 p-4 text-sm text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" /> Generating config…
      </div>
    );
  }
  if (artifacts.length === 0) {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        No services are configured for this environment yet. Enable a capability
        on the Overview tab to see its generated configuration here.
      </p>
    );
  }

  const selected = artifacts.find((a) => a.id === selectedId) ?? artifacts[0];

  return (
    <div className="flex h-full min-h-0">
      {/* Artifact picker */}
      <div className="w-56 shrink-0 overflow-auto border-r p-2">
        {artifacts.map((a) => (
          <button
            key={a.id}
            type="button"
            onClick={() => setSelectedId(a.id)}
            className={cn(
              "flex w-full items-start gap-2 rounded-md px-2 py-1.5 text-left text-sm",
              a.id === selected.id
                ? "bg-accent text-accent-foreground"
                : "text-muted-foreground hover:bg-accent/50 hover:text-foreground",
            )}
          >
            <FileCode className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span className="min-w-0 break-words">{a.label}</span>
          </button>
        ))}
      </div>

      {/* Read-only Monaco for the selected artifact */}
      <div className="flex min-w-0 flex-1 flex-col">
        <p className="border-b px-4 py-2 text-xs text-muted-foreground">
          {selected.description}
        </p>
        <div className="min-h-0 flex-1">
          <Editor
            // Key by id so switching artifacts swaps the model cleanly.
            key={selected.id}
            language={selected.language}
            value={selected.content}
            theme="vs-dark"
            options={{
              readOnly: true,
              domReadOnly: true,
              minimap: { enabled: false },
              fontSize: 13,
              scrollBeyondLastLine: false,
              automaticLayout: true,
              wordWrap: "on",
            }}
          />
        </div>
      </div>
    </div>
  );
}
