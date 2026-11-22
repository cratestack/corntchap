import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig, loadEnv } from "vite";

// `just console-dev` sets CORNTCHAP_URL to the running `just serve` (from .env.dev).
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const target = process.env.CORNTCHAP_URL ?? env.CORNTCHAP_URL ?? "http://127.0.0.1:8080";
  // The server's CSRF check compares Origin with PUBLIC_URL, so the proxy presents the server's own origin.
  const proxy = Object.fromEntries(
    ["/api", "/auth", "/logout", "/oauth", "/device", "/files"].map((p) => [
      p,
      { target, changeOrigin: false, xfwd: true, headers: { origin: target } },
    ]),
  );
  return {
    plugins: [react(), tailwindcss()],
    resolve: {
      alias: {
        "@": fileURLToPath(new URL("./src", import.meta.url)),
        "@corntchap/api": fileURLToPath(new URL("./src/gen/src/index.ts", import.meta.url)),
      },
    },
    server: { port: 5173, proxy },
    build: { outDir: "dist", sourcemap: false, chunkSizeWarningLimit: 1200 },
  };
});
