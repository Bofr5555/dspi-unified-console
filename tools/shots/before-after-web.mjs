// -- WP SHOW2 -- the web console before (0.14.0) and after (0.15.0) on the same screens of the core's demo DSPi:
// node before-after-web.mjs DIST OUT old|new  ->  OUT/<id>-<dark|light>.png at 1366x900 (tools/shots/before-after-web.sh)
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { chromium } from "playwright";
const [root, out, which] = process.argv.slice(2);
const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".wasm": "application/wasm", ".json": "application/json", ".svg": "image/svg+xml", ".png": "image/png", ".ico": "image/x-icon" };
const srv = http.createServer((q, r) => {
  let p = decodeURIComponent(new URL(q.url, "http://x").pathname);
  if (p.endsWith("/")) p += "index.html";
  const f = path.join(root, p);
  if (!f.startsWith(root) || !fs.existsSync(f)) { r.writeHead(404); return r.end(); }
  r.writeHead(200, { "content-type": types[path.extname(f)] ?? "application/octet-stream" });
  fs.createReadStream(f).pipe(r);
}).listen(0, "127.0.0.1");
await new Promise((ok) => srv.once("listening", ok));
const base = `http://127.0.0.1:${srv.address().port}/`;
// id: [[role, name], ...] clicked in order from the device's main view
const OLD = {
  main: [["tab", "Main"]], overview: [["tab", "Overview"]], eq: [["tab", "EQ"]], inputs: [["tab", "Inputs"]],
  matrix: [["tab", "Matrix"]], effects: [["tab", "Effects"]], measure: [["tab", "Measure"]],
  snapshots: [["tab", "Snapshots"]], files: [["tab", "Files"]], events: [["tab", "Events"]], system: [["tab", "System"]],
  device: [["tab", "Device"]], settings: [["tab", "Settings"]], spectrum: [["tab", "Analyser"]],
  presets: [["tab", "Main"], ["button", /^Recall 1/]],
};
const NEW = {
  main: [["button", "Overview"], ["tab", "Dashboard"]], overview: [["button", "Overview"], ["tab", "All Channels"]],
  eq: [["button", "Equaliser"]], inputs: [["button", "Equaliser"], ["tab", "Inputs"]], matrix: [["button", "Routing"]],
  effects: [["button", "Effects"]], measure: [["button", "Measure"]], snapshots: [["button", "Library"], ["tab", "Snapshots"]],
  files: [["button", "Library"], ["tab", "Files"]], events: [["button", "System"], ["tab", "Events"]],
  system: [["button", "System"]], device: [["button", "Devices"]], settings: [["button", "Settings"]],
  spectrum: [["button", "Analyse"]], presets: [["button", /^Presets:/]], history: [["button", "Library"], ["tab", "History"]],
  crossover: [["button", "Equaliser"], ["tab", "Crossover"]],
};
const plan = which === "old" ? OLD : NEW;
const b = await chromium.launch();
fs.mkdirSync(out, { recursive: true });
for (const scheme of ["dark", "light"]) {
  const page = await b.newPage({ viewport: { width: 1366, height: 900 }, colorScheme: scheme, locale: "en-GB" });
  await page.goto(base);
  await page.waitForTimeout(2500);
  await page.screenshot({ path: `${out}/start-${scheme}.png` });
  await page.getByRole("listitem").filter({ hasText: /Demo/ }).getByRole("button", { name: /Connect/ }).first().click();
  await page.waitForTimeout(3000);
  for (const [id, clicks] of Object.entries(plan)) {
    try {
      await page.keyboard.press("Escape");
      for (const [role, name] of clicks) {
        const el = page.getByRole(role, { name, exact: typeof name === "string" }).first();
        await el.click({ timeout: 4000 });
        await page.waitForTimeout(700);
      }
      await page.waitForTimeout(800);
      await page.screenshot({ path: `${out}/${id}-${scheme}.png` });
      console.log("ok", which, scheme, id);
    } catch (e) {
      console.log("MISS", which, scheme, id, String(e).split("\n")[0]);
    }
  }
  await page.close();
}
await b.close(); srv.close();
