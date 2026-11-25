// Smoke check for the virtualized lists: render the 10k-row Storybook stories and assert that only a bounded number
// of rows is in the DOM and that scrolling to the end renders the last row. Then the VirtualThread stories: loading
// older messages keeps the message you were reading where it was (within 2px), a "New messages" pill shows when
// one arrives while you are scrolled up, and the thread follows new messages while you are at the bottom.
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
  ["ui-virtualtable--ten-thousand-rows", desktop, /^Row 10000$/, "tbody tr[data-index]"],
  ["ui-virtualtable--phone", phone, /^Row 10000$/],
];

const browser = await chromium.launch();
let failed = 0;
for (const [id, viewport, last, rowSelector = "[role=listitem]"] of CASES) {
  const page = await browser.newPage({ viewport, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=${id}&viewMode=story`);
  await page.waitForSelector(rowSelector, { timeout: 20000 });
  const count = () => page.locator(rowSelector).count();
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

// ---- VirtualThread ----

const SCROLLER = "[data-virtuoso-scroller]";
const gap = (page) => page.evaluate((sel) => { const el = document.querySelector(sel); return el.scrollHeight - el.scrollTop - el.clientHeight; }, SCROLLER);
const topOf = (page, seq) =>
  page.evaluate(([sel, seq]) => {
    const el = document.querySelector(`[data-seq="${seq}"]`);
    return el ? el.getBoundingClientRect().top - document.querySelector(sel).getBoundingClientRect().top : null;
  }, [SCROLLER, seq]);
const check = (name, ok, detail) => {
  if (!ok) failed++;
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? " " + detail : ""}`);
};

for (const [label, viewport] of [["desktop", desktop], ["phone", phone]]) {
  const page = await browser.newPage({ viewport, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=ui-virtualthread--ten-thousand-messages&viewMode=story`);
  await page.waitForSelector("[data-testid=msg]", { timeout: 20000 });
  await page.waitForTimeout(400);
  check(`thread ${label}: opens at the newest message`, (await page.locator('[data-seq="10000"]').count()) === 1 && (await gap(page)) < 80, `gap=${await gap(page)}`);

  // scroll to the top of what is loaded (message 9901): the older page loads 300 ms later
  await page.evaluate((sel) => { document.querySelector(sel).scrollTop = 0; }, SCROLLER);
  await page.waitForSelector('[data-seq="9901"]', { timeout: 5000 });
  await page.waitForTimeout(80);
  const before = await topOf(page, 9901);
  await page.waitForSelector('[data-seq="9900"]', { timeout: 5000, state: "attached" });
  await page.waitForTimeout(500);
  const after = await topOf(page, 9901);
  check(`thread ${label}: loading older keeps the anchored message in place`, before !== null && after !== null && Math.abs(after - before) <= 2, `before=${before} after=${after}`);
  check(`thread ${label}: no page errors`, errors.length === 0, errors.join("|"));
  await page.close();
}

{
  const page = await browser.newPage({ viewport: desktop, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=ui-virtualthread--live-append&viewMode=story`);
  await page.waitForSelector("[data-testid=msg]", { timeout: 20000 });
  await page.waitForTimeout(500);
  const lastSeq = () => page.evaluate(() => Math.max(...[...document.querySelectorAll("[data-testid=msg]")].map((e) => Number(e.dataset.seq))));
  const pill = page.getByRole("button", { name: /New messages/ });

  // at the bottom: new messages are followed and there is no pill
  const seq0 = await lastSeq();
  await page.waitForTimeout(3200);
  const seq1 = await lastSeq();
  check("thread: follows new messages at the bottom", seq1 > seq0 && (await gap(page)) < 80 && (await pill.count()) === 0, `seq ${seq0} -> ${seq1} gap=${await gap(page)}`);

  // scrolled up: the thread stays put and the pill appears
  await page.evaluate((sel) => { const el = document.querySelector(sel); el.scrollTop = el.scrollTop - 900; }, SCROLLER);
  await page.waitForTimeout(300);
  const at = await page.evaluate((sel) => document.querySelector(sel).scrollTop, SCROLLER);
  await page.waitForTimeout(2600);
  const stayed = await page.evaluate((sel) => document.querySelector(sel).scrollTop, SCROLLER);
  check("thread: stays put and shows the New messages pill when scrolled up", (await pill.count()) === 1 && Math.abs(stayed - at) <= 2, `scrollTop ${at} -> ${stayed} pill=${await pill.count()}`);

  // the pill takes you to the newest message and goes away
  await pill.click();
  await page.waitForTimeout(1200);
  check("thread: the pill scrolls to the newest message", (await pill.count()) === 0 && (await gap(page)) < 80, `gap=${await gap(page)} pill=${await pill.count()}`);
  check("thread: no page errors (live)", errors.length === 0, errors.join("|"));
  await page.close();
}

await browser.close();
server.close();
process.exit(failed ? 1 : 0);
