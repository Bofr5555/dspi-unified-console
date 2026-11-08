// Builds the web console (WP WB5) with esbuild. No dev server, no plugins.
//
//   node tools/build.mjs           dist/: the standalone page, the library and the Worker
//   node tools/build.mjs --unit    dist-test/unit/: the unit tests, for `node --test`
//   node tools/build.mjs --pages   dist/test/: the e2e tests' own pages (embedding)
//
// dist/ is what a host serves: index.html, app.js (standalone), dspi-web.js (the
// library), worker.js (one per session), dspi_wasm.wasm (the core, WB3: built by
// crates/dspi-wasm/tools/build.sh and taken from $DSPI_WASM_DIR), dspi-web.css.
// --pages adds test/stub-worker.js, a Worker on the tests' fake core, and (WB4a)
// test/usb.html over a WebUSB mock, with dspi_wasm_test.wasm as the mock's engine.

import { build } from "esbuild";
import { cpSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = new Set(process.argv.slice(2));
const common = {
  absWorkingDir: root,
  bundle: true,
  format: "esm",
  target: ["chrome137", "firefox153", "safari18"],
  jsx: "automatic",
  jsxImportSource: "preact",
  sourcemap: true,
  logLevel: "warning",
};

if (args.has("--unit")) {
  const out = join(root, "dist-test/unit");
  rmSync(out, { recursive: true, force: true });
  const entries = readdirSync(join(root, "test/unit"))
    .filter((f) => f.endsWith(".test.ts"))
    .map((f) => join("test/unit", f));
  await build({ ...common, entryPoints: entries, outdir: out, platform: "node", outExtension: { ".js": ".mjs" }, target: "node24" });
  console.log(`unit tests built: ${entries.length}`);
} else {
  const out = join(root, "dist");
  // Empty it, not remove it: in CI it is a mount point.
  mkdirSync(out, { recursive: true });
  for (const f of readdirSync(out)) rmSync(join(out, f), { recursive: true, force: true });
  await build({
    ...common,
    entryPoints: {
      app: "src/standalone.ts",
      "dspi-web": "src/index.tsx",
      worker: "src/core/worker.ts",
    },
    outdir: out,
    minify: true,
  });
  cpSync(join(root, "index.html"), join(out, "index.html"));
  cpSync(join(root, "src/dspi-web.css"), join(out, "dspi-web.css"));
  // -- WP WB3 -- the core
  const wasm = process.env.DSPI_WASM_DIR ? join(process.env.DSPI_WASM_DIR, "dspi_wasm.wasm") : "";
  if (wasm && existsSync(wasm)) cpSync(wasm, join(out, "dspi_wasm.wasm"));
  else console.warn("build: no core module (DSPI_WASM_DIR): the page will say the core could not be loaded");
  if (args.has("--pages")) {
    cpSync(join(root, "test/pages"), join(out, "test"), { recursive: true, filter: (p) => !p.endsWith(".ts") });
    await build({
      ...common,
      entryPoints: {
        "test/embed": "test/pages/embed.ts",
        "test/stub-worker": "test/pages/stub-worker.ts",
        // -- WP WB4a -- the USB page over a WebUSB mock
        "test/usb": "test/pages/usb.ts",
        "test/usb-worker": "test/pages/usb-worker.ts",
        // -- WP WB4b -- a fleet server's page with hubs
        "test/fleet-hub": "test/pages/fleet-hub.ts",
      },
      outdir: out,
      minify: false,
    });
    // -- WP WB4a -- the mock DSPis' engine (crates/dspi-wasm's test module), and
    // the core beside test/usb-worker.js, which loads it from its own directory
    const testWasm = process.env.DSPI_WASM_DIR ? join(process.env.DSPI_WASM_DIR, "dspi_wasm_test.wasm") : "";
    if (testWasm && existsSync(testWasm)) cpSync(testWasm, join(out, "test/dspi_wasm_test.wasm"));
    if (wasm && existsSync(wasm)) cpSync(wasm, join(out, "test/dspi_wasm.wasm"));
  }
  console.log("built dist/");
}
