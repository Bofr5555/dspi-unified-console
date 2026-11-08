// WP WB5: the e2e tests, in the official Playwright image (apps/web/tools/ci.sh).
import { defineConfig, devices } from "@playwright/test";

const port = Number(process.env.PORT ?? 8737);
// -- WP WB4b -- the page served by a hub (tools/hub-host.mjs over dspi-link's fake
// hub), for test/e2e/link.spec.ts; without DSPI_FAKEHUB those tests are skipped
export const hubPort = Number(process.env.HUB_PORT ?? 8738);
const fakehub = process.env.DSPI_FAKEHUB ?? "";
// -- WP WB7 -- the real hub serving the console (test/e2e/hosting.spec.ts): `dspi
// hub --web` with the demo device, and `dspi web` (a loopback hub, the page a secure
// context); without DSPI_TERM those tests are skipped
export const dspiHubPort = Number(process.env.DSPI_HUB_PORT ?? 8739);
export const dspiWebPort = Number(process.env.DSPI_WEB_PORT ?? 8740);
const term = process.env.DSPI_TERM ?? "";
const store = (name: string) => `${process.env.TMPDIR ?? "test-results"}/${name}`;
const hubFlags = "--console dist --open --no-keep-awake --forget-clients";
export default defineConfig({
  testDir: "test/e2e",
  outputDir: process.env.PW_OUTPUT_DIR ?? "test-results",
  timeout: 30_000,
  retries: 0,
  reporter: [["list"]],
  use: { baseURL: `http://127.0.0.1:${port}/`, trace: "retain-on-failure" },
  webServer: [
    { command: `node tools/serve.mjs dist ${port}`, url: `http://127.0.0.1:${port}/`, reuseExistingServer: false },
    ...(fakehub
      ? [{ command: `node tools/hub-host.mjs dist ${hubPort} ${fakehub}`, url: `http://127.0.0.1:${hubPort}/`, reuseExistingServer: false }]
      : []),
    ...(term
      ? [
          {
            command: `${term} --demo hub --web --loopback --port ${dspiHubPort} ${hubFlags} --store ${store("wb7-hub")} --name "WB7 hub"`,
            url: `http://127.0.0.1:${dspiHubPort}/console/`,
            reuseExistingServer: false,
          },
          {
            command: `${term} --demo web --port ${dspiWebPort} ${hubFlags} --store ${store("wb7-web")} --name "WB7 local"`,
            url: `http://127.0.0.1:${dspiWebPort}/console/`,
            reuseExistingServer: false,
          },
        ]
      : []),
  ],
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "firefox", use: { ...devices["Desktop Firefox"] } },
    { name: "narrow", use: { ...devices["Desktop Chrome"], viewport: { width: 375, height: 800 } } },
  ],
});
