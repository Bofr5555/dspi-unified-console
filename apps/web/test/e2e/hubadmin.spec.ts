// WP HUBADMIN e2e: the Installer page edits the HUB's copy over DSPi Link when the DSPi is
// reached through a hub, on a real hub (`uconsole --demo hub`, DSPI_TERM), and this console's
// copy on a direct DSPi (the demo). What the hub keeps is read back from its store file.
//
// - An Admin (open access: every connection is an Admin) sees which copy is edited, changes the
//   hub's profile (a limit, the note, what the lock covers), a role's pinned mode and the hub's
//   installer PIN (two fields, the recovery code shown once, no current PIN asked; the hub
//   counts the Admin who set it as unlocked), and clears
//   them again; a second console sees the change through the hub's event, without a reload.
//   The hub's copy is never kept in the browser's own per-serial storage.
// - A Control client (PIN pairing) sees the page read-only, with the core's role row.
// - The demo (a direct DSPi) says it edits this console's copy, as before.

import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";

const DEMO = "DEMO0000DSPI0001";

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

const tab = (session: Locator, name: string) => session.getByRole("tab", { name, exact: true });

async function setMode(session: Locator, page: Page, mode: "Simple" | "Advanced" | "Installer") {
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: mode }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await expect(session.getByRole("button", { name: `Mode: ${mode}` })).toBeVisible();
}

async function installerPage(session: Locator, page: Page): Promise<void> {
  await setMode(session, page, "Installer");
  await tab(session, "Installer").click();
  await expect(session.getByRole("region", { name: "Limits" })).toBeVisible();
}

async function noSidewaysScroll(page: Page) {
  if (test.info().project.name !== "narrow") return;
  const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(over).toBeLessThanOrEqual(0);
}

// -- A hub of the test's own --

type OwnHub = { port: number; pin: string; child: ChildProcess; store: string };

function newStore(): string {
  const store = `${process.env.TMPDIR ?? "test-results"}/hubadmin-${process.pid}-${Math.random().toString(36).slice(2)}`;
  mkdirSync(store, { recursive: true });
  return store;
}

async function startHub(extra: string[], pairing: boolean, store: string): Promise<OwnHub> {
  const child = spawn(
    process.env.DSPI_TERM!,
    ["--demo", "hub", "--web", "--loopback", "--console", "dist", ...extra, "--port", "0", "--json", "--no-keep-awake", "--store", store],
    { stdio: ["ignore", "pipe", "inherit"] },
  );
  const lines = createInterface({ input: child.stdout! });
  let port = 0;
  const pin = await new Promise<string>((ok, fail) => {
    child.on("exit", (code) => fail(new Error(`uconsole hub exited ${code}`)));
    lines.on("line", (line) => {
      const e = JSON.parse(line) as { event?: string; port?: number; pin?: string };
      if (e.event === "status") {
        port = e.port ?? 0;
        if (!pairing) ok("");
      }
      if (e.event === "pairing" && e.pin) ok(e.pin);
    });
  });
  return { port, pin, child, store };
}

async function stopHub(h: OwnHub): Promise<void> {
  const exited = new Promise<void>((r) => h.child.once("exit", () => r()));
  h.child.kill();
  await exited;
}

/** What the hub keeps, from its store file ({} while it has none). */
function kept(h: OwnHub): Record<string, Record<string, unknown> | undefined> {
  try {
    return JSON.parse(readFileSync(`${h.store}/hub-store.json`, "utf8")) as Record<string, Record<string, unknown> | undefined>;
  } catch {
    return {};
  }
}
const limitsKept = (h: OwnHub) => (kept(h).device_limits?.[DEMO] ?? null) as Record<string, unknown> | null;

