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

// the board's columns as laid out: one box per column section
const columnBoxes = (page) =>
  page.evaluate(() => [...document.querySelectorAll("[role=region][aria-label=Board] > section, [role=tabpanel] > section")].map((e) => { const r = e.getBoundingClientRect(); return { name: e.getAttribute("aria-label"), x: Math.round(r.left), y: Math.round(r.top + window.scrollY), w: Math.round(r.width) }; }));
// five columns on one row, left to right, none overlapping
const sideBySide = async (page) => {
  const b = await columnBoxes(page);
  return b.length === 5 && b.every((c) => c.y === b[0].y) && b.every((c, i) => i === 0 || c.x >= b[i - 1].x + b[i - 1].w);
};

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
  check("board 1920: all five columns side by side, no tab strip", (await sideBySide(page)) && (await page.getByRole("tablist").count()) === 0, JSON.stringify(await columnBoxes(page)));
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

// Narrower than five columns (1280, 768, 375): a tab strip and one column; every tab's column scrolls to its last
// task. A wrapping grid here would put a column below one that pages forever.
const TAB_LASTS = [/^PAY-9996$/, /^PAY-9997$/, /^PAY-9998$/, /^PAY-9999$/, /^PAY-10000$/];
for (const width of [1280, 768, 375]) {
  const page = await browser.newPage({ viewport: { width, height: width < 768 ? 812 : 800 }, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=console-board--ten-thousand-tasks&viewMode=story`);
  await page.waitForSelector("[role=listitem]", { timeout: 20000 });
  const tabs = page.getByRole("tab");
  check(`board ${width}: a tab strip of five and one column`, (await tabs.count()) === 5 && (await columnBoxes(page)).length === 1, JSON.stringify(await columnBoxes(page)));
  const reached = [];
  let peak = 0;
  for (let t = 0; t < 5; t++) {
    await tabs.nth(t).click();
    await page.waitForTimeout(300);
    const selected = (await tabs.nth(t).getAttribute("aria-selected")) === "true";
    for (let i = 0; i < 60; i++) {
      await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
      await page.waitForTimeout(100);
      peak = Math.max(peak, await page.locator("[role=listitem]").count());
      if (selected && (await page.locator("[role=tabpanel]").getByText(TAB_LASTS[t]).count()) > 0) {
        reached.push(t);
        break;
      }
    }
    // the tab strip stays in view at the bottom of a 2,000-card column
    const strip = await page.getByRole("tablist").boundingBox();
    if (!strip || strip.y < 0 || strip.y > 80) reached.push(`tabs off screen at ${strip?.y}`);
  }
  const sw = await page.evaluate(() => document.scrollingElement.scrollWidth - document.scrollingElement.clientWidth);
  check(`board ${width}: every tab's column scrolls to its last task`, reached.join() === "0,1,2,3,4", `reached=${reached}`);
  check(`board ${width}: the DOM stays bounded`, peak < MAX_ROWS * 2, `peak=${peak}`);
  check(`board ${width}: no horizontal page scroll`, sw <= 0, `overflow=${sw}px`);
  check(`board ${width}: no page errors`, errors.length === 0, errors.join("|"));
  await page.close();
}

{
  // 1280, paged: the last tab (Tossed) pages to its 10,000th task by scrolling the page, and nothing else loads
  const page = await browser.newPage({ viewport: desktop, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/iframe.html?id=console-board--paged-laptop&viewMode=story`);
  await page.waitForSelector("[role=listitem]", { timeout: 20000 });
  await page.getByRole("tab", { name: /^Tossed/ }).click();
  await page.waitForTimeout(300);
  let found = false;
  for (let i = 0; i < 200 && !found; i++) {
    await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
    await page.waitForTimeout(100);
    found = (await page.locator("[role=tabpanel]").getByText(/^PAY-10000$/).count()) > 0;
  }
  const loads = await loadsOf(page);
  check("board 1280 paged: Tossed scrolls to its 10,000th task", found && loads.cancelled >= 49, JSON.stringify(loads));
  check("board 1280 paged: no other column loads", loads.ready <= 1 && loads.claimed === 0 && loads.waiting === 0 && loads.done === 0, JSON.stringify(loads));
  check("board 1280 paged: no page errors", errors.length === 0, errors.join("|"));
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
