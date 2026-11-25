// WP UWN e2e (UXP A15, A16) over DSPi Link, on the real core and dspi-link's fake hub (tools/hub-host.mjs, as
// link.spec.ts): devices you can tell apart (the chooser's rows, two tabs of one name), and recovery that tells the
// truth (writes fenced while the link is away, nothing red left once it is back, a hub that now answers as another,
// a hub that cannot be reached).

import { expect, test, type APIRequestContext, type Locator, type Page } from "@playwright/test";
import { openDevices } from "../nav";

const HUB = `http://127.0.0.1:${Number(process.env.HUB_PORT ?? 8738)}`;
const PLAIN = `http://127.0.0.1:${Number(process.env.PORT ?? 8737)}`;
/** The fake hub's DSPi ("Bench"), and a second unit on another hub. */
const SERIAL = "0123456789ABCDEF";
const SERIAL_B = "FEDCBA9876543210";

test.skip(!process.env.DSPI_FAKEHUB, "no fake hub (set DSPI_FAKEHUB)");

async function newHub(request: APIRequestContext, args: string): Promise<number> {
  const r = await request.post(`${HUB}/__hub?args=${encodeURIComponent(args)}`);
  expect(r.ok()).toBe(true);
  return ((await r.json()) as { id: number }).id;
}

async function fault(request: APIRequestContext, id: number, line: string): Promise<void> {
  const r = await request.post(`${HUB}/__fault?hub=${id}`, { data: line });
  expect(await r.text()).toBe("ok");
}

const hubAddress = (id: number) => `ws://127.0.0.1:${new URL(HUB).port}/h/${id}/dspi/v1`;

/** The page served by hub `id`, its hub connected, its DSPi chosen and open. */
async function openOnHub(page: Page, id: number): Promise<Locator> {
  await page.goto(`${HUB}/?hub=${id}`);
  await page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: "Hub 127.0.0.1:" }).getByRole("button", { name: /^Connect Hub 127\.0\.0\.1:/ }).click();
  const session = page.getByRole("article");
  await session.getByRole("list", { name: /DSPis on/ }).getByRole("button", { name: /^Open Bench/ }).click();
  await expect(session).toHaveAttribute("data-serial", new RegExp(SERIAL));
  await expect(session.locator(".banner")).toHaveCount(0);
  return session;
}

test("UWN A15: the hub's list names each DSPi with its state and the part of its serial that tells it apart", async ({ page, request }) => {
  const id = await newHub(request, "--auth none");
  await page.goto(`${HUB}/?hub=${id}`);
  await page.getByRole("navigation", { name: "DSPi devices" }).getByRole("button", { name: /^Connect Hub 127\.0\.0\.1:/ }).click();
  const row = page.getByRole("article").getByRole("list", { name: /DSPis on/ }).getByRole("listitem");
  await expect(row).toHaveCount(1);
  await expect(row.locator(".source-name")).toHaveText("Bench");
  await expect(row.locator(".source-meta")).toHaveText("online · 89ABCDEF");
  await expect(row.getByRole("button")).toHaveAccessibleName("Open Bench, 89ABCDEF");
});

test("UWN A15: one installation: the tab is the device's name over its hub and firmware; two sites: each tab says its site", async ({ page, request }) => {
  const a = await newHub(request, "--auth none --name Stage");
  const session = await openOnHub(page, a);
  const tabs = page.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab");
  const name = (await tabs.first().locator(".device-tab-name").innerText()).trim();
  expect(name.length).toBeGreaterThan(0);
  // No installation ("This console") while one is open: the hub's name and the firmware under the name
  await expect(tabs.first().locator(".device-tab-kind")).toHaveText(/^Stage · firmware /);
  // The two lines with a separator, never run together
  await expect(tabs.first()).toHaveAccessibleName(new RegExp(`^${name}, Stage · firmware \\S+$`));
  await expect(session.getByRole("complementary", { name: "Status" }).getByLabel("Connection", { exact: true })).toHaveText(/^Connected/);

  // Two sites, two DSPis of the same name: the site says which
  const b = await newHub(request, `--auth none --serial ${SERIAL_B} --name Bar`);
  const q = new URLSearchParams({ a: hubAddress(a), b: hubAddress(b), device: SERIAL });
  await page.goto(`${HUB}/test/fleet-hub.html?${q}`);
  await page.getByRole("region", { name: "Site A" }).getByRole("button", { name: /^Connect Stage rack/ }).click();
  await expect(page.getByRole("article")).toHaveAttribute("data-serial", new RegExp(SERIAL));
  await openDevices(page);
  await page.getByRole("region", { name: "Site B" }).getByRole("button", { name: /^Connect Hub/ }).click();
  await page.getByRole("article").getByRole("list", { name: /DSPis on/ }).getByRole("button", { name: /^Open Bench/ }).click();
  await expect(page.getByRole("article")).toHaveAttribute("data-serial", new RegExp(SERIAL_B));
  await expect(tabs).toHaveCount(2);
  await expect(tabs.nth(0)).toHaveAccessibleName(new RegExp(`^${name}, Site A · Stage · firmware `));
  await expect(tabs.nth(1)).toHaveAccessibleName(new RegExp(`^${name}, Site B · Bar · firmware `));
});

