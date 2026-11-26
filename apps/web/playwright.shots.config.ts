// The screenshot specs (test/shots/*.spec.ts), not a test suite: pictures of the console on
// the core's demo device and on `uconsole --demo hub --alink`, dark and light, at a desktop
// width and at 375 px. Run through tools/shots/web.sh, which builds with apps/web/tools/ci.sh
// --no-e2e and then runs this config alone in the same Playwright image, on the specs PW_ARGS
// names (-- INT016 -- by default parity_web.spec.ts; the README's are readme_web.spec.ts, and the
// old tour.spec.ts is gone):
//
//   PW_ARGS="test/shots/<spec>.ts --project desktop" tools/shots/web.sh <out dir>
//
// The PNGs go to $SHOTS_DIR (web.sh sets it to <out dir>; default test-results/shots).
import { defineConfig, devices } from "@playwright/test";

const port = Number(process.env.PORT ?? 8737);

export default defineConfig({
  testDir: "test/shots",
  outputDir: process.env.PW_OUTPUT_DIR ?? "test-results",
  timeout: 120_000,
  workers: 2,
  retries: 0,
  reporter: [["list"]],
  use: { baseURL: `http://127.0.0.1:${port}/`, trace: "off" },
  webServer: [{ command: `node tools/serve.mjs dist ${port}`, url: `http://127.0.0.1:${port}/`, reuseExistingServer: false }],
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"], viewport: { width: 1366, height: 900 } } },
    { name: "narrow", use: { ...devices["Desktop Chrome"], viewport: { width: 375, height: 800 } } },
  ],
});
