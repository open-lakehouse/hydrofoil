import { useState } from "react";
import type { ServiceSurface } from "@/lib/services";

// Embeds a service's own web UI in a full-height iframe. The src is a
// gateway-relative path (e.g. /mlflow), so the iframe loads same-origin through
// the Vite dev proxy -> Envoy, sidestepping cross-origin and CSP frame issues.
export function ServiceFrame({ service }: { service: ServiceSurface }) {
  const [loaded, setLoaded] = useState(false);

  return (
    <div className="relative h-[calc(100vh-3rem)] w-full">
      {!loaded && (
        <div className="absolute inset-0 flex items-center justify-center bg-background">
          <div className="flex flex-col items-center gap-2 text-muted-foreground">
            <div className="h-6 w-6 animate-spin rounded-full border-2 border-muted border-t-primary" />
            <span className="text-sm">Loading {service.label}…</span>
          </div>
        </div>
      )}
      <iframe
        key={service.id}
        title={service.label}
        src={service.path}
        className="h-full w-full border-0"
        onLoad={() => setLoaded(true)}
      />
    </div>
  );
}
