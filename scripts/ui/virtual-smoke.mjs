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

const OPTION = "[role=option]";
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
  // the pickers: 10,000 options each, in a listbox (`role=option` rows) in a popover, sheet, dialog or under a field
  ["ui-virtuallistbox--ten-thousand", desktop, /^cook-10000$/, OPTION],
  ["ui-virtuallistbox--phone", phone, /^cook-10000$/, OPTION],
  ["ui-virtualcombobox--ten-thousand", desktop, /^cook-10000$/, OPTION],
  ["ui-virtualcombobox--phone", phone, /^cook-10000$/, OPTION],
  ["ui-virtualmenu--ten-thousand", desktop, /^cook-10000$/, OPTION],
  ["ui-virtualcommand--ten-thousand", desktop, /^cook-10000$/, OPTION],
  ["console-taginput--ten-thousand-suggestions", desktop, /^label-10000$/, OPTION],
  ["console-mentionpopup--ten-thousand-chefs", desktop, /^cook-10000$/, OPTION],
  ["console-listpage--ten-thousand", desktop, /^cook-10000$/],
  ["console-listpage--phone", phone, /^cook-10000$/],
  ["console-listpage--table", desktop, /^cook-10000$/, "tbody tr[data-index]"],
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

// ---- the pickers' keyboard: End then ArrowUp x5 lands on option 9995 (of 10,000), which is rendered and in view; Enter picks it ----

const check = (name, ok, detail) => {
  if (!ok) failed++;
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? " " + detail : ""}`);
};

const KEYBOARD = [
  ["ui-virtuallistbox--ten-thousand", desktop],
  ["ui-virtualcombobox--ten-thousand", desktop],
  ["ui-virtualcombobox--phone", phone],
  ["ui-virtualmenu--ten-thousand", desktop],
  ["ui-virtualmenu--not-searchable", desktop, 6],
  ["ui-virtualcommand--ten-thousand", desktop],
  ["ui-virtualcommand--phone", phone],
  ["console-taginput--ten-thousand-suggestions", desktop],
  ["console-mentionpopup--ten-thousand-chefs", desktop],
];
for (const [id, viewport, total = 10000] of KEYBOARD) {
  const page = await browser.newPage({ viewport, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=${id}&viewMode=story`);
  await page.waitForSelector(OPTION, { timeout: 20000 });
  await page.waitForTimeout(300);
  const combo = page.locator("[role=combobox]").first();
  const target = (await combo.count()) ? combo : page.locator("[role=listbox]").first();
  await target.focus();
  await page.keyboard.press("End");
  await page.waitForTimeout(500);
  for (let i = 0; i < 5; i++) await page.keyboard.press("ArrowUp");
  await page.waitForTimeout(400);
  const at = await page.evaluate(() => {
    const el = document.querySelector("[role=combobox], [role=listbox]");
    const opt = document.getElementById(el.getAttribute("aria-activedescendant") ?? "");
    const scroller = opt?.closest("[data-virtuoso-scroller]");
    if (!opt || !scroller) return null;
    const o = opt.getBoundingClientRect();
    const s = scroller.getBoundingClientRect();
    return { text: opt.textContent, selectedRole: opt.getAttribute("role"), inView: o.top >= s.top - 1 && o.bottom <= s.bottom + 1 };
  });
  const want = String(total - 5);
  check(`keyboard ${id}: End then ArrowUp x5 reaches option ${want}, rendered and in view`, !!at && at.selectedRole === "option" && at.inView && at.text.includes(want), JSON.stringify(at));
  await page.keyboard.press("Enter");
  await page.waitForTimeout(300);
  const picked = await page.locator("[data-testid=picked]").textContent();
  check(`keyboard ${id}: Enter picks it`, picked.includes(want), `picked=${picked}`);
  check(`keyboard ${id}: no page errors`, errors.length === 0, errors.join("|"));
  await page.close();
}

// ---- the pickers' query: typing "999" into a server-backed picker leaves only the options that match ----

