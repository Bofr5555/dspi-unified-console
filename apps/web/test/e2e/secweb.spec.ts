// WP SECWEB e2e (PLAN-SEC 4.6), on real hubs (`uconsole`, DSPI_TERM) serving the built console:
//
// 1. The hub's own https page (`uconsole hub --web --tls require`): with the hub's
//    certificate accepted, the console connects over wss://, the connection bar is not red,
//    and the hub settings show the hub key's fingerprint, the same as the hub's own. No
//    mixed-content ws:// is ever attempted (a ws:// hub kept in storage is listed as wss://).
//    The probe (S11): isSecureContext and the WebUSB gate on that page, accepted either by a
//    click-through equivalent (ignoreHTTPSErrors) or with the hub's key trusted
//    (Chromium's --ignore-certificate-errors-spki-list); written to secweb-probe-*.json.
// 2. `uconsole web` reaching a TLS hub on this machine's LAN address through its pinning
//    proxy: encrypted, the fingerprint shown; the hub replaced by one with another key at
//    the same address: refused, in the proxy's words.
// 3. `uconsole web` reaching a hub without TLS (an upstream hub, compatibility policy): it
//    connects, and the bar is red with " (unsecured)" and the info button's explanation.

import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { networkInterfaces } from "node:os";
import { createInterface } from "node:readline";
import { expect, test, type Browser, type Locator, type Page } from "@playwright/test";
import { fingerprint } from "../../src/fleet/security";
import { fleetOf, openOnHub, openPage, showThisComputer } from "../nav"; // -- WP UB6 -- the Fleet

const DEMO = "DEMO0000DSPI0001";

type OwnHub = { port: number; child: ChildProcess; store: string };

/** The hub's TLS key folders under its store (`<store>/.../tls`). */
function findTlsDirs(dir: string): string[] {
  const found: string[] = [];
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    if (!e.isDirectory()) continue;
    const p = `${dir}/${e.name}`;
    if (e.name === "tls") found.push(p);
    else found.push(...findTlsDirs(p));
  }
  return found;
}

function newStore(): string {
  const store = `${process.env.TMPDIR ?? "test-results"}/secweb-${process.pid}-${Math.random().toString(36).slice(2)}`;
  mkdirSync(store, { recursive: true });
  return store;
}

/** `uconsole --demo <args> --port <port> --json`, once it says it is up. */
async function startHub(args: string[], port = 0, store = newStore()): Promise<OwnHub> {
  const child = spawn(process.env.DSPI_TERM!, ["--demo", ...args, "--port", String(port), "--json", "--no-keep-awake", "--store", store], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  const lines = createInterface({ input: child.stdout! });
  const up = await new Promise<number>((ok, fail) => {
    child.on("exit", (code) => fail(new Error(`uconsole ${args.join(" ")} exited ${code}`)));
    lines.on("line", (line) => {
      try {
        const e = JSON.parse(line) as { event?: string; port?: number };
        if (e.event === "status") ok(e.port ?? 0);
      } catch {
        // not a JSON line
      }
    });
  });
  return { port: up, child, store };
}

async function stopHub(h: OwnHub | undefined): Promise<void> {
  if (!h || h.child.exitCode !== null) return;
  const exited = new Promise<void>((r) => h.child.once("exit", () => r()));
  h.child.kill();
  await exited;
}

/** This machine's first LAN IPv4 address: a hub there is not "on this machine" to the page. */
function lanAddress(): string | null {
  for (const list of Object.values(networkInterfaces())) {
    for (const a of list ?? []) if (a.family === "IPv4" && !a.internal) return a.address;
  }
  return null;
}

const CONSOLE = ["--console", "dist", "--open", "--forget-clients"];
const picker = (page: Page) => page.getByRole("navigation", { name: "DSPi devices" });
const connection = (session: Locator) => session.getByRole("complementary", { name: "Status" }).getByLabel("Connection", { exact: true });

async function openDemo(page: Page, host: string): Promise<Locator> {
  const row = picker(page).getByRole("listitem").filter({ hasText: `Hub ${host}` });
  // -- WP UB6 -- (UXP B6) the page's own hub lists its DSPis on its Fleet; another hub is an entry on This Computer
  if (host === new URL(page.url()).host) {
    await expect(fleetOf(page)).toBeVisible({ timeout: 20_000 });
    return openOnHub(page, { host, serial: DEMO });
  }
  await showThisComputer(page);
  await expect(row).toHaveCount(1);
  await row.getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  const entry = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: DEMO.slice(-8) });
  // What the session says when it does not get there.
  const failed = page.locator('article[data-status="failed"]');
  await expect(entry.or(failed).first()).toBeVisible();
  if (await failed.count()) console.log(`SECWEB session failed: ${await failed.first().textContent()}`);
  await entry.getByRole("button", { name: "Open" }).click();
  await expect(session).toHaveAttribute("data-serial", new RegExp(DEMO));
  return session;
}

