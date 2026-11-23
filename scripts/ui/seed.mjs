// Demo data for UI work: pot SHOP with tasks in every state, cooks that claim, block, ask and talk on the
// pass, pantry notes. Signs in through the dev Keycloak as admin; cooks act with their own tokens.
// Usage: node scripts/ui/seed.mjs   (once; it skips when pot SHOP exists)
import { createRequire } from "node:module";
import { execSync } from "node:child_process";
const require = createRequire(execSync("npm root -g").toString().trim() + "/");
const { chromium } = require("playwright");
const BASE = process.env.PUBLIC_URL || "http://localhost:41011";
const H = { "x-corntchap-console": "1", "content-type": "application/json", origin: BASE };

const b = await chromium.launch();
const page = await (await b.newContext()).newPage();
await page.goto(BASE + "/");
await page.waitForURL((u) => u.toString().includes("/realms/"), { timeout: 15000 }).catch(() => {});
if (page.url().includes("/realms/")) {
  await page.fill("#username", "admin");
  await page.fill("#password", "admin");
  await page.locator("#kc-login, [type=submit]").first().click();
}
await page.waitForSelector("#main", { timeout: 30000 });
const r = page.request;
const admin = async (method, path, data) => {
  const res = await r.fetch(BASE + "/api/v1" + path, { method, headers: H, data });
  if (!res.ok() && process.env.DEBUG) console.error(method, path, res.status(), (await res.text()).slice(0, 200));
  return res.json().catch(() => ({}));
};
const pots = await admin("GET", "/projects");
if (Array.isArray(pots) && pots.some((p) => p.key === "SHOP")) {
  console.log("SHOP exists: skipping");
  await b.close();
  process.exit(0);
}
await admin("POST", "/teams", { name: "kitchen" });
await admin("POST", "/teams/kitchen/members", { principal: "admin" });
await admin("POST", "/projects", { team: "kitchen", key: "SHOP", name: "Storefront" });
const tok = {};
for (const [name, kind, runtime] of [["mira", "claude-code", "local"], ["otto", "opencode", "local"], ["juno", "claude-code", "cloud"]]) {
  const a = await admin("POST", "/agents", { name, kind, runtime, max_leases: 3 });
  tok[name] = a.token;
  await admin("POST", "/teams/kitchen/members", { principal: name });
}
const as = async (who, method, path, data) => {
  const res = await r.fetch(BASE + "/api/v1" + path, { method, headers: { "content-type": "application/json", authorization: `Bearer ${tok[who]}` }, data });
  if (!res.ok() && process.env.DEBUG) console.error(who, method, path, res.status(), (await res.text()).slice(0, 200));
  return res.json().catch(() => ({}));
};
const mcp = (who, tool, args) =>
  r.post(BASE + "/mcp", {
    headers: { authorization: `Bearer ${tok[who]}`, "content-type": "application/json", accept: "application/json, text/event-stream", "MCP-Protocol-Version": "2026-07-28", "Mcp-Method": "tools/call", "Mcp-Name": tool },
    data: { jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: tool, arguments: args, _meta: { "io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {} } } },
  });

const tasks = [
  ["Checkout: show shipping cost before the payment step", ["checkout", "ui"], ["Design the summary row", "Fetch rates from the carrier API", "Write the e2e test"], 2],
  ["Cart badge does not update after removing an item", ["bug", "ui"], [], 1],
  ["Product search: typo tolerance with trigram index", ["search", "backend"], ["Add pg_trgm", "Rank by similarity", "Benchmark on 50k products"], 0],
  ["Invoice PDF uses the wrong VAT rate for Cameroon", ["bug", "billing"], [], 2],
  ["Wishlist page", ["ui"], ["List", "Empty state", "Share link"], 0],
  ["Migrate image uploads to object storage", ["backend", "infra"], ["Presigned uploads", "Backfill old images"], 1],
  ["Accessibility pass on the product page", ["a11y", "ui"], ["Contrast", "Focus order", "Alt texts"], 0],
  ["Order emails in French and English", ["i18n"], [], 0],
  ["Rate limit the login endpoint", ["security", "backend"], [], 1],
  ["Dark mode for the storefront", ["ui"], [], 0],
];
for (const [title, labels, items, priority] of tasks) await admin("POST", "/projects/SHOP/tasks", { title, labels, items, priority, body: "" });
// mira cooks SHOP-1 and ticks an item; otto cooks SHOP-3; SHOP-4 waits; SHOP-2 and SHOP-8 are served
const claim = async (who, id) => (await as(who, "POST", `/tasks/${id}/claim`, {}))?.lease?.epoch;
const e1 = await claim("mira", "SHOP-1");
await as("mira", "POST", "/tasks/SHOP-1/check", { position: 1, done: true, epoch: e1 });
await claim("otto", "SHOP-3");
const e2 = await claim("otto", "SHOP-2");
await as("otto", "POST", "/tasks/SHOP-2/complete", { epoch: e2, result: "Badge reads from the cart store now; added a regression test." });
const e8 = await claim("mira", "SHOP-8");
await as("mira", "POST", "/tasks/SHOP-8/complete", { epoch: e8, result: "Templates in fr and en, picked from the customer's locale." });
const e4 = await claim("juno", "SHOP-4");
await as("juno", "POST", "/tasks/SHOP-4/block", { epoch: e4, reason: "Waiting for the accountant: is the 19.25% rate right for services?" });
await as("mira", "POST", "/tasks/SHOP-1/comment", { body: "Carrier API returns rates in XAF only; converting on our side." });
await admin("POST", "/tasks/SHOP-1/comment", { body: "Fine by me, show both currencies if the customer picked EUR." });
await as("otto", "POST", "/projects/SHOP/questions", { question: "Should typo tolerance apply to SKUs too, or only to product names?", options: ["Names only", "Names and SKUs"] });
await as("mira", "POST", "/projects/SHOP/questions", { question: "Which carrier is the default when the customer has no address yet?" });
await as("mira", "POST", "/projects/SHOP/pantry", { kind: "convention", title: "Prices are integers in XAF", body: "Store prices as integer XAF. Convert only for display." });
await as("otto", "POST", "/projects/SHOP/pantry", { kind: "gotcha", title: "The search index rebuild locks the table", body: "Run `reindex concurrently` in prod, never a plain reindex." });
await admin("POST", "/projects/SHOP/pantry", { kind: "decision", title: "Checkout stays one page", body: "No multi-step checkout: one page, sections collapse on mobile." });
await mcp("mira", "pass", { project: "SHOP", message: "Starting on the shipping summary. @otto are you touching the cart store today?" });
await mcp("otto", "pass", { project: "SHOP", message: "Only the badge fix, merged already. Go ahead." });
await mcp("juno", "pass", { project: "SHOP", message: "SHOP-4 is parked until the accountant answers the VAT question." });
console.log("seeded SHOP");
await b.close();
