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
  },
];

export function getServiceSurface(id: string): ServiceSurface | undefined {
  return SERVICE_SURFACES.find((s) => s.id === id);
}