/** The console served by the hub: connect its entry (pairing with its PIN when it asks), open the demo DSPi. */
async function openHub(page: Page, hub: OwnHub): Promise<Locator> {
  await page.goto(`http://127.0.0.1:${hub.port}/console/`);
  const row = page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` });
  await row.getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  if (hub.pin) {
    await session.getByLabel("PIN").fill(hub.pin);
    await session.getByRole("button", { name: "Pair" }).click();
  }
  const entry = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: DEMO.slice(-8) });
  await entry.getByRole("button", { name: "Open" }).click();
  await expect(session.locator(".device-meta")).toContainText(DEMO);
  return session;
}

if (!process.env.DSPI_TERM) {
  test("HUBADMIN: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no uconsole binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("HUBADMIN on a hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
  });

  test("HUBADMIN Admin: edits the hub's profile, pinned mode and PIN, and sees which copy", async ({ page, context }) => {
    test.setTimeout(120_000);
    const errors = errorsOf(page);
    hub = await startHub(["--open", "--forget-clients"], false, newStore());
    const session = await openHub(page, hub);
    await installerPage(session, page);
    const copy = session.getByRole("note", { name: "Copy edited" });
    await expect(copy).toHaveAttribute("data-key", "installer.hub_copy");
    await expect(copy).toContainText("Editing the hub's copy");
    await expect(session.getByRole("note", { name: "Read-only" })).toHaveCount(0);
    await expect(session.locator(".hub-role")).toContainText("Admin");
    await noSidewaysScroll(page);

    // A second console on the same hub: it hears the hub's event and reads the page again.
    const other = await context.newPage();
    const otherErrors = errorsOf(other);
    const second = await openHub(other, hub);
    await installerPage(second, other);
    const otherPeq = second.getByRole("region", { name: "Limits" }).getByLabel(/^Highest PEQ boost/);
    await expect(otherPeq).toHaveValue("");

    // The profile: a limit and the note, kept by the hub, never in this browser's own storage.
    const limits = session.getByRole("region", { name: "Limits" });
    const peq = limits.getByLabel(/^Highest PEQ boost/);
    await peq.fill("3");
    await peq.dispatchEvent("change");
    await expect.poll(() => limitsKept(hub!)?.maxPeqBoostDb).toBe(3);
    await expect(otherPeq).toHaveValue("3");
    const note = limits.getByLabel("Note for whoever looks after the system");
    await note.fill("Hub note");
    await note.dispatchEvent("change");
    await expect.poll(() => limitsKept(hub!)?.note).toBe("Hub note");
    expect(await page.evaluate(() => localStorage.getItem("dspi-web.limits") ?? "")).not.toContain("DEMO0000DSPI0001");

    // What the lock covers is the hub's profile's lockedPages.
    const covers = session.getByRole("region", { name: "Installer lock" }).getByRole("group", { name: "What the lock covers" });
    const first = covers.getByRole("checkbox").first();
    // (a click: the box follows the hub's answer, not the click)
    await first.click();
    await expect.poll(() => ((limitsKept(hub!)?.lockedPages as string[] | undefined) ?? []).length).toBe(1);
    await expect(first).toBeChecked();
    await first.click();
    await expect.poll(() => ((limitsKept(hub!)?.lockedPages as string[] | undefined) ?? []).length).toBe(0);

    // A pinned mode per role.
    const modes = session.getByRole("region", { name: "Pinned modes" });
    for (const role of ["Viewer", "Control", "Admin"]) await expect(modes.getByLabel(role, { exact: true })).toBeEnabled();
    await modes.getByLabel("Control", { exact: true }).selectOption("simple");
    await expect.poll(() => kept(hub!).role_modes?.control).toBe("simple");
    await expect(modes.getByLabel("Control", { exact: true })).toHaveValue("simple");
    await modes.getByLabel("Control", { exact: true }).selectOption("");
    await expect.poll(() => kept(hub!).role_modes?.control ?? null).toBe(null);

    // The hub's PIN: two fields, no current PIN asked; the recovery code shown once.
    const pinPanel = session.getByRole("region", { name: "Hub installer PIN" });
    await pinPanel.getByLabel("New PIN", { exact: true }).fill("1357");
    await pinPanel.getByLabel("New PIN again").fill("1375");
    await pinPanel.getByRole("button", { name: "Set PIN" }).click();
    await expect(pinPanel.getByRole("alert")).toContainText("not the same");
    await pinPanel.getByLabel("New PIN", { exact: true }).fill("1357");
    await pinPanel.getByLabel("New PIN again").fill("1357");
    await pinPanel.getByRole("button", { name: "Set PIN" }).click();
    const recovery = session.getByRole("status", { name: "Recovery code" });
    await expect(recovery).toHaveAttribute("data-key", "installer_lock.hub_recovery");
    await expect(recovery).toContainText("Keep the recovery code");
    await expect(recovery.locator("code")).not.toBeEmpty();
    await expect.poll(() => !!kept(hub!).device_pins?.[DEMO]).toBe(true);
    await noSidewaysScroll(page);
    await recovery.getByRole("button", { name: "Done" }).click();
    await expect(recovery).toHaveCount(0);

    // The hub counts the Admin who set it as unlocked; the Admin clears it again.
    await expect(pinPanel.locator(".lock-state")).toContainText("An installer PIN is set on the hub; this console has unlocked it.");
    await pinPanel.getByRole("button", { name: "Clear PIN" }).click();
    await expect(pinPanel.locator(".lock-state")).toContainText("No installer PIN is set on the hub");
    await expect.poll(() => !!kept(hub!).device_pins?.[DEMO]).toBe(false);

    // Clear Profile: the hub keeps none.
    await session.getByRole("region", { name: "Limits" }).getByRole("button", { name: "Clear Profile" }).click();
    await expect.poll(() => limitsKept(hub!)).toBe(null);
    await expect(otherPeq).toHaveValue("");
    expect(errors).toEqual([]);
    expect(otherErrors).toEqual([]);
  });

  test("HUBADMIN Control: a client paired with the hub's PIN sees the hub's copy read-only, with the role row", async ({ page }) => {
    test.setTimeout(90_000);
    const errors = errorsOf(page);
    // The hub keeps a profile for the demo DSPi (as an Admin set it).
    const store = newStore();
    await stopHub(await startHub(["--pair"], true, store));
    const file = `${store}/hub-store.json`;
    const data = JSON.parse(readFileSync(file, "utf8")) as Record<string, unknown>;
    data.device_limits = { [DEMO]: { maxPeqBoostDb: 4, note: "Kept by the hub." } };
    writeFileSync(file, JSON.stringify(data, null, 2));
    hub = await startHub(["--pair"], true, store);

    const session = await openHub(page, hub);
    await installerPage(session, page);
    await expect(session.getByRole("note", { name: "Copy edited" })).toHaveAttribute("data-key", "installer.hub_copy");
    const ro = session.getByRole("note", { name: "Read-only" });
    await expect(ro).toHaveAttribute("data-key", "installer.hub_role");
    await expect(ro).toContainText("Read-only: Admin needed");
    await expect(session.locator(".hub-role")).toContainText("Control");
    const limits = session.getByRole("region", { name: "Limits" });
    await expect(limits.getByLabel(/^Highest PEQ boost/)).toHaveValue("4");
    await expect(limits.getByLabel(/^Highest PEQ boost/)).toBeDisabled();
    await expect(limits.getByLabel("Note for whoever looks after the system")).toHaveValue("Kept by the hub.");
    await expect(limits.getByLabel("Note for whoever looks after the system")).toBeDisabled();
    await expect(limits.getByRole("button", { name: "Load Profile…" })).toBeDisabled();
    await expect(limits.getByRole("button", { name: "Clear Profile" })).toBeDisabled();
    const pinPanel = session.getByRole("region", { name: "Hub installer PIN" });
    await expect(pinPanel.getByLabel("New PIN", { exact: true })).toBeDisabled();
    await expect(pinPanel.getByRole("button", { name: "Set PIN" })).toBeDisabled();
    const modes = session.getByRole("region", { name: "Pinned modes" });
    await expect(modes.getByLabel("Control", { exact: true })).toBeDisabled();
    await expect(modes.getByLabel("Admin", { exact: true })).toHaveCount(0);
    await expect(session.getByRole("region", { name: "Installer lock" }).getByRole("group", { name: "What the lock covers" }).getByRole("checkbox").first()).toBeDisabled();
    await noSidewaysScroll(page);
    // Nothing changed on the hub.
    expect(limitsKept(hub)).toEqual({ maxPeqBoostDb: 4, note: "Kept by the hub." });
    expect(errors).toEqual([]);
  });
});

test("HUBADMIN direct: the demo DSPi edits this console's copy, with no hub parts", async ({ page }) => {
  const errors = errorsOf(page);
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(session.getByRole("region", { name: "Presets" })).toBeVisible();
  await installerPage(session, page);
  const copy = session.getByRole("note", { name: "Copy edited" });
  await expect(copy).toHaveAttribute("data-key", "installer.console_copy");
  await expect(copy).toContainText("Editing this console's copy");
  await expect(session.getByRole("note", { name: "Read-only" })).toHaveCount(0);
  await expect(session.getByRole("region", { name: "Hub installer PIN" })).toHaveCount(0);
  await expect(session.getByRole("region", { name: "Pinned modes" })).toHaveCount(0);
  await expect(session.getByRole("region", { name: "Installer lock" })).toContainText("No installer PIN is set: nothing is locked.");
  await expect(session.getByRole("region", { name: "Limits" }).getByLabel(/^Highest PEQ boost/)).toBeEnabled();
  await noSidewaysScroll(page);
  expect(errors).toEqual([]);
});
