// WP WB7 e2e: the console reaches the demo device through each hosting route.
//
//   1. `dspi hub --web`: the real hub (DSPI_TERM, the Terminal's binary) serves the
//      console at /console/ beside its own page at /, which stays byte for byte the
//      page it was; the console knows its hub from the <meta> the hub fills in and
//      reaches the hub's demo DSPi over Link, under the hub's policy (CSP).
//   2. `dspi web`: a hub on loopback serving the console on http://localhost, a
//      secure context, so one page holds a DSPi on USB (the WebUSB mock, Chromium)
//      and the hub's demo over Link at once.
//   3. The static build (tools/serve.mjs, any static host): the core's own demo,
//      and the third-party notices beside it.
//
// The Origin rule is the hub's, unchanged: a page from anywhere else is refused.

import { readFileSync } from "node:fs";
import { expect, test, type Page, type TestInfo } from "@playwright/test";

const linkWords = (await import("../../../../ci/generated/web/link-words.json", { with: { type: "json" } })).default as {
  link: { key: string; text: string; action: string | null }[];
};
const words = (key: string) => linkWords.link.find((w) => w.key === key)!;

const HUB = `http://127.0.0.1:${Number(process.env.DSPI_HUB_PORT ?? 8739)}`;
const LOCAL_PORT = Number(process.env.DSPI_WEB_PORT ?? 8740);
const LOCAL = `http://localhost:${LOCAL_PORT}`;
const PLAIN = `http://127.0.0.1:${Number(process.env.PORT ?? 8737)}`;
/** The core's simulated DSPi (FakeDevice::demo), which `dspi --demo hub` shares. */
const DEMO = "DEMO0000DSPI0001";
const PLACEHOLDER = readFileSync(new URL("../../../../crates/dspi-hub/assets/web/index.html", import.meta.url));

async function noPageErrors(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    // A refused WebSocket is logged by the browser; the Origin test causes one.
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

const picker = (page: Page) => page.getByRole("navigation", { name: "DSPi devices" });

/** Connect the hub's entry, choose the demo DSPi from its list, and see it read. */
async function openHubDemo(page: Page, host: string) {
  const row = picker(page).getByRole("listitem").filter({ hasText: `Hub ${host}` });
  await expect(row).toHaveCount(1);
  await row.getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  await expect(session.locator(".banner")).toContainText(words("choose_device").text);
  const entry = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: DEMO.slice(-8) });
  await entry.getByRole("button", { name: "Open" }).click();
  await expect(session.locator(".device-meta")).toContainText(DEMO);
  await expect(session.locator(".banner")).toHaveCount(0);
  return session;
}

/** A write with read-back, in one project only: the hub's demo DSPi is shared by all. */
async function writeOnce(session: ReturnType<Page["getByRole"]>, info: TestInfo, db: string) {
  if (info.project.name !== "chromium") return;
  const slider = session.getByRole("slider", { name: "Volume" });
  await slider.fill(db);
  await slider.dispatchEvent("change");
  await expect(session.locator(".volume-value")).toHaveText(`${Number(db).toFixed(1)} dB`);
}

