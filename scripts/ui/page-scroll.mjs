// Page-scroll check: every page scrolls vertically with the window at every screen size. For each Storybook story that
// stands for a page (board, activity feed, users table, the pass, the virtualized lists) at 375, 768, 1280 and 1920 px:
//   - the page does not scroll horizontally (document.scrollingElement.scrollWidth <= clientWidth);
//   - no element has a vertical scroller of its own (computed overflow-y auto/scroll holding more than it shows),
//     except the chat thread (`[role=log]`), which is anchored to the bottom by design.
// Usage: pnpm -C console build-storybook && node scripts/ui/page-scroll.mjs   (serves console/storybook-static itself)
import { chromium, serve } from "./storybook.mjs";

const WIDTHS = (process.env.WIDTHS ?? "375,768,1280,1920").split(",").map(Number);
const ONLY = process.env.ONLY;
// [story id, a selector that is there once the story has rendered]
const PAGES = [
  ["console-board--ten-thousand-tasks", "[role=listitem]"],
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
      const bodyO = getComputedStyle(document.body).overflowY;
      return { sw: root.scrollWidth, cw: root.clientWidth, panes, bodyO };
    }, ALLOWED);
    const ok = r.sw <= r.cw && r.panes.length === 0 && errors.length === 0;
    if (!ok) failed++;
    console.log(`${ok ? "PASS" : "FAIL"} page-scroll ${id} @${width}${ok ? "" : ` scrollWidth=${r.sw} clientWidth=${r.cw} panes=${JSON.stringify(r.panes)}${errors.length ? " errors=" + errors.join("|") : ""}`}`);
    await page.close();
  }
}

await browser.close();
close();
process.exit(failed ? 1 : 0);