for (const [id, viewport] of [
  ["ui-virtualcombobox--paged", desktop],
  ["ui-virtualmenu--paged", desktop],
  ["ui-virtualcommand--paged", desktop],
  ["console-taginput--paged", desktop],
  ["console-mentionpopup--paged", desktop],
  ["ui-virtualcombobox--paged", phone],
]) {
  const page = await browser.newPage({ viewport, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=${id}&viewMode=story`);
  await page.waitForSelector(OPTION, { timeout: 20000 });
  const input = page.locator("[role=combobox]").first();
  // (the mention story's text ends in "@" with the caret after it, and focusing keeps the caret there: "999" completes it)
  await input.fill("999");
  await page.waitForTimeout(900);
  const texts = await page.locator(OPTION).allTextContents();
  check(`query ${id}@${viewport.width}: "999" leaves only matching options`, texts.length > 0 && texts.length < MAX_ROWS && texts.every((t) => t.includes("999")), `options=${texts.length} first=${texts[0]}`);
  check(`query ${id}@${viewport.width}: no page errors`, errors.length === 0, errors.join("|"));
  await page.close();
}

// ---- review fixes: TagInput in a form, a query still pending, picked text, URL filters, active row after scrolling, End while paging, late sections ----

const story = async (id, viewport = desktop) => {
  const page = await browser.newPage({ viewport, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=${id}&viewMode=story`);
  return { page, errors };
};
const activeText = (page) =>
  page.evaluate(() => {
    const el = document.querySelector("[role=combobox], [role=listbox]");
    const id = el?.getAttribute("aria-activedescendant");
    return id ? (document.getElementById(id)?.textContent ?? "(not rendered)") : null;
  });

{
  // TagInput in a form: nothing highlighted until an arrow key; Enter on an empty field submits; ArrowDown, Enter, Tab adds exactly one tag and moves focus on
  const { page, errors } = await story("console-taginput--inside-form");
  await page.waitForSelector(OPTION, { timeout: 20000 });
  await page.waitForTimeout(300);
  const input = page.getByRole("combobox", { name: "Labels" });
  check("taginput: nothing is active before an arrow key", (await input.getAttribute("aria-activedescendant")) === null && (await page.locator("[role=option][data-active]").count()) === 0);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(150);
  check("taginput: Enter on an empty field submits the form", (await page.getByTestId("submits").textContent()) === "1");
  await page.keyboard.press("ArrowDown");
  await page.waitForTimeout(250);
  check("taginput: the first ArrowDown activates option 0", (await activeText(page)) === "label-1", String(await activeText(page)));
  await page.keyboard.press("Enter");
  await page.waitForTimeout(150);
  await page.keyboard.press("Tab");
  await page.waitForTimeout(250);
  const focus = await page.evaluate(() => document.activeElement?.getAttribute("aria-label"));
  check("taginput: ArrowDown, Enter, Tab adds exactly one tag and moves focus out", (await page.getByTestId("picked").textContent()) === "label-1" && focus === "Next field", `picked=${await page.getByTestId("picked").textContent()} focus=${focus}`);
  check("taginput: no page errors", errors.length === 0, errors.join("|"));
  await page.close();
}
{
  const { page } = await story("console-taginput--inside-form");
  await page.waitForSelector(OPTION, { timeout: 20000 });
  await page.getByRole("combobox", { name: "Labels" }).fill("zzz");
  await page.keyboard.press("Tab");
  await page.waitForTimeout(250);
  const focus = await page.evaluate(() => document.activeElement?.getAttribute("aria-label"));
  check("taginput: Tab moves on and the text typed is added when the field is left", (await page.getByTestId("picked").textContent()) === "zzz" && focus === "Next field", `picked=${await page.getByTestId("picked").textContent()} focus=${focus}`);
  await page.close();
}

// a query still pending: Enter typed at once picks from the answer to the new text, never a row of the old one
for (const [id, viewport, keys] of [
  ["ui-virtualcombobox--paged", desktop, ["Enter"]],
  ["ui-virtualmenu--paged", desktop, ["Enter"]],
  ["ui-virtualcommand--paged", desktop, ["Enter"]],
  ["console-taginput--paged", desktop, ["ArrowDown", "Enter"]],
]) {
  const { page, errors } = await story(id, viewport);
  await page.waitForSelector(OPTION, { timeout: 20000 });
  await page.waitForTimeout(300);
  await page.locator("[role=combobox]").first().fill("999");
  for (const k of keys) await page.keyboard.press(k);
  await page.waitForTimeout(1200);
  const picked = await page.locator("[data-testid=picked]").textContent();
  check(`pending ${id}: Enter typed at once picks from the new query`, picked.includes("999"), `picked=${picked}`);
  check(`pending ${id}: no page errors`, errors.length === 0, errors.join("|"));
  await page.close();
}
{
  // clicking a stale row while the query is pending picks nothing
  const { page } = await story("ui-virtualcombobox--paged");
  await page.waitForSelector(OPTION, { timeout: 20000 });
  await page.waitForTimeout(300);
  await page.locator("[role=combobox]").first().fill("999");
  await page.locator(OPTION).first().click({ force: true });
  await page.waitForTimeout(100);
  const picked = await page.locator("[data-testid=picked]").textContent();
  check("pending: a click on a row of the old query picks nothing", picked === "", `picked=${picked}`);
  await page.close();
}

{
  // the trigger names the pick by its text: restored (valueText), and remembered after a search moves the rows on
  const { page, errors } = await story("ui-virtualcombobox--restored-value");
  const trigger = page.getByRole("button", { name: "Cook", exact: true });
  await trigger.waitFor({ timeout: 20000 });
  check("combobox: a restored value shows its text, not its key", ((await trigger.textContent()) ?? "").includes("Cook number 9000"), await trigger.textContent());
  await trigger.click();
  await page.locator("[role=combobox]").fill("cook-30");
  await page.waitForTimeout(900);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(300);
  await trigger.click();
  await page.locator("[role=combobox]").fill("cook-4000");
  await page.waitForTimeout(900);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(300);
  check("combobox: a picked value keeps its text after the search moved on", ((await trigger.textContent()) ?? "").includes("Cook number 30"), await trigger.textContent());
  check("combobox: no page errors (restored)", errors.length === 0, errors.join("|"));
  await page.close();
}
{
  const { page } = await story("console-filterbar--restored-values");
  const chip = page.getByRole("button", { name: "Owner: Cook number 9000" });
  await chip.waitFor({ timeout: 20000 }).catch(() => {});
  check("filter chip: a restored value shows its label, not its key", (await chip.count()) === 1);
  await page.close();
}

{
  // URL filters: an explicit "none" survives a non-empty default; several changes in one handler all land
  const { page, errors } = await story("console-filterbar--defaults");
  const state = () => page.getByTestId("url-state").textContent();
  await page.getByTestId("url-state").waitFor({ timeout: 20000 });
  const start = await state();
  await page.getByRole("button", { name: "No owner, no labels" }).click();
  await page.waitForTimeout(150);
  const none = await state();
  await page.getByRole("button", { name: "Back to the defaults" }).click();
  await page.waitForTimeout(150);
  const back = await state();
  await page.getByRole("button", { name: "Owner none, kind task, search x" }).click();
  await page.waitForTimeout(150);
  const many = await state();
  check("url filters: defaults apply at first", start.includes('owner="cook-1"') && start.includes('label=["label-1"]'), start);
  check("url filters: an empty value is kept against a non-empty default", none.includes('owner=""') && none.includes("label=[]"), none);
  check("url filters: clear goes back to the defaults", back.includes('owner="cook-1"') && back.includes('q=""'), back);
  check("url filters: three changes in one handler all land", many.includes('owner=""') && many.includes('kind="task"') && many.includes('q="x"'), many);
  check("url filters: no page errors", errors.length === 0, errors.join("|"));
  await page.close();
}

{
  // after a wheel or touch scroll the active row is not rendered: aria-activedescendant must not name it
  const { page } = await story("ui-virtuallistbox--ten-thousand");
  await page.waitForSelector(OPTION, { timeout: 20000 });
  await page.waitForTimeout(300);
  await page.locator("[role=listbox]").first().focus();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.waitForTimeout(300);
  await page.evaluate(() => { document.querySelector("[data-virtuoso-scroller]").scrollTop = 20000; });
  await page.waitForTimeout(700);
  const away = await page.evaluate(() => { const id = document.querySelector("[role=listbox]").getAttribute("aria-activedescendant"); return id === null ? null : document.getElementById(id) !== null; });
  check("listbox: scrolled away, aria-activedescendant names no unrendered option", away === null || away === true, `attr resolves=${away}`);
  await page.keyboard.press("ArrowDown");
  await page.waitForTimeout(500);
  const back = await page.evaluate(() => { const id = document.querySelector("[role=listbox]").getAttribute("aria-activedescendant"); return id ? document.getElementById(id)?.textContent : null; });
  check("listbox: moving again names a rendered option once more", back.startsWith("cook-4Cook"), `text=${back}`);
  await page.close();
}
{
  // End before every page is loaded: the active row follows the end until it is the true last row
  const { page, errors } = await story("ui-virtualcombobox--paged-short");
  await page.waitForSelector(OPTION, { timeout: 20000 });
  await page.waitForTimeout(300);
  await page.locator("[role=combobox]").focus();
  await page.keyboard.press("End");
  await page.waitForTimeout(3500);
  const at = await activeText(page);
  check("combobox: End before all pages load ends on the true last row", String(at).startsWith("cook-200Cook"), String(at));
  await page.keyboard.press("Enter");
  await page.waitForTimeout(300);
  check("combobox: Enter picks the true last row", (await page.getByTestId("picked").textContent()) === "cook-200");
  check("combobox: no page errors (End)", errors.length === 0, errors.join("|"));
  await page.close();
}
{
  // a section above the active row fills in late: the active row stays on the same item; headings are not listbox content
  const { page, errors } = await story("ui-virtualcommand--late-section");
  await page.waitForSelector(OPTION, { timeout: 20000 });
  await page.locator("[role=combobox]").focus();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.waitForTimeout(250);
  const before = await activeText(page);
  await page.waitForTimeout(2200);
  const after = await activeText(page);
  check("command: the active row stays on the same item when a section above it loads", !!before && before === after && (await page.locator(OPTION).first().textContent()).startsWith("Recent"), `before=${before} after=${after}`);
  const headings = await page.evaluate(() => [...document.querySelectorAll("[role=listbox] p")].filter((p) => p.getAttribute("role") !== "presentation").length);
  check("command: section headings are presentation inside the listbox", headings === 0, `headings=${headings}`);
  check("command: no page errors (late)", errors.length === 0, errors.join("|"));
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

// ---- the board: every column is a scroller of its own in a row that scrolls sideways (phones: one column per screen) ----

const loadsOf = (page) => page.evaluate(() => { const o = document.querySelector("[data-testid=loads]"); return Object.fromEntries(["ready", "claimed", "waiting", "done", "cancelled"].map((k) => [k, Number(o.dataset[k])])); });
const COLUMN_NAMES = ["Ready to scoop", "On the fire", "Waiting", "Served", "Tossed"];
const STATIC_LASTS = [/^PAY-9996$/, /^PAY-9997$/, /^PAY-9998$/, /^PAY-9999$/, /^PAY-10000$/];

// the board's columns as laid out: one box per column section (x relative to the page, so a row scrolled sideways keeps its order)
const columnBoxes = (page) =>
  page.evaluate(() => [...document.querySelectorAll("[role=region][aria-label=Board] > section")].map((e) => { const r = e.getBoundingClientRect(); return { name: e.getAttribute("aria-label"), x: Math.round(r.left + window.scrollX), y: Math.round(r.top + window.scrollY), w: Math.round(r.width), h: Math.round(r.height) }; }));
// five columns on one row, left to right, none overlapping
const sideBySide = async (page) => {
  const b = await columnBoxes(page);
  return b.length === 5 && b.every((c) => c.y === b[0].y) && b.every((c, i) => i === 0 || c.x >= b[i - 1].x + b[i - 1].w);
};
// the page's own overflow: sideways and down (the board fills the screen below the header, so the page does not scroll)
const pageOverflow = (page) => page.evaluate(() => ({ x: document.scrollingElement.scrollWidth - document.scrollingElement.clientWidth, y: document.scrollingElement.scrollHeight - document.scrollingElement.clientHeight }));

// Bring a column into view and scroll it to its end until its last task renders; true when that task is on screen.
async function reachColumn(page, name, last, stats, tries = 60) {
  const section = page.getByRole("region", { name, exact: true });
  for (let i = 0; i < tries; i++) {
    await page.evaluate((n) => {
      const el = [...document.querySelectorAll("[role=region][aria-label=Board] > section")].find((e) => e.getAttribute("aria-label") === n);
      el.scrollIntoView({ behavior: "instant", block: "nearest", inline: "nearest" });
      const sc = el.querySelector("[data-virtuoso-scroller]");
      if (sc) sc.scrollTop = sc.scrollHeight;
    }, name);
    await page.waitForTimeout(110);
    stats.peak = Math.max(stats.peak, await page.locator("[role=listitem]").count());
    const item = section.getByText(last);
    if ((await item.count()) > 0) {
      const box = await item.first().boundingBox();
      const vp = page.viewportSize();
      if (box && box.x >= -1 && box.y >= -1 && box.x + box.width <= vp.width + 1 && box.y + box.height <= vp.height + 1) return true;
    }
  }
  return false;
}

// from 640px up: five columns side by side, the row scrolling sideways where they do not fit
for (const width of [768, 1280, 1920]) {
  const page = await browser.newPage({ viewport: { width, height: 800 }, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=console-board--ten-thousand-tasks&viewMode=story`);
  await page.waitForSelector("[role=listitem]", { timeout: 20000 });
  await page.waitForTimeout(300);
  check(`board ${width}: five columns side by side, no tab strip`, (await sideBySide(page)) && (await page.getByRole("tab").first().isVisible().catch(() => false)) === false, JSON.stringify(await columnBoxes(page)));
  const over = await pageOverflow(page);
  check(`board ${width}: the board fills the screen: the page does not scroll`, over.x <= 0 && over.y <= 1, JSON.stringify(over));
  const stats = { peak: 0 };
  const reached = [];
  for (const [i, name] of COLUMN_NAMES.entries()) if (await reachColumn(page, name, STATIC_LASTS[i], stats)) reached.push(i);
  const boxes = await columnBoxes(page);
  check(`board ${width}: every column is reachable and scrolls to its last task`, reached.join() === "0,1,2,3,4", `reached=${reached}`);
  check(`board ${width}: a column is 19rem wide`, boxes.every((c) => Math.abs(c.w - 304) <= 1), JSON.stringify(boxes));
  check(`board ${width}: the DOM stays bounded`, stats.peak < MAX_ROWS * 3, `peak=${stats.peak}`);
  const after = await pageOverflow(page);
  check(`board ${width}: scrolling the columns never scrolls the page`, after.x <= 0 && after.y <= 1 && (await page.evaluate(() => window.scrollY + window.scrollX)) === 0, JSON.stringify(after));
  check(`board ${width}: no page errors`, errors.length === 0, errors.join("|"));
  await page.close();
}

// paged: every column is its own server-paged list of 10,000; scrolling a column to its end loads pages until the last is on screen
for (const width of [768, 1280, 1920]) {
  const page = await browser.newPage({ viewport: { width, height: 800 }, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=console-board--paged-ten-thousand-per-column&viewMode=story`);
  await page.waitForSelector("[role=listitem]", { timeout: 20000 });
  await page.waitForTimeout(500);
  const stats = { peak: 0 };
  const reached = [];
  for (const name of COLUMN_NAMES) if (await reachColumn(page, name, /^PAY-10000$/, stats, 200)) reached.push(name);
  const loads = await loadsOf(page);
  check(`board ${width} paged: every column scrolls to its 10,000th task`, reached.length === COLUMN_NAMES.length, `reached=${reached} loads=${JSON.stringify(loads)}`);
  check(`board ${width} paged: every column asked for its own pages`, Object.values(loads).every((n) => n >= 49), JSON.stringify(loads));
  check(`board ${width} paged: the DOM stays bounded`, stats.peak < MAX_ROWS * 3, `peak=${stats.peak}`);
  check(`board ${width} paged: no page errors`, errors.length === 0, errors.join("|"));
  await page.close();
}

{
  // a column that is not scrolled to loads nothing beyond what it needs to fill itself
  const page = await browser.newPage({ viewport: { width: 1920, height: 900 }, deviceScaleFactor: 1 });
  await page.goto(`${base}/iframe.html?id=console-board--paged-ten-thousand-per-column&viewMode=story`);
  await page.waitForSelector("[role=listitem]", { timeout: 20000 });
  await page.waitForTimeout(800);
  const loads = await loadsOf(page);
  check("board 1920 paged: nothing loads before a column's end is near", Object.values(loads).every((n) => n <= 1), JSON.stringify(loads));
  await page.close();
}

// phones: one column per screen in a snapping strip; a header strip of the five columns jumps to one, a swipe moves the highlight
for (const [width, story, lasts] of [[375, "ten-thousand-tasks", STATIC_LASTS], [390, "paged-ten-thousand-per-column", STATIC_LASTS.map(() => /^PAY-10000$/)]]) {
  const page = await browser.newPage({ viewport: { width, height: 812 }, deviceScaleFactor: 1, hasTouch: true });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=console-board--${story}&viewMode=story`);
  await page.waitForSelector("[role=listitem]", { timeout: 20000 });
  await page.waitForTimeout(400);
  const tabs = page.getByRole("tab");
  const stripInfo = () => page.evaluate(() => { const s = document.querySelector("[role=region][aria-label=Board]"); const c = [...s.children].map((e) => Math.round(e.getBoundingClientRect().width)); return { snap: getComputedStyle(s).scrollSnapType, left: Math.round(s.scrollLeft), step: Math.round(s.children[1].getBoundingClientRect().left - s.children[0].getBoundingClientRect().left), widths: c, vw: innerWidth }; });
  const info = await stripInfo();
  check(`board ${width}: a header strip of five and a snapping strip of one-screen columns`, (await tabs.count()) === 5 && /x/.test(info.snap) && info.widths.every((w) => Math.abs(w - (width - 32)) <= 1), JSON.stringify(info));
  check(`board ${width}: Ready is open first`, (await tabs.nth(0).getAttribute("aria-selected")) === "true");
  const stats = { peak: 0 };
  const tapped = [];
  for (let t = 4; t >= 0; t--) {
    await tabs.nth(t).tap();
    await page.waitForTimeout(900);
    const now = await stripInfo();
    const selected = (await tabs.nth(t).getAttribute("aria-selected")) === "true" && (await page.locator("[role=tab][aria-selected=true]").count()) === 1;
    if (selected && Math.abs(now.left - t * now.step) <= 2 && (await reachColumn(page, COLUMN_NAMES[t], lasts[t], stats, 200))) tapped.push(t);
  }
  check(`board ${width}: tapping each header scrolls to its column, lights it, and the column scrolls to its last task`, tapped.sort().join() === "0,1,2,3,4", `tapped=${tapped}`);
  // a swipe: scroll the strip by a screen; the next header lights up
  await page.getByRole("tab").nth(0).tap();
  await page.waitForTimeout(900);
  const swiped = [];
  for (let t = 1; t < 5; t++) {
    await page.evaluate(() => { const s = document.querySelector("[role=region][aria-label=Board]"); s.scrollBy({ left: s.children[1].getBoundingClientRect().left - s.children[0].getBoundingClientRect().left, behavior: "instant" }); });
    await page.waitForTimeout(500);
    if ((await tabs.nth(t).getAttribute("aria-selected")) === "true") swiped.push(t);
  }
  check(`board ${width}: swiping moves the highlight from column to column`, swiped.join() === "1,2,3,4", `swiped=${swiped}`);
  const over = await pageOverflow(page);
  check(`board ${width}: no horizontal page scroll, and the page does not scroll down`, over.x <= 0 && over.y <= 1, JSON.stringify(over));
  check(`board ${width}: the DOM stays bounded`, stats.peak < MAX_ROWS * 3, `peak=${stats.peak}`);
  if (story.startsWith("paged")) {
    const loads = await loadsOf(page);
    check(`board ${width} paged: every column asked for its own pages`, Object.values(loads).every((n) => n >= 49), JSON.stringify(loads));
  }
  check(`board ${width}: no page errors`, errors.length === 0, errors.join("|"));
  await page.close();
}

await browser.close();
close();

// the page-scroll checks (no horizontal page scroll, no inner scroll panes) run on the same stories
const scroll = spawnSync(process.execPath, [fileURLToPath(new URL("./page-scroll.mjs", import.meta.url))], { stdio: "inherit" });
process.exit(failed || scroll.status ? 1 : 0);
