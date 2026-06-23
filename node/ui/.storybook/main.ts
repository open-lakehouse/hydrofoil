import type { StorybookConfig } from "@storybook/react-vite";

// Storybook reuses the app's own Vite config (the `@` alias to src/ and the
// Tailwind plugin), so components render with the exact styling/resolution they
// get in the app. The react-vite framework supplies the React + Vite builder.
const config: StorybookConfig = {
  stories: ["../src/**/*.stories.@(ts|tsx)"],
  addons: [],
  framework: {
    name: "@storybook/react-vite",
    options: {},
  },
  core: {
    disableTelemetry: true,
  },
};

export default config;
