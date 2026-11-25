// Smoke check for the virtualized lists: render the 10k-row Storybook stories and assert that only a bounded number
// of rows is in the DOM and that scrolling to the end renders the last row. Then the VirtualThread stories: loading
// older messages keeps the message you were reading where it was (within 2px), a "New messages" pill shows when
// one arrives while you are scrolled up, and the thread follows new messages while you are at the bottom.
// Usage: pnpm -C console build-storybook && node scripts/ui/virtual-smoke.mjs   (serves console/storybook-static itself)
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { chromium, serve } from "./storybook.mjs";

const MAX_ROWS = 200;
const { base, close } = await serve();

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

// ---- the board: the page scrolls through the columns, and every column pages on its own ----

const loadsOf = (page) => page.evaluate(() => { const o = document.querySelector("[data-testid=loads]"); return Object.fromEntries(["ready", "claimed", "waiting", "done", "cancelled"].map((k) => [k, Number(o.dataset[k])])); });
const wide = { width: 1920, height: 900 };
const COLUMN_NAMES = ["Ready to scoop", "On the fire", "Waiting", "Served", "Tossed"];

// Page scrolling puts one column's end in view: scroll the window so the bottom of the column's cards is a third of
// the way down the viewport.
const toColumnEnd = (page, name) =>
  page.evaluate((n) => {
    const el = [...document.querySelectorAll("section")].find((e) => e.getAttribute("aria-label") === n);
    if (el) window.scrollTo(0, el.getBoundingClientRect().bottom + window.scrollY - window.innerHeight / 3);
  }, name);

