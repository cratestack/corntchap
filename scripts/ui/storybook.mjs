// Shared by the UI smoke scripts: serve console/storybook-static on a free port and load Playwright from the global install.
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { createRequire } from "node:module";
import { execSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const require = createRequire(execSync("npm root -g").toString().trim() + "/");
export const { chromium } = require("playwright");

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../console/storybook-static");
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml", ".woff2": "font/woff2", ".woff": "font/woff" };

/** Serves the built Storybook; returns its base URL and a `close`. */
export async function serve() {
  const server = http.createServer((req, res) => {
    const file = path.join(root, decodeURIComponent(new URL(req.url, "http://x").pathname));
    const target = fs.existsSync(file) && fs.statSync(file).isFile() ? file : path.join(root, "index.html");
    if (!target.startsWith(root)) return res.writeHead(403).end();
    res.writeHead(200, { "content-type": TYPES[path.extname(target)] ?? "application/octet-stream" }).end(fs.readFileSync(target));
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return { base: `http://127.0.0.1:${server.address().port}`, close: () => server.close() };
}
