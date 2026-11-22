// WP WB5 e2e: the standalone page and the embedding page, in Chromium and
// Firefox (and Chromium at 375 px wide). Since WB3 on the real core
// (crates/dspi-wasm) and its own demo device: name "DSPi", firmware 1.1.6, the
// user volume at 0 dB, a silent meter capture, an analyser with frames. The
// lost-link banner is in link.spec.ts (WB4b), on the core's real Link phases.

import { readFileSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";
import { appMenu, openDevices } from "../nav";

type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };

/** A call on the n-th session the embedding page opened, straight to its core. */
// -- BUG1 -- An output's name on the page is its label ("Out 3 · R · Full"), which follows the
// routing and the filters while the device keeps the slot's default name: matched by its number.
const OUT = (o: number) => `Out ${o + 1} · [^·]+ · \\w+`;
const re = (s: string) => new RegExp(`^${s}$`);

function core(page: Page, n: number, method: string, params: unknown = null) {
  return page.evaluate(
    ([n, method, params]) => (globalThis as unknown as Clients).dspiClients[n as number]!.call(method as string, params),
    [n, method, params] as const,
  );
}

/** The same, for a call that must fail: its code and message. */
function coreRefusal(page: Page, n: number, method: string, params: unknown = null) {
  return page.evaluate(
    ([n, method, params]) =>
      (globalThis as unknown as Clients).dspiClients[n as number]!.call(method as string, params).then(
        () => null,
        (e: { code: string; message: string }) => ({ code: e.code, message: e.message }),
      ),
    [n, method, params] as const,
  );
}

async function noPageErrors(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

test("standalone: the core's demo connects, a volume write reads back, meters are read", async ({ page }) => {
  const errors = await noPageErrors(page);
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "uConsole for DSPi" })).toBeVisible();
  const picker = page.getByRole("navigation", { name: "DSPi devices" });
  await picker.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();

  const session = page.getByRole("article", { name: "DSPi" });
  await expect(session.locator(".device-meta")).toHaveText("RP2350 · firmware 1.1.6 · DEMO0000DSPI0001");
  await expect(session.locator(".volume-value")).toHaveText("0.0 dB");

  // -- WP WB6a -- the main window's slider is the Console's taper: position 128 of 200 is -12 dB
  const slider = session.getByRole("slider", { name: "Volume" });
  await slider.fill("128");
  await slider.dispatchEvent("change");
  await expect(session.locator(".volume-value")).toHaveText("-12.0 dB");

  // The demo's meter capture is silent: every channel the core named, at the floor.
  await expect(session.getByRole("meter")).toHaveCount(17);
  await expect(session.getByRole("meter", { name: "USB 1" })).toHaveAttribute("aria-valuenow", "-90");
  await expect(session.getByRole("meter", { name: re(OUT(8)) })).toBeVisible();

  await session.getByRole("button", { name: "Disconnect" }).click();
  await expect(page.getByText("Choose a DSPi to connect to.")).toBeVisible();
  expect(errors).toEqual([]);
});

test("standalone: USB and hubs say why they are not offered, in words without a device noun", async ({ page, browserName }) => {
  await page.goto("/");
  const usb = page.getByRole("listitem").filter({ hasText: "DSPi on USB" });
  // -- WP WB4a -- Chromium on a secure page offers USB (the browser's picker); Firefox says why not.
  if (browserName === "chromium") {
    await expect(usb.getByRole("button", { name: "Connect" })).toBeEnabled();
    return;
  }
  await expect(usb.getByRole("button")).toHaveCount(0);
  const reason = (await usb.locator(".unavailable").textContent()) ?? "";
  expect(reason.length).toBeGreaterThan(10);
  expect(reason).not.toMatch(/\b(computer|phone|tablet|laptop|desktop)\b/i);
});

test("standalone: an added hub is listed and kept across a reload", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Add a hub").fill("studio.local:11915");
  await page.getByRole("button", { name: "Add" }).click();
  await expect(page.getByText("Hub studio.local:11915")).toBeVisible();
  await page.reload();
  await expect(page.getByText("Hub studio.local:11915")).toBeVisible();
});

test("standalone ?readonly: the volume control is off and says why", async ({ page }) => {
  await page.goto("/?readonly");
  await page.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article", { name: "DSPi" });
  await expect(session.getByText("Read only", { exact: true })).toBeVisible();
  await expect(session.getByRole("slider", { name: "Volume" })).toBeDisabled();
  await expect(session.locator(".read-only-reason")).toHaveText("You can view this DSPi but not change it");
});

test("About: Weeb Labs' links are marked as theirs, from assets/about.json", async ({ page }) => {
  await page.goto("/");
  await appMenu(page, "About");
  const about = page.getByRole("dialog", { name: "About" });
  await expect(about.getByRole("heading", { name: "Weeb Labs: links and support" })).toBeVisible();
  await expect(about.getByText("This console is made by Bofr5555, not Weeb Labs.", { exact: false })).toBeVisible();
  await expect(about.getByRole("link", { name: "Discord" })).toHaveAttribute("rel", /noopener/);
  await about.getByRole("button", { name: "Close" }).click();
  await expect(about).toHaveCount(0);
});

