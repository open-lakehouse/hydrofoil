// The notebook editor tab: a full-pane <iframe> embedding the marimo notebook
// for a `.py` file. Modeled on ServiceFrame (loading overlay, same-origin
// `customizeFrame` tweaks), but driven by a per-tab NotebookController that owns
// the async host call to prepare the session (copy a working copy + ensure the
// marimo sidecar) and yields the iframe URL.
//
// The iframe `src` is a Tauri custom-protocol URL (olservice://notebook/...)
// that proxies to the dynamic sidecar port, so it loads same-origin to that
// protocol — DOM tweaks work, and there is no cross-origin/CSP framing issue.

import { Loader2, NotebookPen } from "lucide-react";
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { customizeMarimoFrame } from "@/lib/notebook/customizeFrame";
import type { NotebookController } from "@/lib/notebook/sessionRegistry";

const FRAME_HEIGHT = "h-[calc(100vh-3rem)]";

export function NotebookPane({
  controller,
}: {
  controller: NotebookController;
}) {
  const snapshot = useSyncExternalStore(controller.subscribe, controller.get);
  const [loaded, setLoaded] = useState(false);
  const iframeRef = useRef<HTMLIFrameElement>(null);

  // Kick off session preparation when this pane first mounts for the tab.
  useEffect(() => {
    controller.start();
  }, [controller]);

  // Apply idempotent marimo DOM tweaks once the frame is same-origin (it is:
  // the olservice:// protocol is same-origin to the embedding document). Re-run
  // on mutations since marimo is an SPA that re-renders without a fresh onLoad.
  useEffect(() => {
    const iframe = iframeRef.current;
    if (!iframe || !snapshot.url) return;

    let observer: MutationObserver | undefined;
    const apply = () => {
      let doc: Document | null = null;
      try {
        doc = iframe.contentDocument;
      } catch {
        return; // cross-origin; bail quietly
      }
      if (!doc?.body) return;
      const frameDoc = doc;
      customizeMarimoFrame(frameDoc);
      if (!observer) {
        observer = new MutationObserver(() => customizeMarimoFrame(frameDoc));
        observer.observe(frameDoc.body, { childList: true, subtree: true });
      }
    };

    iframe.addEventListener("load", apply);
    apply();
    return () => {
      iframe.removeEventListener("load", apply);
      observer?.disconnect();
    };
  }, [snapshot.url]);

  if (snapshot.error) {
    return (
      <div
        className={`flex ${FRAME_HEIGHT} flex-col items-center justify-center gap-3 p-8 text-center`}
      >
        <NotebookPen className="h-8 w-8 text-muted-foreground" />
        <div className="space-y-1">
          <p className="text-sm font-medium">Couldn't start the notebook</p>
          <p className="max-w-md text-sm text-muted-foreground">
            {snapshot.error}
          </p>
        </div>
      </div>
    );
  }

  if (snapshot.loading || !snapshot.url) {
    return (
      <div
        className={`flex ${FRAME_HEIGHT} items-center justify-center gap-2 text-sm text-muted-foreground`}
      >
        <Loader2 className="h-4 w-4 animate-spin" /> Starting notebook
        environment…
      </div>
    );
  }

  return (
    <div className={`relative ${FRAME_HEIGHT} w-full`}>
      {!loaded && (
        <div className="absolute inset-0 flex items-center justify-center bg-background">
          <div className="flex flex-col items-center gap-2 text-muted-foreground">
            <Loader2 className="h-6 w-6 animate-spin" />
            <span className="text-sm">Loading notebook…</span>
          </div>
        </div>
      )}
      <iframe
        ref={iframeRef}
        title="Notebook"
        src={snapshot.url}
        className="h-full w-full border-0"
        onLoad={() => setLoaded(true)}
      />
    </div>
  );
}
