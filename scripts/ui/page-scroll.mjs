// Page-scroll check: every page scrolls vertically with the window at every screen size. For each Storybook story that
// stands for a page (board, activity feed, users table, the pass, the virtualized lists, ListPage, FilterBar) at 320, 375, 768, 1280 and 1920 px:
//   - the page does not scroll horizontally (document.scrollingElement.scrollWidth <= clientWidth);
//   - no element has a vertical scroller of its own (computed overflow-y auto/scroll holding more than it shows),
//     except the chat thread (`[role=log]`) and the board's columns (`[role=region][aria-label=Board]`, its row scrolls
//     sideways and each column inside it scrolls up and down), which are scrollers by design;
//   - the board fills the screen below its header, so the page does not scroll vertically either; from 640px up it has
//     five columns side by side (a row, never wrapped), on a phone one column per screen and a header strip of five.
// Usage: pnpm -C console build-storybook && node scripts/ui/page-scroll.mjs   (serves console/storybook-static itself)
import { chromium, serve } from "./storybook.mjs";

const WIDTHS = (process.env.WIDTHS ?? "320,375,768,1280,1920").split(",").map(Number);
const ONLY = process.env.ONLY;
// [story id, a selector that is there once the story has rendered]
const PAGES = [
  ["console-board--ten-thousand-tasks", "[role=listitem]"],
  ["console-board--wide", "[role=listitem]"],
  ["console-board--laptop", "[role=listitem]"],
  ["console-board--tablet", "[role=listitem]"],
  ["console-board--paged-laptop", "[role=listitem]"],
  ["console-board--paged-ten-thousand-per-column", "[role=listitem]"],
  ["console-board--served-paged", "[role=listitem]"],
  ["console-board--phone", "[role=listitem]"],
  ["console-board--loading", "[aria-busy=true]"],
  ["console-board--empty", "text=The board is bare"],
  ["console-eventfeed--ten-thousand", "[role=listitem]"],
  ["ui-virtualtable--ten-thousand-rows", "tr[data-index], [role=listitem]"],
  ["ui-virtualtable--phone", "tr[data-index], [role=listitem]"],
  ["console-passframe--default", "[data-testid=msg]"],
  ["console-passframe--many-conversations", "[data-testid=msg]"],
  ["console-passframe--phone", "[data-testid=msg]"],
  ["ui-virtuallist--window-scroll", "[role=listitem]"],
  ["ui-virtualgroupedlist--window-scroll", "[role=listitem]"],
  ["ui-virtualcolumn--ten-thousand-mixed-heights", "[role=listitem]"],
  ["console-listpage--ten-thousand", "[role=listitem]"],
  ["console-listpage--table", "tr[data-index], [role=listitem]"],
  ["console-listpage--window-scroll", "[role=listitem]"],
  ["console-listpage--paged", "[role=listitem]"],
  ["console-listpage--loading", "[aria-busy=true]"],
  ["console-listpage--empty", "text=No cooks yet."],
  ["console-filterbar--ten-thousand", "[role=search]"],
  ["console-stations--sessions", "[role=listitem]"],
  ["console-stations--sessions-show-more", "[role=listitem]"],
  ["console-stations--cloud-runs-show-more", "[role=listitem]"],
  ["console-reviewslist--ten-thousand", "[role=listitem]"],
  ["console-reviewslist--show-more", "[role=listitem]"],
  ["console-sourcelist--show-more-button", "[role=listitem]"],
];
// Inner scrollers that are the point: the chat thread (VirtualThread inside the pass's `role=log`) and the board's columns.
const ALLOWED = "[role=log], [role=log] *, [role=region][aria-label=Board], [role=region][aria-label=Board] *";

const { base, close } = await serve();
const browser = await chromium.launch();
let failed = 0;

for (const [id, ready] of PAGES) {
  if (ONLY && !id.includes(ONLY)) continue;
  for (const width of WIDTHS) {
    const page = await browser.newPage({ viewport: { width, height: width < 768 ? 812 : 800 }, deviceScaleFactor: 1 });
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto(`${base}/iframe.html?id=${id}&viewMode=story`);
    try {
      await page.waitForSelector(ready, { timeout: 20000 });
    } catch {
      failed++;
      console.log(`FAIL page-scroll ${id} @${width} never rendered ${ready}`);
      await page.close();
      continue;
    }
    await page.waitForTimeout(500);
    const r = await page.evaluate((allowed) => {
      const root = document.scrollingElement;
      const panes = [];
      for (const el of document.querySelectorAll("body *")) {
        if (el.matches(allowed)) continue;
        const o = getComputedStyle(el).overflowY;
        if ((o === "auto" || o === "scroll") && el.scrollHeight > el.clientHeight + 1) panes.push(`${el.tagName.toLowerCase()}.${String(el.className).split(" ").slice(0, 3).join(".")} ${el.scrollHeight}>${el.clientHeight}`);
      }
      // the board: its column sections (one row, or one per screen on a phone), the header strip, and whether the page scrolls down
      const cols = [...document.querySelectorAll("[role=region][aria-label=Board] > section")].map((e) => {
        const b = e.getBoundingClientRect();
        return { x: Math.round(b.left), y: Math.round(b.top), w: Math.round(b.width) };
      });
      const tabs = [...document.querySelectorAll("[role=tablist] [role=tab]")].filter((e) => e.getBoundingClientRect().width > 0).length;
      return { sw: root.scrollWidth, cw: root.clientWidth, sh: root.scrollHeight, ch: root.clientHeight, panes, cols, tabs };
    }, ALLOWED);
    let layout = "";
    if (id.startsWith("console-board--") && r.cols.length > 0) {
      const row = r.cols.length === 5 && r.cols.every((c, i) => c.y === r.cols[0].y && (i === 0 || c.x >= r.cols[i - 1].x + r.cols[i - 1].w));
      const full = r.sh - r.ch <= 1;
      if (!row) layout = ` board layout wrong: want five columns in a row, got ${JSON.stringify(r.cols)}`;
      else if (width >= 640 && r.tabs !== 0) layout = ` board layout wrong: a header strip at ${width}px`;
      else if (width < 640 && (r.tabs !== 5 || r.cols.some((c) => Math.abs(c.w - (width - 32)) > 1))) layout = ` board layout wrong: want a header strip of five and one-screen columns, got tabs=${r.tabs} ${JSON.stringify(r.cols)}`;
      else if (!full && !id.endsWith("loading")) layout = ` board layout wrong: the page scrolls down (${r.sh} > ${r.ch})`;
    }
    const ok = r.sw <= r.cw && r.panes.length === 0 && errors.length === 0 && !layout;
    if (!ok) failed++;
    console.log(`${ok ? "PASS" : "FAIL"} page-scroll ${id} @${width}${ok ? "" : ` scrollWidth=${r.sw} clientWidth=${r.cw} panes=${JSON.stringify(r.panes)}${layout}${errors.length ? " errors=" + errors.join("|") : ""}`}`);
    await page.close();
  }
}

await browser.close();
close();
process.exit(failed ? 1 : 0);
