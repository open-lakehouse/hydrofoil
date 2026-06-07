import { useEffect, useRef, useState } from "react";
import type { ServiceSurface } from "@/lib/services";

// Embeds a service's own web UI in a full-height iframe. The src is a
// gateway-relative path (e.g. /mlflow), so the iframe loads same-origin through
// the Vite dev proxy -> Envoy, sidestepping cross-origin and CSP frame issues.
//
// Note: some services (e.g. marimo) open content in a new tab via links that
// hard-code target="_blank". We accept that behavior rather than reaching into
// the frame to rewrite it.
//
// Same-origin loading also lets a service opt into DOM-level tweaks via
// `customizeFrame` (e.g. marimo, whose hard-coded home-page sections can't be
// turned off through server config). We apply it on load and re-apply on
// mutations, since the embedded apps are SPAs that re-render without firing a
// fresh `onLoad`.
export function ServiceFrame({ service }: { service: ServiceSurface }) {
  const [loaded, setLoaded] = useState(false);
  const iframeRef = useRef<HTMLIFrameElement>(null);

  useEffect(() => {
    const iframe = iframeRef.current;
    const customize = service.customizeFrame;
    if (!iframe || !customize) {
      return;
    }

    let observer: MutationObserver | undefined;

    const apply = () => {
      // contentDocument is null/throws when cross-origin; bail quietly so a
      // service served from another origin never breaks the embed.
      let doc: Document | null = null;
      try {
        doc = iframe.contentDocument;
      } catch {
        return;
      }
      if (!doc?.body) {
        return;
      }
      const frameDoc = doc;

      customize(frameDoc);

      if (!observer) {
        observer = new MutationObserver(() => customize(frameDoc));
        observer.observe(frameDoc.body, { childList: true, subtree: true });
      }
    };

    iframe.addEventListener("load", apply);
    apply();

    return () => {
      iframe.removeEventListener("load", apply);
      observer?.disconnect();
    };
  }, [service]);

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
        ref={iframeRef}
        key={service.id}
        title={service.label}
        src={service.path}
        className="h-full w-full border-0"
        onLoad={() => setLoaded(true)}
      />
    </div>
  );
}
