// Render the logo to PNG app icons (PWA manifest, Apple touch icon, Tauri's `tauri icon` source).
// Usage: node scripts/ui/icons.mjs   (from the repo root)
import fs from "node:fs";
import { createRequire } from "node:module";
import { execSync } from "node:child_process";
const require = createRequire(execSync("npm root -g").toString().trim() + "/");
const { chromium } = require("playwright");

const svg = fs.readFileSync("console/public/favicon.svg", "utf8");
// the mark without the rounded tile, on a full-bleed square: platforms apply their own mask
const inner = svg.replace(/<rect[^>]*\/>/, "").replace(/^<svg[^>]*>/, "").replace(/<\/svg>$/, "");
const page = (size, pad) => `<html><body style="margin:0"><svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="${-pad} ${-pad} ${24 + 2 * pad} ${24 + 2 * pad}"><rect x="${-pad}" y="${-pad}" width="${24 + 2 * pad}" height="${24 + 2 * pad}" fill="#15183a"/>${inner}</svg></body></html>`;
const out = [
  ["console/public/apple-touch-icon.png", 180, 1.5],
  ["console/public/icons/icon-192.png", 192, 1.5],
  ["console/public/icons/icon-512.png", 512, 1.5],
  ["console/public/icons/maskable-512.png", 512, 5],
  ["console/public/icons/app-icon-1024.png", 1024, 2.5],
];
const b = await chromium.launch();
const p = await b.newPage();
for (const [file, size, pad] of out) {
  await p.setViewportSize({ width: size, height: size });
  await p.setContent(page(size, pad));
  await p.screenshot({ path: file, omitBackground: false, clip: { x: 0, y: 0, width: size, height: size } });
  console.log(file);
}
await b.close();