async function hubSettings(page: Page, session: Locator): Promise<Locator> {
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: "Advanced" }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await openPage(session, "Hub Settings");
  return session.locator(".hub-settings-screen");
}

/** Keep `hubs` in the page's own list before it loads (as the user's "Add hub" would). */
async function withSavedHubs(page: Page, hubs: { url: string; name: string }[]) {
  await page.addInitScript((list) => {
    try {
      localStorage.setItem("dspi-web.hubs.v1", JSON.stringify(list));
    } catch {
      // storage refused: the test then fails visibly
    }
  }, hubs);
}

function probeOut(name: string, v: unknown) {
  const dir = process.env.PW_OUTPUT_DIR ? `${process.env.PW_OUTPUT_DIR}/..` : "test-results";
  try {
    writeFileSync(`${dir}/secweb-probe-${name}.json`, JSON.stringify(v, null, 2));
  } catch {
    // the result is in the log as well
  }
  console.log(`SECWEB probe ${name}: ${JSON.stringify(v)}`);
}

async function probe(page: Page) {
  return page.evaluate(() => ({
    url: location.href,
    isSecureContext,
    webUsbApi: "usb" in navigator && !!(navigator as unknown as { usb?: unknown }).usb,
    serviceWorkerApi: "serviceWorker" in navigator,
    // The page's own gate (fleet/env.ts pageEnv): what the picker shows for USB.
    usbRow: document.querySelector(".source-usb")?.textContent ?? "",
  }));
}

if (!process.env.DSPI_TERM) {
  test("SECWEB: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no uconsole binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("SECWEB: the hub's https console", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  test.describe.configure({ mode: "serial" });
  let hub: OwnHub | undefined;
  test.beforeAll(async () => {
    hub = await startHub(["hub", "--web", "--loopback", "--tls", "require", ...CONSOLE, "--name", "SECWEB tls"]);
  });
  test.afterAll(async () => stopHub(hub));
  test.use({ ignoreHTTPSErrors: true });

  test("SECWEB: over wss://, not red, the fingerprint matches the hub's; no ws:// attempted", async ({ page, request }, info) => {
    const base = `https://127.0.0.1:${hub!.port}`;
    const spki = ((await (await request.get(`${base}/dspi/v1/info`)).json()) as { tls_spki: string }).tls_spki;
    const mixed: string[] = [];
    page.on("console", (m) => {
      if (/Mixed Content|insecure WebSocket/i.test(m.text())) mixed.push(m.text());
    });
    const sockets: string[] = [];
    page.on("websocket", (ws) => sockets.push(ws.url()));
    // A ws:// hub kept from an http visit: on this page it is wss:// (or refused), never ws://.
    await withSavedHubs(page, [{ url: "ws://127.0.0.1:9", name: "Old" }]);
    await page.goto(`${base}/console/`);
    await showThisComputer(page); // -- WP UB6 -- the page starts on the hub's Fleet
    await expect(picker(page).getByRole("listitem").filter({ hasText: "Hub 127.0.0.1:9" })).toHaveCount(1);
    probeOut(`accepted-${info.project.name}`, await probe(page));
    await page.getByRole("tablist", { name: "Pages" }).getByRole("tab", { name: "Fleet" }).click();
    const session = await openDemo(page, `127.0.0.1:${hub!.port}`);
    await expect(connection(session)).toContainText("Connected");
    await expect(connection(session)).not.toContainText("(unsecured)");
    await expect(connection(session)).not.toHaveClass(/link-unsecured/);
    const screen = await hubSettings(page, session);
    const sec = screen.getByRole("region", { name: "Connection security" });
    await expect(sec).toContainText("Encrypted");
    await expect(sec.locator(".hub-fingerprint code")).toHaveText(fingerprint(spki));
    await expect(screen.getByRole("note", { name: "Unencrypted" })).toHaveCount(0);
    // Compatibility mode is off: no downgrade warning, though this console is an Admin.
    await expect(screen.getByRole("alert", { name: "Security downgraded" })).toHaveCount(0);
    expect(mixed).toEqual([]);
    expect(sockets.filter((u) => u.startsWith("ws://"))).toEqual([]);
  });

  test("SECWEB probe: isSecureContext and WebUSB with the hub's key trusted (Chromium)", async ({ browserName, playwright, request }) => {
    test.skip(browserName !== "chromium", "--ignore-certificate-errors-spki-list is Chromium's");
    const base = `https://127.0.0.1:${hub!.port}`;
    const spki = ((await (await request.get(`${base}/dspi/v1/info`)).json()) as { tls_spki: string }).tls_spki;
    const b64 = Buffer.from(spki, "base64url").toString("base64");
    let browser: Browser | undefined;
    try {
      browser = await playwright.chromium.launch({ args: [`--ignore-certificate-errors-spki-list=${b64}`] });
      const page = await (await browser.newContext()).newPage();
      await page.goto(`${base}/console/`);
      await expect(page.getByRole("heading", { name: "uConsole for DSPi" })).toBeVisible();
      await showThisComputer(page); // -- WP UB6 -- the page starts on the hub's Fleet
      await expect(picker(page).locator(".source-usb")).toBeVisible();
      const r = await probe(page);
      probeOut("trusted-chromium", r);
      expect(r.isSecureContext).toBe(true);
      // The gate follows the context: offered where the API is there.
      expect(r.webUsbApi).toBe(true);
      expect(r.usbRow).toContain("Connect");
    } finally {
      await browser?.close();
    }
  });
});

