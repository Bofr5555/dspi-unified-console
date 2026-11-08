// Runs the unit tests (WP WB3): `node --test dist-test/unit/`, with
// `--experimental-wasm-jspi` on a Node whose JSPI is still behind that flag
// (Node 24); Node 26 has it on and refuses the flag.
import { spawnSync } from "node:child_process";

const flags = typeof WebAssembly.Suspending === "function" ? [] : ["--experimental-wasm-jspi"];
const r = spawnSync(process.execPath, [...flags, "--test", "dist-test/unit/"], { stdio: "inherit" });
process.exit(r.status ?? 1);
