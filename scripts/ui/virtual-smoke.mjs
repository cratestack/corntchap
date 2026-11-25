// Smoke check for the virtualized lists: render the 10k-row Storybook stories and assert that only a bounded number
// of rows is in the DOM and that scrolling to the end renders the last row.
// Usage: pnpm -C console build-storybook && node scripts/ui/virtual-smoke.mjs   (serves console/storybook-static itself)
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { createRequire } from "node:module";
import { execSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const require = createRequire(execSync("npm root -g").toString().trim() + "/");
const { chromium } = require("playwright");

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../console/storybook-static");
const MAX_ROWS = 200;
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml", ".woff2": "font/woff2", ".woff": "font/woff" };

const server = http.createServer((req, res) => {
  const file = path.join(root, decodeURIComponent(new URL(req.url, "http://x").pathname));
  const target = fs.existsSync(file) && fs.statSync(file).isFile() ? file : path.join(root, "index.html");
  if (!target.startsWith(root)) return res.writeHead(403).end();
  res.writeHead(200, { "content-type": TYPES[path.extname(target)] ?? "application/octet-stream" }).end(fs.readFileSync(target));
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}`;

const desktop = { width: 1280, height: 800 };
const phone = { width: 390, height: 844 };
// [story id, viewport, text of the last row (a regex, matched against a whole element's text)]
const CASES = [
  ["ui-virtuallist--ten-thousand-mixed-heights", desktop, /^Row 10000$/],
  ["ui-virtuallist--phone", phone, /^Row 10000$/],
  ["ui-virtuallist--window-scroll", phone, /^Row 10000$/],
  ["ui-virtualgroupedlist--ten-thousand-mixed-heights", desktop, /^Row 10000$/],
  ["ui-virtualgroupedlist--window-scroll", phone, /^Row 10000$/],
  ["ui-virtualcolumn--ten-thousand-mixed-heights", desktop, /^Row 10000$/],
  ["console-eventfeed--ten-thousand", desktop, /Event 1:$/],
  ["console-eventfeed--phone", phone, /Event 1:$/],
  ["console-board--ten-thousand-tasks", desktop, /^PAY-9996$/],
  ["console-board--phone", phone, /^PAY-9996$/],
];

const browser = await chromium.launch();
let failed = 0;
for (const [id, viewport, last] of CASES) {
  const page = await browser.newPage({ viewport, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=${id}&viewMode=story`);
  await page.waitForSelector("[role=listitem]", { timeout: 20000 });
  const count = () => page.locator("[role=listitem]").count();
  const initial = await count();
  let peak = initial;
  let found = false;
  for (let i = 0; i < 60 && !found; i++) {
    await page.evaluate(() => {
      window.scrollTo(0, document.documentElement.scrollHeight);
      for (const el of document.querySelectorAll("[data-virtuoso-scroller]")) el.scrollTop = el.scrollHeight;
    });
    await page.waitForTimeout(120);
    peak = Math.max(peak, await count());
    found = (await page.getByText(last).count()) > 0;
  }
  const ok = initial < MAX_ROWS && peak < MAX_ROWS && found && errors.length === 0;
  if (!ok) failed++;
  console.log(`${ok ? "PASS" : "FAIL"} ${id} initial=${initial} peak=${peak} last=${last} ${found ? "rendered" : "NOT rendered"}${errors.length ? " errors=" + errors.join("|") : ""}`);
  await page.close();
}
await browser.close();
server.close();
process.exit(failed ? 1 : 0);