if (!process.env.DSPI_TERM) {
  test("WB7: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no dspi binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("served by dspi hub --web", () => {
  test.skip(!process.env.DSPI_TERM, "no dspi binary (set DSPI_TERM)");

  test("WB7: the hub's own page at / is unchanged, byte for byte", async ({ request }) => {
    const r = await request.get(`${HUB}/`);
    expect(r.status()).toBe(200);
    expect(Buffer.compare(await r.body(), PLACEHOLDER)).toBe(0);
    // And the console is beside it, under its policy, with its types.
    const page = await request.get(`${HUB}/console/`);
    expect(page.headers()["content-type"]).toBe("text/html; charset=utf-8");
    const csp = page.headers()["content-security-policy"]!;
    expect(csp).toContain("script-src 'self' 'wasm-unsafe-eval'");
    expect(csp).toContain(`connect-src 'self' ws://127.0.0.1:${new URL(HUB).port};`);
    expect(await page.text()).toContain(`<meta name="dspi-hub" content="ws://127.0.0.1:${new URL(HUB).port}" />`);
    const wasm = await request.get(`${HUB}/console/dspi_wasm.wasm`);
    expect(wasm.headers()["content-type"]).toBe("application/wasm");
    expect(wasm.headers()["etag"]).toMatch(/^"[0-9a-f]{24}"$/);
    expect((await request.get(`${HUB}/console/test/`)).status()).toBe(404);
    const slash = await request.get(`${HUB}/console`, { maxRedirects: 0 });
    expect(slash.status()).toBe(301);
    expect(slash.headers()["location"]).toBe("/console/");
  });

  test("WB7: the console at /console/ reaches the hub's demo DSPi over Link", async ({ page }, info) => {
    const errors = await noPageErrors(page);
    const violations: string[] = [];
    page.on("console", (m) => {
      if (/Content Security Policy|Content-Security-Policy/i.test(m.text())) violations.push(m.text());
    });
    await page.goto(`${HUB}/console/`);
    await expect(page.getByRole("heading", { name: "DSPi Console" })).toBeVisible();
    // The link to the hub's own page, from ours only.
    await expect(page.getByRole("link", { name: "Hub page" })).toHaveAttribute("href", "/");
    const session = await openHubDemo(page, new URL(HUB).host);
    await expect(session.locator(".volume-value")).toHaveText(/dB$/);
    await writeOnce(session, info, "-7.5");
    expect(violations).toEqual([]);
    expect(errors).toEqual([]);
  });

  test("WB7: the Origin rule is the hub's: a page from elsewhere is refused", async ({ page }) => {
    await page.goto(`${PLAIN}/`);
    await picker(page).getByLabel("Add a hub").fill(new URL(HUB).host);
    await picker(page).getByRole("button", { name: "Add" }).click();
    const row = picker(page).getByRole("listitem").filter({ hasText: `Hub ${new URL(HUB).host}` });
    await row.getByRole("button", { name: "Connect" }).click();
    const session = page.getByRole("article").last();
    await expect(session.getByRole("alert")).toContainText("a hub accepts only pages it serves itself");
    await expect(session.locator(".device-meta")).toHaveCount(0);
  });
});

test.describe("served by dspi web on localhost", () => {
  test.skip(!process.env.DSPI_TERM, "no dspi binary (set DSPI_TERM)");

  test("WB7: the console on http://localhost is a secure page and reaches the local hub's demo", async ({ page }) => {
    const errors = await noPageErrors(page);
    await page.goto(`${LOCAL}/console/`);
    expect(await page.evaluate(() => isSecureContext)).toBe(true);
    await openHubDemo(page, `localhost:${LOCAL_PORT}`);
    expect(errors).toEqual([]);
  });

  test("WB7: one page holds a DSPi on USB (WebUSB mock) and the hub's demo over Link", async ({ page, browserName }, info) => {
    test.skip(browserName !== "chromium", "WebUSB is Chromium's");
    const errors = await noPageErrors(page);
    await page.goto(`${LOCAL}/console/test/usb.html?usb=MOCKA001&permitted=MOCKA001`);
    const usb = picker(page).getByRole("listitem").filter({ hasText: "DSPi on USB · MOCKA001" });
    await usb.getByRole("button", { name: "Connect" }).click();
    const usbSession = page.getByRole("article").last();
    await expect(usbSession.locator(".device-meta")).toHaveText("RP2350 · firmware 1.1.6 · MOCKA001");

    const hubSession = await openHubDemo(page, `localhost:${LOCAL_PORT}`);
    await writeOnce(hubSession, info, "-4.5");
    // Both open at once, each its own session.
    await expect(page.getByRole("tab")).toHaveCount(2);
    await page.getByRole("tab").first().click();
    await expect(page.getByRole("article").locator(".device-meta")).toHaveText("RP2350 · firmware 1.1.6 · MOCKA001");
    expect(errors).toEqual([]);
  });
});

test.describe("the static build", () => {
  test("WB7: any static host: the core's demo, the source link and the notices", async ({ page, request }) => {
    const errors = await noPageErrors(page);
    await page.goto(`${PLAIN}/`);
    await picker(page).getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
    await expect(page.getByRole("article", { name: "DSPi" }).locator(".device-meta")).toContainText("DEMO0000DSPI0001");
    // No hub serves this page: no hub entry, no link to a hub's page.
    await expect(page.getByRole("link", { name: "Hub page" })).toHaveCount(0);
    // The build's source link (DSPI_WEB_SOURCE_URL), when the build was given one.
    if (process.env.DSPI_WEB_SOURCE_URL) {
      await page.getByRole("button", { name: "About" }).click();
      await expect(page.getByRole("link", { name: "Source code" })).toHaveAttribute("href", process.env.DSPI_WEB_SOURCE_URL);
    }
    const notices = await (await request.get(`${PLAIN}/THIRD-PARTY-NOTICES.txt`)).text();
    expect(notices).toMatch(/^preact \d+\.\d+\.\d+ — MIT$/m);
    expect(notices).toContain("Permission is hereby granted");
    expect(notices).toMatch(/^serde_json [\d.]+ — MIT OR Apache-2\.0$/m);
    expect(errors).toEqual([]);
  });
});
