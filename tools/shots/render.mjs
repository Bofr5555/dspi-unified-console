// tools/shots/pty_shots.py's HTML screens to PNGs, a dark and a light terminal each, with
// Playwright's Chromium (in the Playwright image; apps/web's node_modules):
//
//   node render.mjs <html dir> <png dir> [prefix] [schemes, default dark,light]
//
// <png dir>/<prefix><name>-dark.png and -light.png.
import { readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(resolve("/src/apps/web/package.json"));
const { chromium } = require("@playwright/test");

const [src, out, prefix = "", schemes = "dark,light"] = process.argv.slice(2);
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 2000, height: 1200 }, deviceScaleFactor: 1 });
for (const f of readdirSync(src).filter((f) => f.endsWith(".html")).sort()) {
  const name = f.slice(0, -5);
  await page.goto(`file://${resolve(join(src, f))}`);
  for (const scheme of schemes.split(",")) {
    await page.evaluate((s) => document.documentElement.classList.toggle("light", s === "light"), scheme);
    await page.locator("pre").screenshot({ path: join(out, `${prefix}${name}-${scheme}.png`) });
  }
  console.log("rendered", name);
}
await browser.close();
