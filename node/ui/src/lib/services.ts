import { FlaskConical, type LucideIcon, NotebookPen } from "lucide-react";

// Embeddable service surfaces. Each `path` matches a prefix the Envoy gateway
// serves (see environments/docker/envoy/envoy.yaml) and that the Vite dev
// server proxies (see vite.config.ts). Both apps render their own full UI, so
// they are embedded via <iframe> rather than reimplemented.
export interface ServiceSurface {
  id: string;
  label: string;
  description: string;
  path: string;
  icon: LucideIcon;
  // Optional, idempotent tweaks applied to the embedded document. Only invoked
  // when the iframe is same-origin (it is here: both the UI and the service
  // prefix are served from the same Envoy/Vite origin). Called once on load and
  // again on DOM mutations, so it must be safe to run repeatedly.
  customizeFrame?: (doc: Document) => void;
}

// marimo's workspace/home page (served at `/marimo/` with no notebook open)
// renders a "Resources" block (Documentation/GitHub/Discord/molab/YouTube/
// Changelog) and a "Tutorials" dropdown. These are hard-coded in marimo's
// frontend with no server-config toggle, and `display.custom_css` is a no-op on
// the home page (marimo skips custom CSS injection when there is no notebook
// filename). So we strip them client-side from the embedded document instead.
function customizeMarimoFrame(doc: Document): void {
  // Stable hooks: hide via a single injected stylesheet (idempotent by id).
  const STYLE_ID = "ol-marimo-customizations";
  if (!doc.getElementById(STYLE_ID)) {
    const style = doc.createElement("style");
    style.id = STYLE_ID;
    style.textContent = `
      /* Tutorials dropdown trigger on the workspace home page. */
      [data-testid="open-tutorial-button"] { display: none !important; }
    `;
    doc.head.append(style);
  }

  // The "Resources" block has no stable selector, so anchor on its heading
  // text. Structure (marimo home/components.tsx): a section <div> whose first
  // child is the Header's <div>, which contains an <h2> with the text
  // "Resources". Two levels up from that <h2> is the section root.
  for (const heading of doc.querySelectorAll("h2")) {
    if (heading.textContent?.trim() === "Resources") {
      const section = heading.parentElement?.parentElement;
      if (section instanceof HTMLElement) {
        section.style.display = "none";
      }
    }
  }
}

export const SERVICE_SURFACES: ServiceSurface[] = [
  {
    id: "mlflow",
    label: "MLflow",
    description: "Experiment tracking and model registry",
    // Trailing slash: the apps redirect the bare prefix to an absolute gateway
    // URL, so we request the canonical path directly to stay same-origin.
    path: "/mlflow/",
    icon: FlaskConical,
  },
  {
    id: "marimo",
    label: "Marimo",
    description: "Reactive Python notebooks",
    path: "/marimo/",
    icon: NotebookPen,
    customizeFrame: customizeMarimoFrame,
  },
];

export function getServiceSurface(id: string): ServiceSurface | undefined {
  return SERVICE_SURFACES.find((s) => s.id === id);
}
