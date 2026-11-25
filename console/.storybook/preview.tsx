import { useEffect } from "react";
import type { Decorator, Preview } from "@storybook/react-vite";
import { MemoryRouter } from "react-router";
import "../src/index.css";

// The console's tokens (src/index.css) switch on `.dark` on <html>; the toolbar toggles it like the console does.
const withTheme: Decorator = (Story, context) => {
  const dark = context.globals["theme"] === "dark";
  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    document.documentElement.style.colorScheme = dark ? "dark" : "light";
  }, [dark]);
  return <Story />;
};

const withRouter: Decorator = (Story) => (
  <MemoryRouter>
    <Story />
  </MemoryRouter>
);

const preview: Preview = {
  decorators: [withRouter, withTheme],
  globalTypes: {
    theme: {
      description: "Console theme",
      toolbar: {
        title: "Theme",
        icon: "circlehollow",
        items: [
          { value: "light", title: "Light" },
          { value: "dark", title: "Dark" },
        ],
        dynamicTitle: true,
      },
    },
  },
  initialGlobals: { theme: "light" },
  parameters: {
    layout: "fullscreen",
    controls: { disableSaveFromUI: true },
    viewport: {
      options: {
        phone: { name: "Phone", styles: { width: "390px", height: "844px" }, type: "mobile" },
        desktop: { name: "Desktop", styles: { width: "1280px", height: "800px" }, type: "desktop" },
      },
    },
  },
};

export default preview;
