import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { StorybookConfig } from "@storybook/react-vite";

const here = dirname(fileURLToPath(import.meta.url));
// The workspace root (node/), one level above this app — the carved-out UI
// packages live here as sibling workspaces.
const workspaceRoot = resolve(here, "../..");

// Storybook reuses the app's own Vite config (the `@` alias to src/ and the
// Tailwind plugin), so components render with the exact styling/resolution they
// get in the app. The react-vite framework supplies the React + Vite builder.
const config: StorybookConfig = {
  stories: [
    "../src/**/*.stories.@(ts|tsx)",
    // The carved-out UI packages keep their stories alongside their source.
    "../../ui-kit/src/**/*.stories.@(ts|tsx)",
    "../../data-grid/src/**/*.stories.@(ts|tsx)",
    "../../unity-catalog/src/**/*.stories.@(ts|tsx)",
  ],
  addons: [],
  framework: {
    name: "@storybook/react-vite",
    options: {},
  },
  core: {
    disableTelemetry: true,
  },
  viteFinal: (viteConfig) => {
    // Stories now live in sibling workspace packages outside this app's root, so
    // (a) let Vite read files from the whole workspace, and (b) pre-bundle the
    // heavier deps those packages pull in — Storybook's optimizer otherwise fails
    // to locate them when the importer is a symlinked sibling package.
    viteConfig.server = {
      ...viteConfig.server,
      fs: {
        ...viteConfig.server?.fs,
        allow: [...(viteConfig.server?.fs?.allow ?? []), workspaceRoot],
      },
    };
    viteConfig.optimizeDeps = {
      ...viteConfig.optimizeDeps,
      include: [...(viteConfig.optimizeDeps?.include ?? []), "apache-arrow"],
    };
    return viteConfig;
  },
};

export default config;