// -- WP LIC1 --
test("About: the licence notice and its three section 7 terms, from assets/about.json", async ({ page }) => {
  const licence = JSON.parse(readFileSync(new URL("../../../../assets/about.json", import.meta.url), "utf8")).licence;
  await page.goto("/");
  await appMenu(page, "About");
  const about = page.getByRole("dialog", { name: "About" });
  await expect(about.getByRole("heading", { name: licence.title, exact: true })).toBeVisible();
  await expect(about.getByText(licence.statement)).toBeVisible();
  await expect(about.getByRole("heading", { name: licence.terms_title })).toBeVisible();
  const terms = about.locator("ol.terms li");
  await expect(terms).toHaveCount(3);
  for (const [i, t] of licence.terms.entries()) {
    await expect(terms.nth(i)).toHaveText(`${t.title}. ${t.text}`);
  }
  await expect(about.getByText(licence.freedoms)).toBeVisible();
  await expect(about.getByText("DSPi and its firmware by Weeb Labs", { exact: true })).toBeVisible();
  await expect(about.getByRole("link", { name: "Third-party notices" })).toHaveAttribute("href", "THIRD-PARTY-NOTICES.txt");
});

test("embedded: two consoles on one page, each with its user, roles and sessions", async ({ page }) => {
  const errors = await noPageErrors(page);
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  const two = page.locator("#two");
  await expect(one.getByText("Operator One")).toBeVisible();
  await expect(two.getByText("Operator Two")).toBeVisible();
  // The server manages the list: no add-hub form.
  await expect(page.getByLabel("Add a hub")).toHaveCount(0);

  // Console one: Venue A with control, Venue B view only, both open at once.
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  await expect(one.getByRole("article", { name: "DSPi" }).getByRole("slider", { name: "Volume" })).toBeEnabled();
  await openDevices(one); // -- WP WEBM1 --
  await one.getByRole("region", { name: "Venue B" }).getByRole("button", { name: "Connect" }).click();
  // -- WP WB6b -- the sessions' tabs (a session's own screens have tabs too)
  await expect(one.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab")).toHaveCount(2);
  await expect(one.getByRole("tab", { name: "Venue B · DSPi" })).toHaveAttribute("aria-selected", "true");
  const venueB = one.getByRole("article", { name: "DSPi" });
  await expect(venueB.getByRole("slider", { name: "Volume" })).toBeDisabled();
  await expect(venueB.locator(".read-only-reason")).toHaveText("You can view this DSPi but not change it");

  // Console two's session is its own: a write there does not show in console one.
  await two.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const twoSession = two.getByRole("article", { name: "DSPi" });
  const slider = twoSession.getByRole("slider", { name: "Volume" });
  // -- WP WB6a -- position 162 of 200 is -6 dB
  await slider.fill("162");
  await slider.dispatchEvent("change");
  await expect(twoSession.locator(".volume-value")).toHaveText("-6.0 dB");
  await one.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab").first().click();
  await expect(one.getByRole("article", { name: "DSPi" }).locator(".volume-value")).toHaveText("0.0 dB");

  // Our styles are scoped: a button of the host page keeps the host's style.
  const hostButtonColour = await page.evaluate(() => {
    const b = document.createElement("button");
    document.body.appendChild(b);
    const c = getComputedStyle(b).color;
    b.remove();
    return c;
  });
  expect(hostButtonColour).toBe("rgb(255, 0, 0)");
  expect(errors).toEqual([]);
});

test("core: a write reads back what the device kept; a refused one says why and changes nothing", async ({ page }) => {
  const errors = await noPageErrors(page);
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  await expect(one.getByRole("article", { name: "DSPi" }).locator(".volume-value")).toHaveText("0.0 dB");
  expect(await core(page, 0, "write", { path: "vol.user", value: { float: -12.3 } })).toEqual({ float: -12.3 });
  expect(await core(page, 0, "write", { path: "out.gain", indices: [1], value: { float: -4 } })).toEqual({ float: -4 });
  expect(await coreRefusal(page, 0, "write", { path: "vol.user", value: { float: 6 } })).toEqual({
    code: "out_of_range",
    message: "Volume must be between -60 and 0 dB",
  });
  const snap = (await core(page, 0, "state")) as { state: { userVolumeDb: number; outputGainDb: number[] } };
  expect(snap.state.userVolumeDb).toBe(-12.3);
  expect(snap.state.outputGainDb[1]).toBe(-4);
  expect(errors).toEqual([]);
});

test("core: a change made elsewhere arrives as a notification and the page shows it", async ({ page }) => {
  const errors = await noPageErrors(page);
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  await expect(one.getByRole("article", { name: "DSPi" })).toBeVisible();
  // The core's demo: another host renames the device. The Worker's poll drains
  // the notification and pushes the new snapshot.
  await core(page, 0, "demo", { action: "renameElsewhere", name: "Stage left" });
  await expect(one.getByRole("article", { name: "Stage left" }).locator(".device-name")).toHaveText("Stage left");
  expect(errors).toEqual([]);
});

test("core: the analyser delivers RTA frames on the Worker's timers", async ({ page }) => {
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  await expect(one.getByRole("article", { name: "DSPi" })).toBeVisible();
  await core(page, 0, "rtaStart", { tap: 1, mask: 3 });
  type View = { caps: { supported: boolean }; snapshot: { count: number; frames: { nBands: number; avgDb: number[] }[] } };
  await expect
    .poll(async () => {
      const v = (await core(page, 0, "rta")) as View;
      return v.caps.supported && v.snapshot.count >= 1 && v.snapshot.frames[0]!.nBands > 0;
    }, { timeout: 10_000 })
    .toBe(true);
  await core(page, 0, "rtaStop");
});

test("narrow width: no sideways scrolling", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  await expect(page.getByRole("article", { name: "DSPi" })).toBeVisible();
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
});
