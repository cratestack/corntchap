// Screenshots of the console at phone, tablet and desktop sizes, signed in through the dev Keycloak.
// Usage: node scripts/ui/shots.mjs [outDir] [path ...]   (server from scripts/ui/devstack.sh; seeds demo data once)
import fs from "node:fs";
import { createRequire } from "node:module";
import { execSync } from "node:child_process";

// the pre-installed Playwright lives in the global node_modules
const require = createRequire(execSync("npm root -g").toString().trim() + "/");
const { chromium, devices } = require("playwright");

const BASE = process.env.PUBLIC_URL || "http://localhost:41011";
// pages load from CONSOLE_URL when set (a Vite dev server proxying to the server, see scripts/ui/vite.sh);
// sign-in always goes through the server, and its session cookie is shared across localhost ports
const APP = process.env.CONSOLE_URL || BASE;
const out = process.argv[2] || "/tmp/corntchap-ui/shots";
const paths = process.argv.slice(3);
fs.mkdirSync(out, { recursive: true });
const H = { "x-corntchap-console": "1", "content-type": "application/json" };

async function signIn(ctx) {
  const page = await ctx.newPage();
  await page.goto(BASE + "/");
  // the console loads, finds no session and redirects to the IdP
  await page.waitForURL((u) => u.toString().includes("/realms/"), { timeout: 15000 }).catch(() => {});
  if (page.url().includes("/realms/")) {
    await page.fill("#username", process.env.KC_USER || "admin");
    await page.fill("#password", process.env.KC_PASS || "admin");
    await page.click("#kc-login");
  }
  await page.waitForURL((u) => u.toString().startsWith(BASE) && !u.toString().includes("/auth/"), { timeout: 30000 });
  // the console keeps a live event stream open, so "networkidle" never comes: wait for the shell
  await page.waitForSelector("#main", { timeout: 30000 });
  return page;
}

async function seed(page) {
  const r = page.request;
  const post = (p, body) => r.post(BASE + "/api/v1" + p, { headers: H, data: body });
  const pots = await (await r.get(BASE + "/api/v1/projects", { headers: H })).json().catch(() => []);
  if (Array.isArray(pots) && pots.some((p) => p.key === "PAY")) return;
  await post("/teams", { name: "acme" });
  await post("/teams/acme/members", { principal: "admin" });
  await post("/projects", { team: "acme", key: "PAY", name: "Payments" });
  await post("/projects", { team: "acme", key: "WEB", name: "Website relaunch" });
  for (const [name, kind, runtime] of [["rita", "claude-code", "cloud"], ["lou", "opencode", "local"], ["athena", "claude-code", "local"]]) {
    await post("/agents", { name, kind, runtime });
    await post("/teams/acme/members", { principal: name });
  }
  const titles = ["Refund screen with partial amounts", "Webhook retries with exponential backoff", "Ledger export to CSV",
    "Fix rounding in VAT totals", "Card tokenisation via the new vault", "Dispute evidence upload", "Payout schedule settings",
    "Audit log for admin actions"];
  for (const [i, t] of titles.entries())
    await post("/projects/PAY/tasks", { title: t, labels: i % 3 === 0 ? ["backend"] : ["ui"], items: i % 2 ? ["design", "build", "test"] : [] });
  await post("/projects/WEB/tasks", { title: "New landing hero" });
}

const sizes = [
  ["phone", { ...devices["iPhone 13"] }],
  ["tablet", { ...devices["iPad (gen 7)"] }],
  ["desktop", { viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 }],
];
const want = paths.length ? paths : ["/pots/PAY/board", "/pots/PAY/pass", "/pots/PAY/questions", "/pots/PAY/pantry", "/cooks", "/fires", "/teams"];
const browser = await chromium.launch();
let seeded = false;
for (const [name, opts] of sizes) {
  const ctx = await browser.newContext({ ...opts, colorScheme: process.env.SCHEME || "light" });
  const page = await signIn(ctx);
  if (!seeded) { await seed(page); seeded = true; }
  for (const p of want) {
    await page.goto(APP + p);
    await page.waitForSelector("#main", { timeout: 15000 }).catch(() => {});
    await page.waitForTimeout(1500);
    const file = `${out}/${name}${p.replace(/[/?=&]+/g, "_")}.png`;
    await page.screenshot({ path: file, fullPage: false });
    console.log(file);
  }
  await ctx.close();
}
await browser.close();