test.describe("SECWEB: uconsole web's pinning proxy", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  test.describe.configure({ mode: "serial" });
  const lan = lanAddress();
  let web: OwnHub | undefined;
  let remote: OwnHub | undefined;
  test.beforeAll(async () => {
    test.skip(!lan, "no LAN address in this container");
    web = await startHub(["web", ...CONSOLE, "--name", "SECWEB local"]);
  });
  test.afterAll(async () => {
    await stopHub(remote);
    await stopHub(web);
  });

  test("SECWEB: a TLS hub on the LAN through the proxy: pinned, encrypted, its fingerprint; another key refused", async ({ page, browserName }) => {
    test.skip(browserName !== "chromium", "one browser: the proxy pins once per hub");
    test.setTimeout(90_000);
    remote = await startHub(["hub", "--tls", "require", "--open", "--forget-clients", "--name", "SECWEB remote"]);
    const at = `${lan}:${remote.port}`;
    await withSavedHubs(page, [{ url: `ws://${at}`, name: "Remote" }]);
    const sockets: string[] = [];
    page.on("websocket", (ws) => sockets.push(ws.url()));
    await page.goto(`http://localhost:${web!.port}/console/`);
    const session = await openDemo(page, at);
    await expect(connection(session)).not.toContainText("(unsecured)");
    const screen = await hubSettings(page, session);
    const sec = screen.getByRole("region", { name: "Connection security" });
    await expect(sec).toContainText("uconsole web");
    await expect(sec.locator(".hub-fingerprint code")).toHaveText(/^SHA256:[0-9A-Z]{4}(-[0-9A-Z]{4}){3}$/);
    const pinned = await sec.locator(".hub-fingerprint code").textContent();

    // The same hub (its id kept) with another key at the same address: refused, no token sent.
    const port = remote.port;
    const store = remote.store;
    await stopHub(remote);
    for (const tls of findTlsDirs(store)) rmSync(tls, { recursive: true, force: true });
    remote = await startHub(["hub", "--tls", "require", "--open", "--forget-clients", "--name", "SECWEB rekeyed"], port, store);
    const page2 = await page.context().newPage();
    await page2.goto(`http://localhost:${web!.port}/console/`);
    await showThisComputer(page2); // -- WP UB6 -- the page starts on its hub's Fleet
    const row = picker(page2).getByRole("listitem").filter({ hasText: `Hub ${at}` });
    await row.getByRole("button", { name: "Connect" }).click();
    const s2 = page2.getByRole("article").last();
    await expect(s2).toContainText("different key", { timeout: 30_000 });
    await expect(s2).not.toHaveAttribute("data-serial", /./);
    expect(pinned).toBeTruthy();
    // Every page socket went to the page's own origin (the proxy), none to the LAN hub.
    expect(sockets.every((u) => u.startsWith(`ws://localhost:${web!.port}/`))).toBe(true);
  });

  test("SECWEB: an upstream hub without TLS still connects; the bar is red, says (unsecured), and explains", async ({ page, browserName }) => {
    test.skip(browserName === "firefox", "the bar is the same in every browser; Chromium and narrow show it");
    const plain = await startHub(["hub", "--tls", "off", "--open", "--forget-clients", "--name", "SECWEB plain"]);
    try {
      const at = `${lan}:${plain.port}`;
      await withSavedHubs(page, [{ url: `ws://${at}`, name: "Plain" }]);
      await page.goto(`http://localhost:${web!.port}/console/`);
      const session = await openDemo(page, at);
      const bar = connection(session);
      await expect(bar).toContainText("Connected");
      await expect(bar).toContainText("(unsecured)");
      await expect(bar).toHaveClass(/link-unsecured/);
      const bg = await bar.evaluate((el) => getComputedStyle(el).backgroundColor);
      expect(bg).toBe("rgb(179, 38, 30)");
      await bar.getByRole("button", { name: "About this connection" }).click();
      const dialog = page.getByRole("dialog", { name: "Unencrypted connection" });
      await expect(dialog).toContainText("trusted local network or over a VPN");
      await expect(dialog).toContainText("pairing PIN and settings");
      await dialog.getByRole("button", { name: "Close" }).click();
      const screen = await hubSettings(page, session);
      await expect(screen.getByRole("note", { name: "Unencrypted" })).toBeVisible();
      if (test.info().project.name === "narrow") {
        const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
        expect(over).toBeLessThanOrEqual(0);
      }
    } finally {
      await stopHub(plain);
    }
  });
});
