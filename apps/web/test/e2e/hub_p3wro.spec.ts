// WP P3WRO e2e: the firmware rollout on the hub screen, on the console served by a real hub
// (`uconsole --demo hub --web --tls on --rollout --allow-unverified-firmware
// --with-rollout-bench`, DSPI_TERM): the bench is one more simulated DSPi "on the hub's USB"
// with a fake bootloader and writer (apps/terminal/src/hub/rollout_bench.rs: an image of
// 1.1.N comes back running it with its settings; any other version comes back on the old
// firmware with other settings; its core-voltage reads take 2.5 s, the write 2 s).
// - Admin over https: an image over 8 MiB refused before upload; two images uploaded and
//   listed (version, platform, 1.20 V, "not verified"); a rollout started from the dialog
//   (the steps in words, what the DSPi will do, Mute First) and every step shown live; the
//   result's lines; a botched image fails at the verify with the alarm and its step, and
//   "Recall the Export" recalls the hub's export; a cancel before the bootloader entry
//   works, one after it is refused and said so, the alarm clears when a rollout ends ok.
// - (FU1) A second page in the client read-only mode (`?readonly`) on the same Admin hub: Start
//   and Add Image off, but Cancel Rollout on (the core lets fw.rollout.cancel out), and it works.
// - Over plaintext (an http:// page on the `--tls on` hub): the TLS notice, nothing to start.
// - A Control client (PIN pairing): no rollout on its hub screen.
// Chromium, Firefox and 375 px with no sideways scroll.

import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

const DEMO = "DEMO0000DSPI0001";
const BENCH = "BENCH0000DSPI001";

type OwnHub = { port: number; pin: string; child: ChildProcess };

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !/WebSocket|certificate|ERR_CERT/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

async function noSidewaysScroll(page: Page) {
  if (test.info().project.name !== "narrow") return;
  const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  const wide = over > 0
    ? await page.evaluate(() => {
        const w = document.documentElement.clientWidth;
        return [...document.querySelectorAll("body *")]
          .filter((e) => e.getBoundingClientRect().right > w + 1 && !(e.parentElement && e.parentElement.getBoundingClientRect().right > w + 1))
          .slice(0, 5)
          .map((e) => `${e.tagName.toLowerCase()}.${(e as HTMLElement).className}`);
      })
    : [];
  expect(over, wide.join(", ")).toBeLessThanOrEqual(0);
}

function newStore(): string {
  const store = `${process.env.TMPDIR ?? "test-results"}/p3wro-${process.pid}-${Math.random().toString(36).slice(2)}`;
  mkdirSync(store, { recursive: true });
  return store;
}

/** The hub with the rollout and its bench, TLS `on` (encrypted and plaintext both); `--pair` gives a PIN. */
async function startHub(access: string[]): Promise<OwnHub> {
  const child = spawn(
    process.env.DSPI_TERM!,
    [
      "--demo",
      "hub",
      "--web",
      "--loopback",
      "--console",
      "dist",
      "--tls",
      "on",
      "--rollout",
      "--allow-unverified-firmware",
      "--with-rollout-bench",
      ...access,
      "--port",
      "0",
      "--json",
      "--no-keep-awake",
      "--store",
      newStore(),
    ],
    { stdio: ["ignore", "pipe", "inherit"] },
  );
  const lines = createInterface({ input: child.stdout! });
  const pairing = access.includes("--pair");
  let port = 0;
  const pin = await new Promise<string>((ok, fail) => {
    child.on("exit", (code) => fail(new Error(`uconsole hub exited ${code}`)));
    lines.on("line", (line) => {
      try {
        const e = JSON.parse(line) as { event?: string; port?: number; pin?: string };
        if (e.event === "status") {
          port = e.port ?? 0;
          if (!pairing) ok("");
        }
        if (e.event === "pairing" && e.pin) ok(e.pin);
      } catch {
        // not a JSON line
      }
    });
  });
  return { port, pin, child };
}

async function stopHub(h: OwnHub | undefined): Promise<void> {
  if (!h || h.child.exitCode !== null) return;
  const exited = new Promise<void>((r) => h.child.once("exit", () => r()));
  h.child.kill();
  await exited;
}

