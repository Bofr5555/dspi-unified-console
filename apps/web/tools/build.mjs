// Builds the web console (WP WB5) with esbuild. No dev server, no plugins.
//
//   node tools/build.mjs           dist/: the standalone page, the library and the Worker
//   node tools/build.mjs --unit    dist-test/unit/: the unit tests, for `node --test`
//   node tools/build.mjs --pages   dist/test/: the e2e tests' own pages (embedding)
//
// dist/ is what a host serves: index.html, app.js (standalone), dspi-web.js (the
// library), worker.js (one per session), dspi-web.css.

import { build } from "esbuild";
import { cpSync, mkdirSync, readdirSync, rmSync } from "node:fs";
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
  if (args.has("--pages")) {
    cpSync(join(root, "test/pages"), join(out, "test"), { recursive: true, filter: (p) => !p.endsWith(".ts") });
    await build({ ...common, entryPoints: { "test/embed": "test/pages/embed.ts" }, outdir: out, minify: false });
  }
  console.log("built dist/");
}
