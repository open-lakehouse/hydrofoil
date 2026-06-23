// A fake EnvironmentHost serving the environment-management surfaces
// (EnvironmentManager, EnvironmentDetail) from fixtures, registered via
// `registerEnvironmentHost`. Mirrors node/desktop's tauriEnvironmentHost shape
// but is fully in-memory: `managed: true` so the picker/manager render, with a
// couple of environments, a capability checklist, config artifacts, and live
// service statuses for the running one.

import type {
  ActiveEnvironment,
  Capability,
  ConfigArtifact,
  Environment,
  EnvironmentHost,
  KeyStatus,
  ServiceStatus,
} from "@/lib/client/environments";
import { activeEnvironment } from "@/lib/fixtures";

const ENVIRONMENTS: Environment[] = [
  { id: "fixture-env", name: "Fixtures" },
  { id: "scratch", name: "Scratch" },
];

const CAPABILITIES: Capability[] = [
  { id: "lineage", label: "Data lineage" },
  { id: "observability", label: "Observability (Jaeger)" },
  { id: "models", label: "Model tracking (MLflow)" },
  { id: "storage", label: "Object storage (Azurite)" },
];

const CONFIG_ARTIFACTS: ConfigArtifact[] = [
  {
    id: "compose",
    label: "docker-compose.yaml",
    description: "Generated compose for the selected capabilities.",
    language: "yaml",
    content:
      "services:\n  unity-catalog:\n    image: unitycatalog:latest\n    ports:\n      - 8081:8081\n",
  },
  {
    id: "envoy",
    label: "envoy.yaml",
    description: "Gateway routing for the environment's services.",
    language: "yaml",
    content: "static_resources:\n  listeners: []\n",
  },
];

const SERVICE_STATUS: ServiceStatus[] = [
  {
    service: "unity-catalog",
    state: "running",
    health: "healthy",
    shared: false,
  },
  { service: "hydrofoil", state: "running", health: "healthy", shared: false },
  { service: "jaeger", state: "running", health: "starting", shared: true },
];

/** A fully-featured managed host for the environment-management stories. */
export const fixtureEnvironmentHost: EnvironmentHost = {
  managed: true,
  list: async () => ENVIRONMENTS,
  active: async () => activeEnvironment,
  create: async (name) => ({ id: name.toLowerCase(), name }),
  start: async () => activeEnvironment,
  stop: async () => {},
  keyStatus: async (): Promise<KeyStatus> => "keychain",
  configureKey: async (): Promise<KeyStatus> => "keychain",
  dockerStatus: async () => true,
  availableCapabilities: async () => CAPABILITIES,
  environmentCapabilities: async () => ["lineage", "observability"],
  setEnvironmentCapabilities: async () => {},
  configArtifacts: async () => CONFIG_ARTIFACTS,
  serviceStatus: async (id) =>
    id === activeEnvironment.id ? SERVICE_STATUS : [],
  telemetryStatus: async () => true,
};

/** The active environment the manager treats as "running" by default. */
export const fixtureActiveEnvironment: ActiveEnvironment = activeEnvironment;
