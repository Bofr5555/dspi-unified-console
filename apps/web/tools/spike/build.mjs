// WP P5WCAP: builds the P5SPIKE page (development only; never part of the built console).
//
//   node tools/build.mjs && node tools/spike/build.mjs [outdir]    (default: dist-spike/)
//
// outdir is a copy of dist/ (the console, its Worker, capture-worklet.js and the core) with
// spike/measure-spike.html beside it. Serve it as a console from a hub, which fills in the
// page's hub address and gives Firefox its way to the DSPi:
//   uconsole web --console apps/web/dist-spike     then open /console/spike/measure-spike.html
// or, for USB in Chromium only: node tools/serve.mjs dist-spike, then /spike/measure-spike.html
// on 127.0.0.1 (a secure context, as the microphone needs).

import { build } from "esbuild";
import { cpSync, existsSync, mkdirSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "../..");
const dist = join(root, "dist");
const out = resolve(process.argv[2] ?? join(root, "dist-spike"));
for (const f of ["index.html", "worker.js", "capture-worklet.js"]) {
  if (!existsSync(join(dist, f))) {
    console.error(`spike: no ${f} in dist/: run node tools/build.mjs first`);
    process.exit(2);
  }
}
if (!existsSync(join(dist, "dspi_wasm.wasm"))) console.warn("spike: dist/ has no core module (DSPI_WASM_DIR): the page cannot open a DSPi");
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
cpSync(dist, out, { recursive: true, filter: (p) => !p.startsWith(join(dist, "test")) });
mkdirSync(join(out, "spike"), { recursive: true });
await build({
  entryPoints: { "measure-spike": join(here, "measure-spike.ts") },
  outdir: join(out, "spike"),
  absWorkingDir: root,
  bundle: true,
  format: "esm",
  target: ["chrome137", "firefox153"],
  sourcemap: true,
  logLevel: "warning",
});
cpSync(join(here, "measure-spike.html"), join(out, "spike/measure-spike.html"));
cpSync(join(here, "measure-spike.css"), join(out, "spike/measure-spike.css"));
console.log(`spike page built: ${join(out, "spike/measure-spike.html")}`);
