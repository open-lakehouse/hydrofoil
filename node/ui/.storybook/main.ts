import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { StorybookConfig } from "@storybook/react-vite";

const here = dirname(fileURLToPath(import.meta.url));
// This app's workspace root (hydrofoil node/).
const workspaceRoot = resolve(here, "../..");
// The shared UI packages now live in the sibling mangrove repo (consumed via
// file: links). Storybook needs to read their source (stories + components), so
// we allow the mangrove node/ root too.
const mangroveNodeRoot = resolve(here, "../../../../mangrove/node");

// Storybook reuses the app's own Vite config (the `@` alias to src/ and the
// Tailwind plugin), so components render with the exact styling/resolution they
// get in the app. The react-vite framework supplies the React + Vite builder.
const config: StorybookConfig = {
  stories: [
    "../src/**/*.stories.@(ts|tsx)",
    // The shared UI packages keep their stories alongside their source, now in
    // the sibling mangrove repo (file:-linked).
    "../../../../mangrove/node/ui-kit/src/**/*.stories.@(ts|tsx)",
    "../../../../mangrove/node/data-grid/src/**/*.stories.@(ts|tsx)",
    "../../../../mangrove/node/unity-catalog/src/**/*.stories.@(ts|tsx)",
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
    // Stories/components now live in the sibling mangrove repo (file:-linked), so
    // (a) let Vite read files from this workspace AND the mangrove node/ root, and
    // (b) pre-bundle the heavier deps those packages pull in — Storybook's
    // optimizer otherwise fails to locate them when the importer is a symlinked
    // sibling package.
    viteConfig.server = {
      ...viteConfig.server,
      fs: {
        ...viteConfig.server?.fs,
        allow: [
          ...(viteConfig.server?.fs?.allow ?? []),
          workspaceRoot,
          mangroveNodeRoot,
        ],
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
