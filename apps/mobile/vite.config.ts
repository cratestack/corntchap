import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig, type Plugin } from "vite";

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));
const consoleSrc = here("../../console/src");
const mobileRoot = here(".");

/**
 * The picker imports files that live in ../../console/src. The console is NOT a workspace member and
 * has no node_modules in the mobile build, so bare imports made *from those files* (react, clsx,
 * radix-ui, a CSS `@import "tailwindcss"`...) would not resolve. Resolve them as if the importer sat
 * in apps/mobile, so they use the dependencies listed in apps/mobile/package.json.
 */
function consoleDeps(): Plugin {
  return {
    name: "corntchap:console-deps",
    enforce: "pre",
    async resolveId(source, importer, options) {
      if (!importer || !importer.startsWith(consoleSrc)) return null;
      if (source.startsWith(".") || source.startsWith("/") || source.startsWith("@/") || source.startsWith("\0")) return null;
      return this.resolve(source, `${mobileRoot}/package.json`, { ...options, skipSelf: true });
    },
  };
}

// `tauri dev` on a phone sets TAURI_DEV_HOST to the machine's LAN address so the device can reach Vite.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  root: here("ui"),
  base: "./",
  clearScreen: false,
  plugins: [consoleDeps(), react(), tailwindcss()],
  resolve: {
    alias: {
      "@console": consoleSrc,
      // The console's own files import "@/lib/utils" etc.
      "@": consoleSrc,
    },
    dedupe: ["react", "react-dom"],
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    fs: { allow: [here("../..")] },
  },
  build: { outDir: here("dist"), emptyOutDir: true, sourcemap: false, target: "es2022" },
});