test("UWN A16: while the link is away nothing writes and the rail says Reconnecting; back, nothing red is left", async ({ page, request }) => {
  const id = await newHub(request, "--auth none");
  const session = await openOnHub(page, id);
  const rail = session.getByRole("complementary", { name: "Status" });
  await fault(request, id, "refuse 503");
  await fault(request, id, "drop");
  await expect(session.locator(".banner")).toContainText("Connection lost, reconnecting");
  // Fenced: the dock's Mute and the top bar's Save wait for the link; the rail's dot and words agree
  await expect(session.getByRole("button", { name: "Mute" }).first()).toBeDisabled();
  await expect(session.getByRole("button", { name: /^Save to Device/ })).toBeDisabled();
  await expect(rail.getByLabel("Connection", { exact: true })).toHaveText(/^Reconnecting/);
  await expect(rail.locator(".dot")).toHaveClass(/dot-away/);
  // A write past the page (the key) is refused by the session without a failure notice
  await page.keyboard.press("Control+m").catch(() => undefined);
  await expect(session.getByRole("alert")).toHaveCount(0);
  await fault(request, id, "refuse ok");
  await expect(session.locator(".banner")).toHaveCount(0, { timeout: 20_000 });
  await expect(session.getByRole("alert")).toHaveCount(0);
  await expect(rail.getByLabel("Connection", { exact: true })).toHaveText(/^Connected/);
  await expect(rail.locator(".dot")).toHaveClass(/dot-open/);
  await expect(session.getByRole("button", { name: "Mute" }).first()).toBeEnabled();
});

test("UWN A16: a hub that now answers as another hub is said so, with Connect Again", async ({ page, request }) => {
  const id = await newHub(request, "--auth none");
  const session = await openOnHub(page, id);
  // The hub is reset to a new identity: the core's redials find another hub at the address
  await fault(request, id, "reset 0b5e1c2d-3e4f-4a5b-8c6d-7e8f9a0b1c2d");
  await fault(request, id, "drop");
  const other = session.getByRole("alert").filter({ hasText: "now answers as a different hub" });
  await expect(other).toBeVisible({ timeout: 20_000 });
  await expect(other).toContainText(`The hub at 127.0.0.1:${new URL(HUB).port}`);
  await other.getByRole("button", { name: "Connect Again" }).click();
  // A new session for the same DSPi: the hub (no PIN) opens it again
  const again = page.getByRole("article");
  await expect(again).toHaveAttribute("data-serial", new RegExp(SERIAL), { timeout: 15_000 });
  await expect(again.locator(".banner")).toHaveCount(0);
  await expect(page.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab")).toHaveCount(1);
});

test("UWN A16: a hub that cannot be reached stays on Devices with Try Again and Remove, and no second strip", async ({ page }) => {
  await page.goto(`${PLAIN}/`);
  const nav = page.getByRole("navigation", { name: "DSPi devices" });
  await nav.getByRole("textbox", { name: "Add a hub" }).fill("127.0.0.1:9");
  await nav.getByRole("button", { name: "Add" }).click();
  await nav.getByRole("button", { name: "Connect Hub 127.0.0.1:9" }).click();
  const failure = page.getByRole("alert", { name: "Could not connect to 127.0.0.1:9" });
  await expect(failure).toBeVisible({ timeout: 20_000 });
  await expect(page.getByText("No DSPi is connected. Connect one over USB, or try the demo device.")).toHaveCount(0);
  // Still on Devices, with the list under the line
  await expect(nav).toBeVisible();
  await expect(failure.getByRole("button", { name: "Try Again" })).toBeVisible();
  await failure.getByRole("button", { name: "Remove" }).click();
  await expect(nav.getByRole("listitem").filter({ hasText: "127.0.0.1:9" })).toHaveCount(0);
  await expect(page.getByRole("tablist", { name: "Open DSPi" })).toHaveCount(0);
});
