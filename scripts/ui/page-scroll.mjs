// Page-scroll check: every page scrolls vertically with the window at every screen size. For each Storybook story that
// stands for a page (board, activity feed, users table, the pass, the virtualized lists, ListPage, FilterBar) at 320, 375, 768, 1280 and 1920 px:
//   - the page does not scroll horizontally (document.scrollingElement.scrollWidth <= clientWidth);
//   - no element has a vertical scroller of its own (computed overflow-y auto/scroll holding more than it shows),
//     except the chat thread (`[role=log]`), which is anchored to the bottom by design;
//   - the board shows its five columns side by side only where all five fit (1920: one row, no tab strip), and a tab
//     strip with one column everywhere narrower (a wrapping grid would strand a column below one that pages forever).
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
  ["console-board--served-paged", "[role=tablist], [role=listitem]"],
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
// Inner scrollers that are the point: the chat thread (VirtualThread inside the pass's `role=log`).
const ALLOWED = "[role=log], [role=log] *";

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
      // the board: its column sections (side by side in the region, or the one in the tab panel) and whether there are tabs
      const cols = [...document.querySelectorAll("[role=region][aria-label=Board] > section, [role=tabpanel] > section")].map((e) => {
        const b = e.getBoundingClientRect();
        return { x: Math.round(b.left), y: Math.round(b.top), w: Math.round(b.width) };
      });
      const tabs = document.querySelectorAll("[role=tablist] [role=tab]").length;
      return { sw: root.scrollWidth, cw: root.clientWidth, panes, cols, tabs };
    }, ALLOWED);
    let layout = "";
    if (id.startsWith("console-board--") && r.cols.length > 0) {
      // all five fit only at 1920 (the story's board is 1856px wide; five need 1416)
      const across = r.cols.length === 5 && r.tabs === 0 && r.cols.every((c, i) => c.y === r.cols[0].y && (i === 0 || c.x >= r.cols[i - 1].x + r.cols[i - 1].w));
      const tabbed = r.cols.length === 1 && r.tabs === 5;
      if (width >= 1920 ? !across : !tabbed) layout = ` board layout wrong: ${width >= 1920 ? "want five across" : "want a tab strip and one column"}, got ${JSON.stringify(r)}`;
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
