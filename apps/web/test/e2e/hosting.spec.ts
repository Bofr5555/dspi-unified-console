// WP WB7 e2e: the console reaches the demo device through each hosting route.
//
//   1. `uconsole hub --web`: the real hub (DSPI_TERM, the Terminal's binary) serves three
//      paths: / a landing page with two links, /console/ the console, /hub/ upstream's
//      DSPi Link page, byte for byte its build. The console knows its hub from the
//      <meta> the hub fills in and reaches the hub's demo DSPi over Link, under the
//      hub's policy (CSP); upstream's page pairs (PIN) and reaches it too.
//   2. `uconsole web`: a hub on loopback serving the console on http://localhost, a
//      secure context, so one page holds a DSPi on USB (the WebUSB mock, Chromium)
//      and the hub's demo over Link at once.
//   3. The static build (tools/serve.mjs, any static host): the core's own demo,
//      and the third-party notices beside it.
//
// The Origin rule is the hub's, unchanged: a page from anywhere else is refused.

import { spawn, type ChildProcess } from "node:child_process";
import { readFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { appMenu, openDevices } from "../nav";

const linkWords = (await import("../../../../ci/generated/web/link-words.json", { with: { type: "json" } })).default as {
  link: { key: string; text: string; action: string | null }[];
};
const words = (key: string) => linkWords.link.find((w) => w.key === key)!;

// -- WP HB9 -- Each group below starts its own `uconsole hub --web` or `uconsole web` on a free
// port (ownHub), so parallel workers and `--repeat-each` copies never share one.
let HUB = "";
let LOCAL_PORT = 0;
let LOCAL = "";
const PLAIN = `http://127.0.0.1:${Number(process.env.PORT ?? 8737)}`;
/** The core's simulated DSPi (FakeDevice::demo), which `uconsole --demo hub` shares. */
const DEMO = "DEMO0000DSPI0001";
const ASSETS = new URL("../../../../crates/dspi-hub/assets/web/", import.meta.url);
const LANDING = readFileSync(new URL("index.html", ASSETS));
/** Upstream's page as built (crates/dspi-hub/tools/build-link-web.sh): path under /hub/ -> bytes. */
const UPSTREAM = Object.keys(JSON.parse(readFileSync(new URL("hub/PROVENANCE.json", ASSETS), "utf8")).files as Record<string, string>).map(
  (rel) => [rel === "index.html" ? "" : rel, readFileSync(new URL(`hub/${rel}`, ASSETS))] as const,
);

async function noPageErrors(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    // A refused WebSocket is logged by the browser; the Origin test causes one.
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

/** Messages a browser logs when a Content Security Policy blocks something. */
function cspViolations(page: Page): string[] {
  const seen: string[] = [];
  page.on("console", (m) => {
    if (/Content Security Policy|Content-Security-Policy/i.test(m.text())) seen.push(m.text());
  });
  return seen;
}

/** On upstream's page: connect to the hub it was served by, open the demo DSPi. */
async function upstreamOpensDemo(page: Page, connect = true) {
  if (connect) {
    await expect(page.getByLabel("Hub address")).toHaveValue(/^127\.0\.0\.1:\d+$/);
    await page.getByRole("button", { name: "Connect" }).click();
  }
  await expect(page.getByRole("heading", { name: "Devices" })).toBeVisible();
  const row = page.getByRole("button").filter({ hasText: DEMO });
  await expect(row).toBeEnabled();
  await row.click();
  await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("DSPi Demo");
  await expect(page.getByText("Reading the DSPi…")).toHaveCount(0);
}

type OwnHub = { port: number; pin: string; child: ChildProcess };

/** A hub of the caller's own on a free port, `uconsole --demo <args> --port 0 --json`, once it
 *  says it is up (with `pairing`, once it shows its PIN). */
async function ownHub(args: string[], pairing = false): Promise<OwnHub> {
  const store = `${process.env.TMPDIR ?? "test-results"}/wb7-${args[0]}-${process.pid}-${Math.random().toString(36).slice(2)}`;
  const child = spawn(
    process.env.DSPI_TERM!,
    ["--demo", ...args, "--port", "0", "--json", "--no-keep-awake", "--store", store],
    { stdio: ["ignore", "pipe", "inherit"] },
  );
  const lines = createInterface({ input: child.stdout! });
  let port = 0;
  const pin = await new Promise<string>((ok, fail) => {
    child.on("exit", (code) => fail(new Error(`uconsole ${args[0]} exited ${code}`)));
    lines.on("line", (line) => {
      const e = JSON.parse(line) as { event?: string; port?: number; pin?: string };
      if (e.event === "status") {
        port = e.port ?? 0;
        if (!pairing) ok("");
      }
      if (e.event === "pairing" && e.pin) ok(e.pin);
    });
  });
  return { port, pin, child };
}

/** A hub of this test's own, with PIN pairing open: `uconsole --demo hub --web --pair --json` on a free port. */
const pinHub = () => ownHub(["hub", "--web", "--loopback", "--pair"], true);

/** The console's hubs as the e2e had them in playwright.config.ts: open access, the built console. */
const CONSOLE_FLAGS = ["--console", "dist", "--open", "--forget-clients"];

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
  await expect(session).toHaveAttribute("data-serial", new RegExp(DEMO));
  await expect(session.locator(".banner")).toHaveCount(0);
  return session;
}

/** A write with read-back, in one project only: the hub's demo DSPi is shared by all. */
async function writeOnce(session: ReturnType<Page["getByRole"]>, info: TestInfo, db: string) {
  if (info.project.name !== "chromium") return;
  const slider = session.getByRole("slider", { name: "Volume" });
  // -- WP WB6a -- the main window's slider is the Console's taper: the position of `db`
  // (as link.spec.ts writes it; the values used here are ones the taper reaches exactly)
  await slider.fill(String(Math.round(((Number(db) + 60) / 60) ** 2 * 200)));
  await slider.dispatchEvent("change");
  await expect(session.locator(".volume-value")).toHaveText(`${Number(db).toFixed(1)} dB`);
}

if (!process.env.DSPI_TERM) {
  test("WB7: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no uconsole binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("served by uconsole hub --web", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  // One browser at a time on this group's hub, as a person would use it.
  test.describe.configure({ mode: "serial" });
  let hub: OwnHub | undefined;
  test.beforeAll(async () => {
    hub = await ownHub(["hub", "--web", "--loopback", ...CONSOLE_FLAGS, "--name", "WB7 hub"]);
    HUB = `http://127.0.0.1:${hub.port}`;
  });
  test.afterAll(() => {
    hub?.child.kill();
  });

  test("WB7: / is the landing page and /hub/ upstream's page, byte for byte its build", async ({ request }) => {
    const r = await request.get(`${HUB}/`);
    expect(r.status()).toBe(200);
    expect(Buffer.compare(await r.body(), LANDING)).toBe(0);
    for (const [rel, bytes] of UPSTREAM) {
      const f = await request.get(`${HUB}/hub/${rel}`);
      expect(f.status(), rel).toBe(200);
      expect(Buffer.compare(await f.body(), bytes), rel).toBe(0);
    }
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
    await expect(page.getByRole("heading", { name: "uConsole for DSPi" })).toBeVisible();
    // The link to the hub's own page, from ours only.
    await expect(page.getByRole("link", { name: "Hub page" })).toHaveAttribute("href", "/hub/");
    const session = await openHubDemo(page, new URL(HUB).host);
    await expect(session.locator(".volume-value")).toHaveText(/dB$/);
    await writeOnce(session, info, "-6.0");
    expect(violations).toEqual([]);
    expect(errors).toEqual([]);
  });

  test("WB7: the landing page leads to the console and to the hub's own page", async ({ page }) => {
    const errors = await noPageErrors(page);
    const violations = cspViolations(page);
    await page.goto(`${HUB}/`);
    await expect(page.getByRole("heading", { name: "DSPi hub" })).toBeVisible();
    await expect(page.getByText("uConsole for DSPi controls the DSPis this hub shares")).toBeVisible();
    await page.getByRole("link", { name: "uConsole for DSPi" }).click();
    await expect(page).toHaveURL(`${HUB}/console/`);
    await expect(page.getByRole("heading", { name: "uConsole for DSPi" })).toBeVisible();
    await page.goto(`${HUB}/`);
    await page.getByRole("link", { name: "Hub", exact: true }).click();
    await expect(page).toHaveURL(`${HUB}/hub/`);
    await expect(page.getByLabel("Hub address")).toHaveValue(new URL(HUB).host);
    expect(violations).toEqual([]);
    expect(errors).toEqual([]);
  });

  test("WB7: upstream's page at /hub/ loads under its policy and reaches the hub's demo DSPi", async ({ page }) => {
    const errors = await noPageErrors(page);
    const violations = cspViolations(page);
    await page.goto(`${HUB}/hub/`);
    await upstreamOpensDemo(page);
    expect(violations).toEqual([]);
    expect(errors).toEqual([]);
  });

  test("WB7: upstream's page pairs with the hub's PIN, then reaches the demo DSPi", async ({ page }) => {
    const errors = await noPageErrors(page);
    const violations = cspViolations(page);
    const hub = await pinHub();
    try {
      await page.goto(`http://127.0.0.1:${hub.port}/hub/`);
      await page.getByRole("button", { name: "Connect" }).click();
      await expect(page.getByRole("heading", { name: "Pairing" })).toBeVisible();
      await page.getByLabel("PIN (6 digits)").fill(hub.pin);
      await page.getByRole("button", { name: "Pair" }).click();
      await upstreamOpensDemo(page, false);
      expect(violations).toEqual([]);
      expect(errors).toEqual([]);
    } finally {
      hub.child.kill();
    }
  });

  // -- WP HB9 -- A browser's page loads have their own budget, apart from WebSockets: four
  // windows of their own (each a socket pool, up to six sockets and pre-connects) on one
  // address load the console and its hub's demo at once, three times over, and nothing is
  // answered 503 or fails.
  test("HB9: four browser windows on one address load the console at once, and none is refused", async ({ browser }) => {
    test.setTimeout(120_000);
    const refused: string[] = [];
    for (let round = 0; round < 3; round++) {
      const contexts = await Promise.all([0, 1, 2, 3].map(() => browser.newContext()));
      try {
        // Until the windows close, which aborts whatever they still have open.
        let watching = true;
        for (const c of contexts) {
          c.on("response", (r) => {
            if (watching && r.status() === 503) refused.push(`${round}: 503 ${r.url()}`);
          });
          c.on("requestfailed", (r) => {
            if (watching) refused.push(`${round}: failed ${r.url()} ${r.failure()?.errorText}`);
          });
        }
        const pages = await Promise.all(contexts.map((c) => c.newPage()));
        await Promise.all(
          pages.map(async (page) => {
            await page.goto(`${HUB}/console/`);
            await expect(page.getByRole("heading", { name: "uConsole for DSPi" })).toBeVisible();
            const session = await openHubDemo(page, new URL(HUB).host);
            await expect(session.locator(".volume-value")).toHaveText(/dB$/);
          }),
        );
        // And the other two pages, all at once, beside the open consoles' sockets.
        const others = await Promise.all(contexts.map((c) => c.newPage()));
        await Promise.all(others.map((page, i) => page.goto(i % 2 ? `${HUB}/hub/` : `${HUB}/`)));
        for (const page of others) await expect(page.locator("body")).not.toBeEmpty();
        watching = false;
      } finally {
        await Promise.all(contexts.map((c) => c.close()));
      }
    }
    expect(refused).toEqual([]);
  });

  test("WB7: the Origin rule is the hub's: a page from elsewhere is refused", async ({ page }) => {
    await page.goto(`${PLAIN}/`);
    await picker(page).getByLabel("Add a hub").fill(new URL(HUB).host);
    await picker(page).getByRole("button", { name: "Add" }).click();
    const row = picker(page).getByRole("listitem").filter({ hasText: `Hub ${new URL(HUB).host}` });
    await row.getByRole("button", { name: "Connect" }).click();
    const session = page.getByRole("article").last();
    await expect(session.getByRole("alert")).toContainText("a hub accepts only pages it serves itself");
    await expect(session).not.toHaveAttribute("data-serial", /./);
  });
});

test.describe("served by uconsole web on localhost", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  test.describe.configure({ mode: "serial" });
  let hub: OwnHub | undefined;
  test.beforeAll(async () => {
    hub = await ownHub(["web", ...CONSOLE_FLAGS, "--name", "WB7 local"]);
    LOCAL_PORT = hub.port;
    LOCAL = `http://localhost:${LOCAL_PORT}`;
  });
  test.afterAll(() => {
    hub?.child.kill();
  });

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
    await expect(usbSession).toHaveAttribute("data-serial", "MOCKA001");

    await openDevices(page); // -- WP WEBM1 --
    const hubSession = await openHubDemo(page, `localhost:${LOCAL_PORT}`);
    await writeOnce(hubSession, info, "-18.0");
    // Both open at once, each its own session.
    // -- WP WB6b -- the sessions' tabs (a session's own screens have tabs too)
    await expect(page.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab")).toHaveCount(2);
    await page.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab").first().click();
    await expect(page.getByRole("article")).toHaveAttribute("data-serial", "MOCKA001");
    expect(errors).toEqual([]);
  });
});

