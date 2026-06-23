// The app-level Telemetry detail pane: an embedded Jaeger UI for the shared,
// cross-environment trace collector. Jaeger is served under /jaeger (its
// QUERY_BASE_PATH) behind the Envoy gateway, so it embeds same-origin via the
// same proxy path as the other service UIs.
//
// The collector is shared and lazily started by the first observability-enabled
// environment, so it may not be up yet. We poll its status and either embed the
// UI or show a hint explaining how to start it — the entry stays discoverable
// either way.

import { Activity, Loader2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { getEnvironmentHost } from "@/lib/client/environments";

// Jaeger's UI base path (matches QUERY_BASE_PATH in services/desktop/jaeger.yaml
// and the Envoy /jaeger route). Trailing slash to skip the redirect, as with the
// other embedded service UIs.
const JAEGER_PATH = "/jaeger/";

export function TelemetryView() {
  const host = getEnvironmentHost();
  const [running, setRunning] = useState<boolean | null>(null);
  const [loaded, setLoaded] = useState(false);
  const pollRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const up = await host.telemetryStatus();
        if (!cancelled) setRunning(up);
      } catch {
        if (!cancelled) setRunning(false);
      }
      if (!cancelled) pollRef.current = setTimeout(tick, 4000);
    };
    tick();
    return () => {
      cancelled = true;
      if (pollRef.current) clearTimeout(pollRef.current);
    };
  }, [host]);

  if (running === null) {
    return (
      <div className="flex h-[calc(100vh-3rem)] items-center justify-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" /> Checking telemetry…
      </div>
    );
  }

  if (!running) {
    return (
      <div className="flex h-[calc(100vh-3rem)] flex-col items-center justify-center gap-3 p-8 text-center">
        <Activity className="h-8 w-8 text-muted-foreground" />
        <div className="space-y-1">
          <p className="text-sm font-medium">
            The trace collector isn't running
          </p>
          <p className="max-w-sm text-sm text-muted-foreground">
            Telemetry is shared across environments. Enable the{" "}
            <span className="font-medium">Observability</span> capability on an
            environment and start it — that brings the collector up, and traces
            from every environment land here.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="relative h-[calc(100vh-3rem)] w-full">
      {!loaded && (
        <div className="absolute inset-0 flex items-center justify-center bg-background">
          <div className="flex flex-col items-center gap-2 text-muted-foreground">
            <Loader2 className="h-6 w-6 animate-spin" />
            <span className="text-sm">Loading Jaeger…</span>
          </div>
        </div>
      )}
      <iframe
        title="Jaeger"
        src={JAEGER_PATH}
        className="h-full w-full border-0"
        onLoad={() => setLoaded(true)}
      />
    </div>
  );
}
