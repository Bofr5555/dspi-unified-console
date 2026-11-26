// WP WB5: the e2e tests, in the official Playwright image (apps/web/tools/ci.sh).
import { defineConfig, devices } from "@playwright/test";
// -- WP P5WCAP -- the browsers' fake microphone (test/e2e/capture_p5wcap.spec.ts): Chromium's
// plays P5SIM's rendering (test/fixtures/p5wcap-sim.wav, looped), Firefox's is its own tone.
// Chromium's permission is the test context's to grant or not; Firefox's prompt is off (Playwright
// cannot grant a microphone there), so its denial is a unit test. The spec runs the full Chromium
// headless (`channel`), as the headless shell has no getUserMedia.
import { fileURLToPath } from "node:url";
const fakeMicWav = fileURLToPath(new URL("./test/fixtures/p5wcap-sim.wav", import.meta.url));
const chromiumFakeMic = { args: ["--use-fake-device-for-media-stream", `--use-file-for-fake-audio-capture=${fakeMicWav}`] };
const firefoxFakeMic = { firefoxUserPrefs: { "media.navigator.streams.fake": true, "media.navigator.permission.disabled": true } };

const port = Number(process.env.PORT ?? 8737);
// -- WP WB4b -- the page served by a hub (tools/hub-host.mjs over dspi-link's fake
// hub), for test/e2e/link.spec.ts; without DSPI_FAKEHUB those tests are skipped
export const hubPort = Number(process.env.HUB_PORT ?? 8738);
const fakehub = process.env.DSPI_FAKEHUB ?? "";
// -- WP WB7 -- the real hub serving the console (test/e2e/hosting.spec.ts) is `uconsole`
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
  // -- WP UB12 -- the pages' demo is the blank capture these tests were written for (src/ui/App.tsx's
  // BLANK_DEMO_KEY); test/e2e/demo_ub12.spec.ts clears it for the tuned example every page opens
  use: {
    baseURL: `http://127.0.0.1:${port}/`,
    trace: "retain-on-failure",
    storageState: {
      cookies: [],
      origins: [port, hubPort].map((p) => ({ origin: `http://127.0.0.1:${p}`, localStorage: [{ name: "dspi.test.blankDemo", value: "1" }] })),
    },
  },
  webServer: [
    { command: `node tools/serve.mjs dist ${port}`, url: `http://127.0.0.1:${port}/`, reuseExistingServer: false },
    ...(fakehub
      ? [{ command: `node tools/hub-host.mjs dist ${hubPort} ${fakehub}`, url: `http://127.0.0.1:${hubPort}/`, reuseExistingServer: false }]
      : []),
  ],
  projects: [
    // -- WP P5WCAP -- launchOptions: the fake microphone
    { name: "chromium", use: { ...devices["Desktop Chrome"], launchOptions: chromiumFakeMic } },
    { name: "firefox", use: { ...devices["Desktop Firefox"], launchOptions: firefoxFakeMic } },
    { name: "narrow", use: { ...devices["Desktop Chrome"], viewport: { width: 375, height: 800 }, launchOptions: chromiumFakeMic } },
  ],
});
