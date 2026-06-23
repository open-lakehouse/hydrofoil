import type { Meta, StoryObj } from "@storybook/react-vite";

import { activeEnvironment } from "@/lib/fixtures";
import { EnvironmentManager } from "./EnvironmentManager";

// The manager lists environments and shows per-environment detail. It reads the
// environment list / capabilities / config artifacts / service status through
// getEnvironmentHost(), which the Storybook host satisfies with a fake managed
// host (.storybook/fixture-environments.ts). The running environment, transient
// transition, and error are props.
const meta: Meta<typeof EnvironmentManager> = {
  title: "Environment/EnvironmentManager",
  component: EnvironmentManager,
  parameters: { layout: "fullscreen" },
  args: {
    running: null,
    transition: null,
    lastError: null,
    onOpen: () => {},
    onStart: () => {},
    onLaunch: () => {},
    onStop: () => {},
  },
};

export default meta;
type Story = StoryObj<typeof EnvironmentManager>;

/** First run: no environment running yet. */
export const Idle: Story = {};

/** The fixture environment is running — detail shows live service statuses. */
export const Running: Story = {
  args: { running: activeEnvironment },
};

/** A start is in flight (transient "starting" state). */
export const Starting: Story = {
  args: { transition: { id: activeEnvironment.id, kind: "starting" } },
};

/** A failed start surfaces the error. */
export const StartFailed: Story = {
  args: { lastError: "Docker is not running. Start Docker Desktop and retry." },
};
