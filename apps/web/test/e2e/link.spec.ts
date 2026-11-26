// WP WB4b e2e: DSPi Link in the browser, on the real core (crates/dspi-wasm: our
// dspi-link client over the browser's WebSocket), in Chromium and Firefox.
//
// The page is served by a hub (tools/hub-host.mjs) whose WebSocket enforces Link
// spec 4's Origin rule, so the page reaches it because it is the hub's own page.
// Behind it, each test gets fake hubs of its own: dspi-link's `dspi-link-fakehub`
// (DSPI_FAKEHUB), with its fault commands. Every banner here comes from the
// core's real Link phases, in CG10's wording.

import { expect, test, type APIRequestContext, type Page } from "@playwright/test";
import { openDevices } from "../nav";

const linkWords = (await import("../../../../ci/generated/web/link-words.json", { with: { type: "json" } })).default as {
  link: { key: string; text: string; action: string | null }[];
};
const words = (key: string) => linkWords.link.find((w) => w.key === key)!;

const HUB = `http://127.0.0.1:${Number(process.env.HUB_PORT ?? 8738)}`;
const PLAIN = `http://127.0.0.1:${Number(process.env.PORT ?? 8737)}`;
/** The fake hub's DSPi (crates/dspi-link/src/fakehub.rs `HubConfig::default`). */
const SERIAL = "0123456789ABCDEF";
/** -- WP WEBM1 -- a second unit, on another hub (`--serial`). */
const SERIAL_B = "FEDCBA9876543210";

if (!process.env.DSPI_FAKEHUB) {
  test("WB4b: the fake hub is there", () => {
    test.skip(!process.env.DSPI_FAKEHUB_REQUIRED, "no fake hub (set DSPI_FAKEHUB)");
    throw new Error("DSPI_FAKEHUB_REQUIRED, but no DSPI_FAKEHUB");
  });
}
test.skip(!process.env.DSPI_FAKEHUB, "no fake hub (set DSPI_FAKEHUB)");

/** A fake hub of this test's own; its id on the host. */
async function newHub(request: APIRequestContext, args: string): Promise<number> {
  const r = await request.post(`${HUB}/__hub?args=${encodeURIComponent(args)}`);
  expect(r.ok()).toBe(true);
  return ((await r.json()) as { id: number }).id;
}

/** A fault command to hub `id`, once it has taken effect. */
async function fault(request: APIRequestContext, id: number, line: string): Promise<void> {
  const r = await request.post(`${HUB}/__fault?hub=${id}`, { data: line });
  expect(await r.text()).toBe("ok");
}

/** A call on the n-th session the page opened, straight to its core. */
function core(page: Page, n: number, method: string, params: unknown = null) {
  return page.evaluate(
    ([n, method, params]) =>
      (globalThis as unknown as { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] }).dspiClients[n as number]!.call(
        method as string,
        params,
      ),
    [n, method, params] as const,
  );
}

const hubAddress = (id: number) => `ws://127.0.0.1:${new URL(HUB).port}/h/${id}/dspi/v1`;