test.describe("the static build", () => {
  test("WB7: any static host: the core's demo, the source link and the notices", async ({ page, request }) => {
    const errors = await noPageErrors(page);
    await page.goto(`${PLAIN}/`);
    await picker(page).getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
    await expect(page.getByRole("article", { name: "DSPi" })).toHaveAttribute("data-serial", /DEMO0000DSPI0001/);
    // No hub serves this page: no hub entry, no link to a hub's page.
    await expect(page.getByRole("link", { name: "Hub page" })).toHaveCount(0);
    // The build's source link (DSPI_WEB_SOURCE_URL), when the build was given one.
    if (process.env.DSPI_WEB_SOURCE_URL) {
      await appMenu(page, "About uConsole"); // -- WP WEBM4 --
      await expect(page.getByRole("link", { name: "Source code" })).toHaveAttribute("href", process.env.DSPI_WEB_SOURCE_URL);
    }
    const notices = await (await request.get(`${PLAIN}/THIRD-PARTY-NOTICES.txt`)).text();
    expect(notices).toMatch(/^preact \d+\.\d+\.\d+ — MIT$/m);
    expect(notices).toContain("Permission is hereby granted");
    expect(notices).toMatch(/^serde_json [\d.]+ — MIT OR Apache-2\.0$/m);
    // -- WP CLEAN1 fixup cl1 -- the source notice is served beside the console
    const source = await (await request.get(`${PLAIN}/SOURCE.txt`)).text();
    expect(source).toContain("DRAFT");
    expect(source).toMatch(/uConsole-for-DSPi-[\d.]+-source\.tar\.gz/);
    expect(source).toContain("Whoever serves this console to you");
    expect(errors).toEqual([]);
  });
});
