// WP WB5: the e2e tests, in the official Playwright image (apps/web/tools/ci.sh).
import { defineConfig, devices } from "@playwright/test";

const port = Number(process.env.PORT ?? 8737);
export default defineConfig({
  testDir: "test/e2e",
  outputDir: process.env.PW_OUTPUT_DIR ?? "test-results",
  timeout: 30_000,
  retries: 0,
  reporter: [["list"]],
  use: { baseURL: `http://127.0.0.1:${port}/`, trace: "retain-on-failure" },
  webServer: { command: `node tools/serve.mjs dist ${port}`, url: `http://127.0.0.1:${port}/`, reuseExistingServer: false },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "firefox", use: { ...devices["Desktop Firefox"] } },
    { name: "narrow", use: { ...devices["Desktop Chrome"], viewport: { width: 375, height: 800 } } },
  ],
});
