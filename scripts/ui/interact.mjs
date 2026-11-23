// Drive one console page on a device and screenshot after a few steps.
// Usage: node scripts/ui/interact.mjs <device: phone|tablet|desktop> <path> <out.png> [step ...]
//   steps: "click:<css or text=...>", "fill:<css>=<value>", "wait:<ms>", "press:<key>", "dark"
import { createRequire } from "node:module";
import { execSync } from "node:child_process";
const require = createRequire(execSync("npm root -g").toString().trim() + "/");
const { chromium, devices } = require("playwright");
const BASE = process.env.PUBLIC_URL || "http://localhost:41011";
// pages load from CONSOLE_URL when set (a Vite dev server proxying to the server, see scripts/ui/vite.sh);
// sign-in always goes through the server, and its session cookie is shared across localhost ports
const APP = process.env.CONSOLE_URL || BASE;
const [device, path, out, ...steps] = process.argv.slice(2);
const opts = { phone: devices["iPhone 13"], tablet: devices["iPad (gen 7)"], desktop: { viewport: { width: 1440, height: 900 } } }[device];
const b = await chromium.launch();
const ctx = await b.newContext({ ...opts, colorScheme: steps.includes("dark") ? "dark" : "light" });
const page = await ctx.newPage();
await page.goto(BASE + "/");
await page.waitForURL((u) => u.toString().includes("/realms/"), { timeout: 15000 }).catch(() => {});
if (page.url().includes("/realms/")) {
  await page.fill("#username", process.env.KC_USER || "admin");
  await page.fill("#password", process.env.KC_PASS || "admin");
  await page.locator("#kc-login, [type=submit]").first().click();
}
await page.waitForSelector("#main", { timeout: 30000 });
await page.goto(APP + path);
// server-rendered pages (sign-in, device approval, OAuth consent) have no console shell
await page.waitForSelector("#main", { timeout: 5000 }).catch(() => {});
await page.waitForTimeout(1200);
for (const s of steps) {
  const [k, ...rest] = s.split(":");
  const v = rest.join(":");
  if (k === "click") await page.locator(v).first().click();
  else if (k === "fill") { const [sel, val] = v.split("="); await page.locator(sel).first().fill(val); }
  else if (k === "wait") await page.waitForTimeout(Number(v));
  else if (k === "press") await page.keyboard.press(v);
  await page.waitForTimeout(500);
}
await page.screenshot({ path: out });
console.log(out);
await b.close();