/** The console served by the hub (`scheme`, `query` such as "?readonly"): connect (pairing with the PIN when asked), open the demo DSPi, Advanced, the Hub tab. */
async function openHubScreen(page: Page, hub: OwnHub, scheme: "https" | "http", query = ""): Promise<Locator> {
  await page.goto(`${scheme}://127.0.0.1:${hub.port}/console/${query}`);
  const row = page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` });
  await row.getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  if (hub.pin) {
    await session.getByLabel("PIN").fill(hub.pin);
    await session.getByRole("button", { name: "Pair" }).click();
  }
  const entry = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: DEMO.slice(-8) });
  await entry.getByRole("button", { name: "Open" }).click();
  await expect(session).toHaveAttribute("data-serial", new RegExp(DEMO));
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: "Advanced" }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await openPage(session, "Hub");
  await expect(session.getByRole("region", { name: "Devices on the hub" })).toBeVisible();
  return session;
}

// -- A UF2 image of the DSPi firmware `version`, stamped `stamp` mV (laid out as the core's tests build them) --

const XIP = 0x1000_0000;

function uf2(version: string, stamp: string): Buffer {
  const entries: [number, number, string][] = [
    [0x5052, 0x0203_1c86, "DSPi"],
    [0x5052, 0x11a9_bc3a, version],
    [0x5344, 0x0b7a_4c15, stamp],
  ];
  const f = Buffer.alloc(0x2000);
  const a = (off: number) => XIP + off;
  f.writeUInt32LE(0x7188_ebf2, 0x100);
  f.writeUInt32LE(a(0x200), 0x104);
  f.writeUInt32LE(a(0x200 + 4 * entries.length), 0x108);
  f.writeUInt32LE(a(0x180), 0x10c);
  f.writeUInt32LE(0xe71a_a390, 0x110);
  entries.forEach(([tag, id, text], i) => {
    const at = 0x400 + 16 * i;
    f.writeUInt32LE(a(at), 0x200 + 4 * i);
    f.writeUInt16LE(6, at);
    f.writeUInt16LE(tag, at + 2);
    f.writeUInt32LE(id, at + 4);
    const s = 0x800 + 64 * i;
    f.writeUInt32LE(a(s), at + 8);
    f.write(text, s, "latin1");
  });
  const pages = f.length / 256;
  const out = Buffer.alloc(pages * 512);
  for (let i = 0; i < pages; i++) {
    const b = out.subarray(i * 512, (i + 1) * 512);
    [
      [0, 0x0a32_4655],
      [4, 0x9e5d_5157],
      [8, 0x2000],
      [12, XIP + 256 * i],
      [16, 256],
      [20, i],
      [24, pages],
      [28, 0xe48b_ff59],
      [508, 0x0ab1_6f30],
    ].forEach(([off, v]) => b.writeUInt32LE(v!, off!));
    f.copy(b, 32, i * 256, (i + 1) * 256);
  }
  return out;
}

async function pick(page: Page, ro: Locator, name: string, buffer: Buffer) {
  const [chooser] = await Promise.all([page.waitForEvent("filechooser"), ro.getByRole("button", { name: "Add Image…" }).click()]);
  await chooser.setFiles({ name, mimeType: "application/octet-stream", buffer });
}

/** Start a rollout of `image` on the bench from the dialog: Mute First when it asks, else Start. */
async function startOnBench(page: Page, ro: Locator, image: string): Promise<Locator> {
  // The hub's ids follow on (ro1, ro2, ...), and this hub has had no other rollouts.
  const id = `ro${(await ro.getByRole("group", { name: /^Rollout ro\d+$/ }).count()) + 1}`;
  await ro.getByLabel("DSPi or group").selectOption(`serial:${BENCH}`);
  await ro.getByLabel("Image", { exact: true }).selectOption(image);
  await expect(ro.getByRole("button", { name: "Start Update…" })).toBeEnabled({ timeout: 30_000 });
  await ro.getByRole("button", { name: "Start Update…" }).click();
  const dialog = page.getByRole("dialog", { name: "Update the Firmware" });
  await expect(dialog).toContainText("offline for about 30 s");
  await expect(dialog.getByRole("listitem")).toHaveCount(8);
  await dialog.getByRole("button", { name: /^(Start|Mute First)$/ }).click();
  await expect(dialog).toHaveCount(0);
  const said = ro.getByRole("status", { name: "Firmware action" });
  await expect(said).toContainText(`Rollout ${id} started`, { timeout: 30_000 });
  return ro.getByRole("group", { name: `Rollout ${id}` });
}

if (!process.env.DSPI_TERM) {
  test("P3WRO: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no uconsole binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("P3WRO on a hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  test.use({ ignoreHTTPSErrors: true });
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    await stopHub(hub);
    hub = undefined;
  });

  test("P3WRO Admin: upload, start, every step, the result; a failed verify with the alarm and the recall; cancel before and after the entry", async ({ page }) => {
    test.setTimeout(300_000);
    const errors = errorsOf(page);
    hub = await startHub(["--open", "--forget-clients"]);
    const session = await openHubScreen(page, hub, "https");
    const ro = session.getByRole("region", { name: "Firmware rollout" });
    await expect(ro).toBeVisible();
    await expect(ro).toContainText("No images on the hub yet.");
    const said = ro.getByRole("status", { name: "Firmware action" });

    // Over 8 MiB: refused before a byte goes.
    await pick(page, ro, "huge.uf2", Buffer.alloc(8 * 1024 * 1024 + 1));
    await expect(said).toContainText("8.0 MiB");
    await expect(ro.locator("tr.hub-image-row")).toHaveCount(0);

    // Two images: listed with version, platform, 1.20 V and "not verified".
    await pick(page, ro, "dspi-1.1.7.uf2", uf2("1.1.7", "1200"));
    await expect(said).toContainText("dspi-1.1.7.uf2 is in the hub's store.");
    await pick(page, ro, "dspi-2.0.0.uf2", uf2("2.0.0", "1200"));
    await expect(said).toContainText("dspi-2.0.0.uf2 is in the hub's store.");
    const good = ro.getByRole("row", { name: "dspi-1.1.7.uf2" });
    await expect(good.locator(".hub-image-version")).toHaveText("1.1.7");
    await expect(good).toContainText("RP2350");
    await expect(good.locator(".hub-image-stamp")).toContainText("1.20 V");
    await expect(good.locator(".hub-image-verified")).toHaveText("not verified");
    await noSidewaysScroll(page);

    // The start dialog: the steps in words, what the DSPi does, and Mute First (the bench plays).
    await ro.getByLabel("DSPi or group").selectOption(`serial:${BENCH}`);
    await ro.getByLabel("Image", { exact: true }).selectOption("dspi-1.1.7.uf2");
    await ro.getByRole("button", { name: "Start Update…" }).click();
    const dialog = page.getByRole("dialog", { name: "Update the Firmware" });
    await expect(dialog).toContainText("offline for about 30 s");
    await expect(dialog).toContainText("restarts into its bootloader");
    await expect(dialog.getByRole("listitem")).toHaveCount(8);
    await expect(dialog).toContainText("This DSPi is not muted.");
    await expect(dialog).toContainText("Start While Playing updates it as it is");
    await expect(dialog.getByRole("button", { name: "Start While Playing" })).toBeVisible();
    await dialog.getByRole("button", { name: "Mute First" }).click();
    const first = ro.getByRole("group", { name: "Rollout ro1" });
    await expect(first).toBeVisible({ timeout: 30_000 });
    // Each step shown as it runs, then done.
    await expect(first.locator('li[data-step="flash"][data-state="running"]')).toHaveCount(1, { timeout: 60_000 });
    await expect(first).toHaveAttribute("data-state", "ok", { timeout: 90_000 });
    await expect(first.locator('li.hub-step[data-state="done"]')).toHaveCount(8);
    const result = first.getByRole("list", { name: "Result" });
    await expect(result).toContainText("Firmware 1.1.7 (was 1.1.6)");
    await expect(result).toContainText("Extension channel:");
    await expect(result).toContainText("Core voltage 1.20 V read back");
    await expect(result).toContainText("Settings unchanged");
    await expect(first.getByRole("button", { name: "Recall the Export" })).toHaveCount(0);

    // A botched image: failed at the verify, the alarm with its step, the recall offered and done.
    const second = await startOnBench(page, ro, "dspi-2.0.0.uf2");
    await expect(second).toHaveAttribute("data-state", "failed", { timeout: 90_000 });
    const stopped = second.getByRole("alert", { name: "Where it stopped" });
    await expect(stopped).toHaveAttribute("data-key", "rollout.verify");
    await expect(stopped).toContainText("not 2.0.0");
    await expect(second.getByRole("list", { name: "Result" })).toContainText("Settings changed since the export");
    const alarm = ro.getByRole("alert", { name: "Firmware update failed" });
    await expect(alarm).toHaveCount(1, { timeout: 30_000 });
    await expect(alarm).toHaveAttribute("data-step", "verify");
    await expect(alarm).toContainText("Checking the new firmware");
    await expect(session.page().getByRole("status", { name: "Hub alarms" })).toBeVisible();
    await second.getByRole("button", { name: "Recall the Export" }).click();
    await expect(said).toContainText("Recalled the export", { timeout: 30_000 });
    await noSidewaysScroll(page);

    // A cancel before the bootloader entry: taken; the DSPi untouched.
    const third = await startOnBench(page, ro, "dspi-1.1.7.uf2");
    await third.getByRole("button", { name: "Cancel Rollout" }).click();
    await expect(said).toContainText("cancelled before the bootloader entry");
    await expect(third).toHaveAttribute("data-state", "cancelled");
    await expect(third.locator('li[data-step="bootloader"]')).toHaveAttribute("data-state", "skipped");

    // A cancel after the entry: refused, and said so; the update goes on and ends ok, which clears the alarm.
    const fourth = await startOnBench(page, ro, "dspi-1.1.7.uf2");
    await expect(fourth.locator('li[data-step="bootloader"][data-state="done"]')).toHaveCount(1, { timeout: 60_000 });
    await expect(fourth).toContainText("past its bootloader entry");
    await fourth.getByRole("button", { name: "Cancel Rollout" }).click();
    await expect(said).toContainText("Too late to cancel");
    await expect(fourth).toHaveAttribute("data-state", "ok", { timeout: 90_000 });
    await expect(alarm).toHaveCount(0, { timeout: 30_000 });
    await noSidewaysScroll(page);

    // -- WP FU1 -- the client read-only mode: nothing to start or upload, but a cancel goes out.
    const viewer = await page.context().newPage();
    const viewerErrors = errorsOf(viewer);
    const roSession = await openHubScreen(viewer, hub, "https", "?readonly");
    const roRollout = roSession.getByRole("region", { name: "Firmware rollout" });
    await expect(roRollout).toBeVisible();
    await expect(roRollout.getByRole("button", { name: "Start Update…" })).toBeDisabled();
    await expect(roRollout.getByRole("button", { name: "Add Image…" })).toBeDisabled();
    const fifth = await startOnBench(page, ro, "dspi-1.1.7.uf2");
    const fifthId = (await fifth.getAttribute("aria-label"))!.replace(/^Rollout /, "");
    const roFifth = roRollout.getByRole("group", { name: `Rollout ${fifthId}` });
    await expect(roFifth).toHaveAttribute("data-state", "running", { timeout: 30_000 });
    await expect(roFifth.getByRole("button", { name: "Cancel Rollout" })).toBeEnabled();
    await expect(roRollout.getByRole("button", { name: "Start Update…" })).toBeDisabled();
    await expect(roRollout.getByRole("button", { name: "Add Image…" })).toBeDisabled();
    await roFifth.getByRole("button", { name: "Cancel Rollout" }).click();
    await expect(roRollout.getByRole("status", { name: "Firmware action" })).toContainText("cancelled before the bootloader entry");
    await expect(roFifth).toHaveAttribute("data-state", "cancelled");
    await expect(fifth).toHaveAttribute("data-state", "cancelled", { timeout: 30_000 });
    await noSidewaysScroll(viewer);
    await viewer.close();
    expect(viewerErrors).toEqual([]);
    expect(errors).toEqual([]);
  });

  test("P3WRO plaintext: an http page on a --tls on hub gets the TLS notice and nothing to start", async ({ page }) => {
    test.setTimeout(90_000);
    hub = await startHub(["--open", "--forget-clients"]);
    const session = await openHubScreen(page, hub, "http");
    const ro = session.getByRole("region", { name: "Firmware rollout" });
    const notice = ro.getByRole("alert", { name: "Encrypted connection needed" });
    await expect(notice).toHaveAttribute("data-key", "dlink.tls_required", { timeout: 30_000 });
    await expect(notice).toContainText("Connect to the hub with encryption on");
    await expect(ro.getByRole("button", { name: "Start Update…" })).toBeDisabled();
    await expect(ro.getByRole("button", { name: "Add Image…" })).toBeDisabled();
    await expect(ro.locator("tr.hub-image-row")).toHaveCount(0);
    await noSidewaysScroll(page);
  });

  test("P3WRO Control: the rollout is not on a Control client's hub screen", async ({ page }) => {
    test.setTimeout(90_000);
    hub = await startHub(["--pair"]);
    const session = await openHubScreen(page, hub, "https");
    await expect(session.locator(".hub-role-tag")).toHaveText("Control");
    await expect(session.getByRole("region", { name: "Health board" })).toBeVisible();
    await expect(session.getByRole("region", { name: "Firmware rollout" })).toHaveCount(0);
    await expect(session.getByText("Firmware rollout")).toHaveCount(0);
  });
});
