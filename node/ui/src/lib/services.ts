import { FlaskConical, type LucideIcon, NotebookPen } from "lucide-react";
import { customizeMarimoFrame } from "@/lib/notebook/customizeFrame";

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
