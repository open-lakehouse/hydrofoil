import {
  ThemeProvider,
  Toaster,
  TooltipProvider,
} from "@open-lakehouse/ui-kit";
import type { Decorator, Preview } from "@storybook/react-vite";
import { QueryClientProvider } from "@tanstack/react-query";
import { useEffect } from "react";
import { ActiveEnvironmentProvider } from "@/components/environment/ActiveEnvironmentContext";
import { activeEnvironment } from "@/lib/fixtures";
import { createQueryClient } from "@/lib/query-client";

// The app's global stylesheet (Tailwind layers + design tokens) — the same one
// src/main.tsx imports, so stories render pixel-identical to the app.
import "@/app/globals.css";

// Register the fixture fakes at the UI's seams ONCE, before any story renders.
// (Importing for side effect would also work, but the explicit call documents
// intent and stays idempotent.)
import { installStorybookHost } from "./storybook-host";

installStorybookHost();

// Reflect the toolbar theme onto <html class="dark">, the same hook the app's
// ThemeProvider drives, so Tailwind dark: variants apply in stories.
function ThemeSync({ theme }: { theme: string }) {
  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
  }, [theme]);
  return null;
}

// Every story renders inside the app's real provider stack (minus the router):
// a fresh QueryClient per story (no cache bleed between stories), theme,
// tooltips, the active-environment context (mock), and the toaster.
const withProviders: Decorator = (Story, context) => {
  const queryClient = createQueryClient();
  const theme = context.globals.theme ?? "light";
  return (
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <ThemeSync theme={theme} />
        <TooltipProvider delayDuration={300}>
          <ActiveEnvironmentProvider environment={activeEnvironment}>
            <div className="bg-background text-foreground p-6">
              <Story />
            </div>
          </ActiveEnvironmentProvider>
          <Toaster position="bottom-right" />
        </TooltipProvider>
      </ThemeProvider>
    </QueryClientProvider>
  );
};

const preview: Preview = {
  decorators: [withProviders],
  parameters: {
    layout: "fullscreen",
    controls: { expanded: true },
  },
  globalTypes: {
    theme: {
      description: "Color theme",
      defaultValue: "light",
      toolbar: {
        title: "Theme",
        icon: "circlehollow",
        items: [
          { value: "light", icon: "sun", title: "Light" },
          { value: "dark", icon: "moon", title: "Dark" },
        ],
        dynamicTitle: true,
      },
    },
  },
};

export default preview;