async function noPageErrors(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    // A browser logs a WebSocket that fails or drops; that is the test's own doing.
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

/** The standalone page served by hub `id`: Connect on its own entry. */
async function connectServingHub(page: Page, id: number) {
  await page.goto(`${HUB}/?hub=${id}`);
  const row = page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:` });
  await expect(row).toHaveCount(1);
  await row.getByRole("button", { name: "Connect" }).click();
  return page.getByRole("article");
}

// -- WP WEBM1 --
test("one session per DSPi: another entry for a DSPi that is open shows its tab and says so; once closed, it opens", async ({ page, request }) => {
  const errors = await noPageErrors(page);
  const a = await newHub(request, "--auth none");
  // Both sites list the same hub's DSPi.
  const q = new URLSearchParams({ a: hubAddress(a), b: hubAddress(a), device: SERIAL });
  await page.goto(`${HUB}/test/fleet-hub.html?${q}`);
  const entry = (site: string) => page.getByRole("region", { name: site }).getByRole("listitem").filter({ hasText: "Stage rack" }).getByRole("button", { name: "Connect" });
  await entry("Site A").click();
  const session = page.getByRole("article");
  await expect(session).toHaveAttribute("data-serial", new RegExp(SERIAL));
  const tabs = page.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab");
  await openDevices(page);
  // Twice at once: still one session.
  await entry("Site B").dblclick({ noWaitAfter: true }).catch(() => undefined);
  await expect(page.getByText(/^This DSPi is already open: Site A · .*, Hub$/)).toBeVisible();
  await expect(tabs).toHaveCount(1);
  await expect(tabs.first()).toHaveAttribute("aria-selected", "true");
  expect(await page.evaluate(() => (globalThis as unknown as { dspiClients: unknown[] }).dspiClients.length)).toBe(1);
  await session.getByRole("tablist", { name: "Open DSPi" }).getByRole("button", { name: /^Close / }).click(); // -- WP REVEFIX -- the tab's ×
  await expect(tabs).toHaveCount(0);
  await entry("Site B").click();
  await expect(session).toHaveAttribute("data-serial", new RegExp(SERIAL));
  await expect(tabs).toHaveCount(1);
  expect(errors).toEqual([]);
});

async function chooseAndOpen(session: ReturnType<Page["getByRole"]>, serial = SERIAL) {
  await expect(session.locator(".banner")).toContainText(words("choose_device").text);
  const entry = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: serial.slice(-8) });
  await entry.getByRole("button", { name: "Open" }).click();
  await expect(session).toHaveAttribute("data-serial", new RegExp(serial));
  await expect(session.locator(".banner")).toHaveCount(0);
}

async function writeVolume(session: ReturnType<Page["getByRole"]>, db: string) {
  const slider = session.getByRole("slider", { name: "Volume" });
  // -- WP WB6a -- the main window's slider is the Console's taper: the position of `db`
  // (the values below are ones the taper reaches exactly)
  await slider.fill(String(Math.round(((Number(db) + 60) / 60) ** 2 * 200)));
  await slider.dispatchEvent("change");
  await expect(session.locator(".volume-value")).toHaveText(`${Number(db).toFixed(1)} dB`);
}

test("served by its hub: pairs with the PIN, lists the hub's DSPis, opens one, writes with read-back", async ({ page, request }) => {
  const errors = await noPageErrors(page);
  const id = await newHub(request, "--auth fixed:246810");
  const session = await connectServingHub(page, id);

  // CG10's needs_pairing wording for a hub with its own PIN; a wrong PIN keeps the prompt.
  await expect(session.locator(".banner")).toContainText(words("needs_pairing.fixed").text);
  await session.getByLabel("PIN").fill("111111");
  await session.getByRole("button", { name: "Pair" }).click();
  await expect(session.getByRole("alert")).toContainText("wrong PIN");
  await session.getByLabel("PIN").fill("246810");
  await session.getByRole("button", { name: "Pair" }).click();

  await chooseAndOpen(session);
  await writeVolume(session, "-12");

  // The token was kept for this hub: the next visit asks no PIN.
  const stored = await page.evaluate(() => localStorage.getItem("dspi-web.hub-tokens.v1") ?? "");
  expect(stored).toContain(hubAddress(id));
  await page.reload();
  const again = await connectServingHub(page, id);
  await expect(again.getByLabel("PIN")).toHaveCount(0);
  await chooseAndOpen(again);
  expect(errors).toEqual([]);
});

test("lost link: the core's reconnecting phase with its way out, back by itself; a hub that ends it is closed", async ({ page, request }) => {
  const errors = await noPageErrors(page);
  const id = await newHub(request, "--auth none");
  const session = await connectServingHub(page, id);
  await chooseAndOpen(session);
  await writeVolume(session, "-6");

  // The link drops; the hub turns new connections away for a while (a page cannot
  // see why, so the core retries with its plain backoff), so the banner stays.
  await fault(request, id, "refuse 503");
  await fault(request, id, "drop");
  const reconnecting = words("reconnecting");
  const banner = session.locator(".banner");
  await expect(banner).toContainText(reconnecting.text);
  await expect(banner.getByRole("button", { name: reconnecting.action! })).toBeVisible();
  // The mirror stays, last known.
  await expect(session.locator(".volume-value")).toHaveText("-6.0 dB");

  // The hub takes connections again: the core reconnects and binds the DSPi by itself.
  await fault(request, id, "refuse ok");
  await expect(banner).toHaveCount(0, { timeout: 20_000 });
  await writeVolume(session, "-24");

  // 4000: the hub ended the session for good.
  await fault(request, id, "close 4000");
  await expect(banner).toContainText(words("closed").text);
  await expect(banner.getByRole("button")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("reconnecting: its Disconnect leaves the hub", async ({ page, request }) => {
  const id = await newHub(request, "--auth none");
  const session = await connectServingHub(page, id);
  await chooseAndOpen(session);
  await fault(request, id, "refuse 503");
  await fault(request, id, "drop");
  const reconnecting = words("reconnecting");
  await session.locator(".banner").getByRole("button", { name: reconnecting.action! }).click();
  await expect(page.getByText("No DSPi is connected. Connect one over USB, or try the demo device.")).toBeVisible();
});

test("the Origin rule: a page from another site is refused by the hub", async ({ page, request }) => {
  const id = await newHub(request, "--auth none");
  // The same fleet page, served by another origin than the hub's.
  await page.goto(`${PLAIN}/test/fleet-hub.html?a=${encodeURIComponent(hubAddress(id))}&b=${encodeURIComponent(hubAddress(id))}`);
  await page.getByRole("region", { name: "Site B" }).getByRole("button", { name: "Connect" }).click();
  await expect(page.getByRole("article").getByRole("alert")).toContainText("a hub accepts only pages it serves itself");
});

test("fleet embedding: hubDevices names the DSPi, hubCredentials supplies the token; two hubs at once, one session per DSPi", async ({ page, request }) => {
  const errors = await noPageErrors(page);
  const a = await newHub(request, "--auth fixed:135790");
  const b = await newHub(request, `--auth none --serial ${SERIAL_B}`);

  // A token for hub a, as a fleet server would hold one: paired once on the hub's own page.
  const first = await connectServingHub(page, a);
  await first.getByLabel("PIN").fill("135790");
  await first.getByRole("button", { name: "Pair" }).click();
  await expect(first.locator(".banner")).toContainText(words("choose_device").text);
  const saved = JSON.parse(await page.evaluate(() => localStorage.getItem("dspi-web.hub-tokens.v1") ?? "{}")) as Record<
    string,
    { token: string; hubId: string }
  >;
  const cred = saved[hubAddress(a)]!;
  expect(cred.token.length).toBeGreaterThan(0);

  const q = new URLSearchParams({ a: hubAddress(a), b: hubAddress(b), token: cred.token, hubId: cred.hubId, device: SERIAL });
  await page.goto(`${HUB}/test/fleet-hub.html?${q}`);
  await expect(page.getByText("Operator")).toBeVisible();

  // Site A: the fleet's own entry for the DSPi; it opens with the fleet's token, no PIN, no list.
  await page.getByRole("region", { name: "Site A" }).getByRole("listitem").filter({ hasText: "Stage rack" }).getByRole("button", { name: "Connect" }).click();
  const one = page.getByRole("article");
  await expect(one).toHaveAttribute("data-serial", new RegExp(SERIAL));
  await expect(one.getByLabel("PIN")).toHaveCount(0);
  await writeVolume(one, "-36");
  expect(await page.evaluate(() => (globalThis as unknown as { fleet: { asked: string[] } }).fleet.asked)).toContain(hubAddress(a));
  // A write the hub's DSPi keeps, read back.
  expect(await core(page, 0, "write", { path: "vol.master", value: { float: -7.5 } })).toEqual({ float: -7.5 });
  // The link drops and stays down (the hub busy). A write now fails at once and is
  // never sent later (the owner's rule); meanwhile the DSPi changes at the hub.
  await fault(request, a, "refuse 503");
  await fault(request, a, "drop");
  await expect(one.locator(".banner")).toContainText(words("reconnecting").text);
  const refused = await page.evaluate(() =>
    (globalThis as unknown as { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] }).dspiClients[0]!.call("write", {
      path: "vol.master",
      value: { float: -20 },
    }).then(
      () => "written",
      (e: { code: string }) => e.code,
    ),
  );
  expect(refused).not.toBe("written");
  await fault(request, a, "change 4700 0000a0c0 1 0"); // -5.0 dB, made at the hub
  await fault(request, a, "refuse ok");
  await expect(one.locator(".banner")).toHaveCount(0, { timeout: 20_000 });
  // Read again from the hub after the core's own reconnect: its -5, not our -20.
  expect(((await core(page, 0, "state")) as { state: { masterVolumeDb: number } }).state.masterVolumeDb).toBe(-5);

  // Site B: no list from the fleet, so the console asks the hub.
  const siteB = page.getByRole("region", { name: "Site B" }).getByRole("button", { name: "Connect" });
  await openDevices(page);
  await siteB.click();
  await chooseAndOpen(page.getByRole("article"), SERIAL_B);
  // -- WP WEBM1 -- one session per DSPi: hub b's again is refused, in a session that goes on asking.
  await openDevices(page);
  await siteB.click();
  const third = page.getByRole("article");
  await third.getByRole("listitem").filter({ hasText: SERIAL_B.slice(-8) }).getByRole("button", { name: "Open" }).click();
  await expect(third.getByRole("alert")).toHaveText(/^This DSPi is already open: Site B · .*, Hub$/);
  await expect(third).toHaveAttribute("data-status", "choosing");
  // The session that has it goes: the third may open it.
  await page.getByRole("button", { name: /^Close / }).nth(1).click();
  await expect(page.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab")).toHaveCount(2);
  await page.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab").nth(1).click();
  await chooseAndOpen(page.getByRole("article"), SERIAL_B);
  // -- WP WB6b -- the sessions' tabs (a session's own screens have tabs too)
  await expect(page.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab")).toHaveCount(2);

  // Each session is its own: hub b drops, hub a's session stays bound.
  await fault(request, b, "refuse 503");
  await fault(request, b, "drop");
  await expect(page.getByRole("article").locator(".banner")).toContainText(words("reconnecting").text);
  await page.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab").first().click();
  // (The user volume shows what the hub re-read gave: the fake hub keeps only the master volume.)
  await expect(page.getByRole("article")).toHaveAttribute("data-serial", new RegExp(SERIAL));
  await expect(page.getByRole("article").locator(".banner")).toHaveCount(0);
  expect(((await core(page, 0, "state")) as { state: { masterVolumeDb: number } }).state.masterVolumeDb).toBe(-5);
  expect(errors).toEqual([]);
});