{
  // desktop: 2,000 tasks a column on one page: the page is the only scroller, and each column renders only what is near
  const page = await browser.newPage({ viewport: wide, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=console-board--ten-thousand-tasks&viewMode=story`);
  await page.waitForSelector("[role=listitem]", { timeout: 20000 });
  const lasts = { "Ready to scoop": /^PAY-9996$/, "On the fire": /^PAY-9997$/, Waiting: /^PAY-9998$/, Served: /^PAY-9999$/, Tossed: /^PAY-10000$/ };
  let peak = 0;
  const reached = [];
  for (const [name, last] of Object.entries(lasts)) {
    for (let i = 0; i < 40; i++) {
      await toColumnEnd(page, name);
      await page.waitForTimeout(120);
      peak = Math.max(peak, await page.locator("[role=listitem]").count());
      if ((await page.getByRole("region", { name }).getByText(last).count()) > 0) {
        reached.push(name);
        break;
      }
    }
  }
  const own = await page.evaluate(() => [...document.querySelectorAll("[data-virtuoso-scroller]")].filter((e) => e.scrollHeight > e.clientHeight + 1).length);
  check("board desktop: every column's last task renders when the page is scrolled to its end", reached.length === 5, `reached=${reached}`);
  check("board desktop: the DOM stays bounded", peak < MAX_ROWS * 3, `peak=${peak}`);
  check("board desktop: no column has a scroller of its own", own === 0, `scrollers=${own}`);
  check("board desktop: no page errors", errors.length === 0, errors.join("|"));
  await page.close();
}

{
  // desktop, paged: scrolling the page to each column's end loads pages until the mocked total (10,000) is on screen
  const page = await browser.newPage({ viewport: wide, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=console-board--paged-ten-thousand-per-column&viewMode=story`);
  await page.waitForSelector("[role=listitem]", { timeout: 20000 });
  const reached = new Set();
  let peak = 0;
  for (const name of COLUMN_NAMES) {
    for (let i = 0; i < 120 && !reached.has(name); i++) {
      await toColumnEnd(page, name);
      await page.waitForTimeout(100);
      peak = Math.max(peak, await page.locator("[role=listitem]").count());
      if ((await page.getByRole("region", { name }).getByText(/^PAY-10000$/).count()) > 0) reached.add(name);
    }
  }
  const loads = await loadsOf(page);
  check("board desktop paged: every column scrolls to its 10,000th task", reached.size === COLUMN_NAMES.length, `reached=${[...reached]} loads=${JSON.stringify(loads)}`);
  check("board desktop paged: every column asked for its own pages", Object.values(loads).every((n) => n >= 49), JSON.stringify(loads));
  check("board desktop paged: the DOM stays bounded", peak < MAX_ROWS * 3, `peak=${peak}`);
  check("board desktop paged: no page errors", errors.length === 0, errors.join("|"));
  await page.close();
}

{
  // desktop, short columns: nothing loads until a column's end is near the viewport, so a column that is not scrolled to stays put
  const page = await browser.newPage({ viewport: wide, deviceScaleFactor: 1 });
  await page.goto(`${base}/iframe.html?id=console-board--paged-ten-thousand-per-column&viewMode=story`);
  await page.waitForSelector("[role=listitem]", { timeout: 20000 });
  await page.waitForTimeout(800);
  const loads = await loadsOf(page);
  check("board desktop paged: nothing loads before a column's end is near", Object.values(loads).every((n) => n <= 1), JSON.stringify(loads));
  await page.close();
}

{
  // phone: one column under a tab strip; scrolling Served loads Served only; each tab keeps its place
  const page = await browser.newPage({ viewport: phone, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=console-board--paged-phone&viewMode=story`);
  await page.waitForSelector("[role=listitem]", { timeout: 20000 });
  const shown = () => page.locator("[role=tabpanel] [role=listitem]").count();
  check("board phone: Ready is open first", (await page.getByRole("tab", { name: /^Ready/ }).getAttribute("aria-selected")) === "true" && (await shown()) > 0);
  await page.getByRole("tab", { name: /^Served/ }).click();
  await page.waitForTimeout(800);
  for (let i = 0; i < 25; i++) {
    await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
    await page.waitForTimeout(150);
  }
  const loads = await loadsOf(page);
  check("board phone: scrolling Served loads Served", loads.done >= 5, JSON.stringify(loads));
  check("board phone: no other column loads", loads.ready === 0 && loads.claimed === 0 && loads.waiting === 0 && loads.cancelled === 0, JSON.stringify(loads));
  check("board phone: only the open column is in the DOM", (await page.getByRole("region", { name: /^Ready to scoop/ }).count()) === 0);

  // Served is scrolled well down: leave for Ready (which starts at its top), come back, and Served is where it was
  await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight / 2));
  await page.waitForTimeout(400);
  const servedY = await page.evaluate(() => window.scrollY);
  const servedTop = await page.evaluate(() => [...document.querySelectorAll("[role=tabpanel] [role=listitem]")].map((e) => ({ id: e.textContent.match(/PAY-\d+/)?.[0], y: e.getBoundingClientRect().top })).find((r) => r.y > 120));
  await page.getByRole("tab", { name: /^Ready/ }).click();
  await page.waitForTimeout(800);
  const readyY = await page.evaluate(() => window.scrollY);
  check("board phone: a column not seen before starts at its top", readyY < 400, `scrollY=${readyY}`);
  await page.getByRole("tab", { name: /^Served/ }).click();
  await page.waitForTimeout(1200);
  const backY = await page.evaluate(() => window.scrollY);
  const backTop = await page.evaluate(() => [...document.querySelectorAll("[role=tabpanel] [role=listitem]")].map((e) => ({ id: e.textContent.match(/PAY-\d+/)?.[0], y: e.getBoundingClientRect().top })).find((r) => r.y > 120));
  check(
    "board phone: switching back restores the column's scroll position",
    Math.abs(backY - servedY) <= 2 && !!servedTop && servedTop.id === backTop?.id && Math.abs(servedTop.y - backTop.y) <= 3,
    `scrollY ${servedY} -> ${backY}, first card ${servedTop?.id}@${servedTop?.y} -> ${backTop?.id}@${backTop?.y}`,
  );
  const loadsAfter = await loadsOf(page);
  check("board phone: still no hidden column loads", loadsAfter.claimed === 0 && loadsAfter.waiting === 0 && loadsAfter.cancelled === 0, JSON.stringify(loadsAfter));
  check("board phone: no page errors", errors.length === 0, errors.join("|"));
  await page.close();
}

await browser.close();
close();

// the page-scroll checks (no horizontal page scroll, no inner scroll panes) run on the same stories
const scroll = spawnSync(process.execPath, [fileURLToPath(new URL("./page-scroll.mjs", import.meta.url))], { stdio: "inherit" });
process.exit(failed || scroll.status ? 1 : 0);
