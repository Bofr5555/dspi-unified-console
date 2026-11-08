// WP WB5: the e2e tests, in the official Playwright image (apps/web/tools/ci.sh).
import { defineConfig, devices } from "@playwright/test";

const port = Number(process.env.PORT ?? 8737);
// -- WP WB4b -- the page served by a hub (tools/hub-host.mjs over dspi-link's fake
// hub), for test/e2e/link.spec.ts; without DSPI_FAKEHUB those tests are skipped
export const hubPort = Number(process.env.HUB_PORT ?? 8738);
const fakehub = process.env.DSPI_FAKEHUB ?? "";
// -- WP WB7 -- the real hub serving the console (test/e2e/hosting.spec.ts) is `dspi`
// (DSPI_TERM); without it those tests are skipped. -- WP HB9 -- Each of those tests' groups
// starts its own hubs on free ports, so parallel workers and `--repeat-each` copies never
// share one.
export default defineConfig({
  testDir: "test/e2e",
  outputDir: process.env.PW_OUTPUT_DIR ?? "test-results",
  timeout: 30_000,
  // -- WP HB9 -- 4 at once: at Playwright's default (one per core) a loaded build host
  // crashed browsers and reset hubs. `PW_ARGS="--workers N"` (tools/ci.sh) overrides it.
  workers: 4,
  retries: 0,
  reporter: [["list"]],
  use: { baseURL: `http://127.0.0.1:${port}/`, trace: "retain-on-failure" },
  webServer: [
    { command: `node tools/serve.mjs dist ${port}`, url: `http://127.0.0.1:${port}/`, reuseExistingServer: false },
    ...(fakehub
      ? [{ command: `node tools/hub-host.mjs dist ${hubPort} ${fakehub}`, url: `http://127.0.0.1:${hubPort}/`, reuseExistingServer: false }]
      : []),
  ],
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "firefox", use: { ...devices["Desktop Firefox"] } },
    { name: "narrow", use: { ...devices["Desktop Chrome"], viewport: { width: 375, height: 800 } } },
  ],
});
