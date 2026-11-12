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
//
// -- WP WB7 -- Build settings, from the environment:
//   DSPI_WEB_SOURCE_URL   the link to this build's source that About shows (GPL);
//                         an embedder's mount({sourceUrl}) overrides it
//   DSPI_WEB_RELEASE=1    a build for publishing: refuses to build without the
//                         source link
// dist/ also gets page.css (the standalone page's frame), the page's icons (icons/:
// favicon.ico, favicon.svg, apple-touch-icon.png) and THIRD-PARTY-NOTICES.txt:
// Preact's licence from node_modules and the core's Rust crates
// (notices/rust-crates.txt, tools/gen_rust_notices.py).

import { build } from "esbuild";
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = new Set(process.argv.slice(2));
// -- WP WB7 --
const sourceUrl = (process.env.DSPI_WEB_SOURCE_URL ?? "").trim();
if (sourceUrl && !/^https?:\/\//.test(sourceUrl)) {
  console.error(`build: DSPI_WEB_SOURCE_URL must be an http(s) address, not "${sourceUrl}"`);
  process.exit(2);
}
const common = {
  define: { __DSPI_SOURCE_URL__: JSON.stringify(sourceUrl) },
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
  // -- WP WB7 --
  cpSync(join(root, "src/page.css"), join(out, "page.css"));
  // -- job ICON1 -- the standalone page's icons (assets/icon/render.sh makes them)
  cpSync(join(root, "icons"), out, { recursive: true });
  writeFileSync(join(out, "THIRD-PARTY-NOTICES.txt"), notices());
  // -- WP CLEAN1 fixup cl1 -- where this build's source is, for whoever serves it
  writeFileSync(join(out, "SOURCE.txt"), sourceNotice());
  if (!sourceUrl) {
    if (process.env.DSPI_WEB_RELEASE === "1") {
      console.error("build: a release needs DSPI_WEB_SOURCE_URL, the link to its source (GPL)");
      process.exit(2);
    }
    console.warn("build: no DSPI_WEB_SOURCE_URL: About shows no source link (set it before publishing)");
  }
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

// -- WP LIC1 --
/** The project's licence statement and its section 7 terms, from assets/about.json, as paragraphs. */
function projectLicence() {
  const l = JSON.parse(readFileSync(join(root, "../../assets/about.json"), "utf8")).licence;
  const out = [...l.copyright, "", l.statement, "", l.terms_title, "", l.terms_note, ""];
  l.terms.forEach((t, i) => out.push(`${i + 1}. ${t.title}. ${t.text}`, ""));
  out.push(l.freedoms);
  return out;
}

// -- WP WB7 --
/** dist/THIRD-PARTY-NOTICES.txt: the page's JavaScript (Preact) and the core's Rust crates. */
// -- WP CLEAN1 fixup cl1 -- SOURCE.txt (DRAFT wording, pending the owner's approval):
// the web console reaches people from whatever serves it, a hub or a web server, so
// the notice says where the source is and that it comes with the program.
function sourceNotice() {
  const version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
  const where = sourceUrl
    ? `It is at ${sourceUrl}`
    : "This build names no address for it (DSPI_WEB_SOURCE_URL was not set)";
  return [
    "DRAFT notice, pending the owner's approval.",
    "",
    `DSPi uConsole's web console ${version}. The program as a whole is distributed`,
    "under the GNU General Public License version 3; Bofr5555's own additions are",
    "licensed under version 3 or later, with additional terms (About > Licence).",
    "",
    "Its complete source code is published with each release of DSPi uConsole as",
    `dspi-uconsole-${version}-source.tar.gz. ${where}.`,
    "Whoever serves this console to you, a DSPi Link hub or any web server, passes",
    "the program on to you, and with it your right to that source.",
    "",
    "The licences of the parts it contains: THIRD-PARTY-NOTICES.txt.",
    "",
  ].join("\n");
}

function notices() {
  const preactDir = join(root, "node_modules/preact");
  const preact = JSON.parse(readFileSync(join(preactDir, "package.json"), "utf8"));
  const licence = readFileSync(join(preactDir, "LICENSE"), "utf8").trim();
  const rust = readFileSync(join(root, "notices/rust-crates.txt"), "utf8");
  return [
    "DSPi web console: licence and third-party notices",
    "",
    // -- WP LIC1 -- the console's own licence and its additional terms, as About shows them
    ...projectLicence(),
    "",
    "=".repeat(72),
    "The console bundles the parts below, each under its own licence.",
    "",
    "=".repeat(72),
    `preact ${preact.version} — ${preact.license}`,
    "https://github.com/preactjs/preact",
    "",
    "--- LICENSE ---",
    licence,
    "",
    "",
    rust,
  ].join("\n");
}
