import type { StorybookConfig } from "@storybook/react-vite";

// The console's own vite.config.ts is merged in (React, Tailwind v4, the `@` aliases).
const config: StorybookConfig = {
  stories: ["../src/**/*.stories.@(ts|tsx)"],
  framework: "@storybook/react-vite",
  core: { disableTelemetry: true },
};

export default config;
