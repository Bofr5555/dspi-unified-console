// WP ROLEW e2e: the speaker layout on the core's demo device, in every project (Chromium,
// Firefox, Chromium at 375 px without sideways scroll, light and dark). Settings › Speakers shows
// the suggestion with its banner, Confirm keeps it; the Measure setup is pre-filled from it and an
// edit there is written back; a reload sets the page's copy again; an imported layout shows the
// "imported" banner until it is confirmed (review RA F1).

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage, openSetting } from "../nav";

type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };

async function call<T>(page: Page, method: string, params: unknown = null): Promise<T> {
  return (await page.evaluate(([m, p]) => (globalThis as unknown as Clients).dspiClients[0]!.call(m as string, p), [method, params] as const)) as T;
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(session.getByRole("button", { name: /^Presets: / })).toBeVisible(); // -- WP INTD -- the session bar's presets (DL5)
  return session;
}

async function speakersPage(session: Locator): Promise<Locator> {
  await openPage(session, "Settings");
  await openSetting(session, "Speakers"); // -- WP WEB7 -- its part, then its row
  const page = session.getByRole("region", { name: "Speakers" });
  await expect(page.getByRole("list", { name: "Speaker layout" })).toBeVisible();
  return page;
}

async function measureSetup(session: Locator): Promise<Locator> {
  await openPage(session, "Measure");
  const setup = session.getByRole("region", { name: "Measurement setup" });
  await expect(setup).toBeVisible();
  return setup;
}

async function noSidewaysScroll(page: Page) {
  if (test.info().project.name !== "narrow") return;
  const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(over).toBeLessThanOrEqual(0);
}

test("ROLEW: the suggestion confirmed in Settings › Speakers; Measure pre-filled, an edit written back; kept across a reload", async ({ page }) => {
  const errors = errorsOf(page);
  let session = await openDemo(page);
  let sp = await speakersPage(session);
  await expect(sp.getByRole("status", { name: "Speaker layout", exact: true })).toContainText("Suggested from the filters");
  // The demo's outputs that are off are left out until asked for.
  const rows = sp.getByRole("list", { name: "Speaker layout" }).getByRole("listitem");
  const shown = await rows.count();
  const more = sp.getByRole("button", { name: /^Show the outputs that are off \(\d+\)$/ });
  await expect(more).toBeVisible();
  await more.click();
  await expect(more).toHaveCount(0);
  expect(await rows.count()).toBeGreaterThan(shown);
  const firstRole = sp.getByRole("combobox", { name: /^Role of OUT1 / });
  await firstRole.selectOption("low");
  await expect(sp.getByRole("status", { name: "Speaker layout state" })).toHaveText("");
  await sp.getByRole("button", { name: "Confirm" }).click();
  await expect(sp.getByRole("status", { name: "Speaker layout saved" })).toContainText("Confirmed");
  await expect(sp.getByRole("status", { name: "Speaker layout", exact: true })).toHaveCount(0);
  await expect(sp.getByRole("button", { name: "Confirm" })).toBeDisabled();
  await expect(sp.getByRole("status", { name: "Speaker layout state" })).toContainText(/^Confirmed today at /);
  await noSidewaysScroll(page);
  const stored = await call<{ confirmedByUser: boolean; outputs: { output: number; role: string }[] }>(page, "speakerLayout");
  expect(stored.confirmedByUser).toBe(true);
  expect(stored.outputs.find((o) => o.output === 0)?.role).toBe("low");

  // Measure: the rows come from the layout, with no banner; an edit is kept in the layout.
  let setup = await measureSetup(session);
  await expect(setup.getByRole("status", { name: "Speaker layout", exact: true })).toHaveCount(0);
  const way = setup.getByRole("combobox", { name: /^Way of Out 1 / });
  await expect(way).toHaveValue("low");
  await setup.getByRole("combobox", { name: /^Way of Out 2 / }).selectOption("high");
  await expect(setup.getByRole("status", { name: "Speaker layout saved" })).toHaveText("Saved to your speaker layout.");
  await noSidewaysScroll(page);
  const after = await call<{ confirmedByUser: boolean; outputs: { output: number; role: string }[] }>(page, "speakerLayout");
  expect(after.confirmedByUser, "the person's own edit keeps it confirmed").toBe(true);
  expect(after.outputs.find((o) => o.output === 1)?.role).toBe("high");

  // A reload: a fresh core, the page's copy set again when the device opens.
  await page.reload();
  session = await openDemo(page);
  await expect.poll(async () => (await call<{ confirmedByUser: boolean } | null>(page, "speakerLayout"))?.confirmedByUser).toBe(true);
  setup = await measureSetup(session);
  await expect(setup.getByRole("combobox", { name: /^Way of Out 2 / })).toHaveValue("high");
  await expect(setup.getByRole("status", { name: "Speaker layout", exact: true })).toHaveCount(0);
  sp = await speakersPage(session);
  await expect(sp.getByRole("combobox", { name: /^Role of OUT1 / })).toHaveValue("low");
  expect(errors).toEqual([]);
});

test("ROLEW: an imported layout is not confirmed: its banner on Measure and Settings › Speakers until Confirm", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  // A configuration whose layout says confirmed (another DSPi's, a friend's file).
  const proposal = await call<{ outputs: unknown[] }>(page, "speakerLayoutPropose");
  await call(page, "speakerLayoutSet", { layout: { ...proposal, confirmedByUser: true } });
  const { text } = await call<{ text: string }>(page, "configExport", { name: "Friend", appVersion: "e2e" });
  const doc = JSON.parse(text) as { meta: { speakerLayout: { outputs: { output: number; role: string }[] } } };
  doc.meta.speakerLayout.outputs[0]!.role = "sub";
  const token = await call<string>(page, "confirmToken", { op: "config_import" });
  const r = await call<{ speakerLayout: boolean }>(page, "configImport", { text: JSON.stringify(doc), options: (1 << 16) | (1 << 3), token });
  expect(r.speakerLayout).toBe(true);

  const setup = await measureSetup(session);
  const banner = setup.getByRole("status", { name: "Speaker layout", exact: true });
  await expect(banner).toContainText("Imported speaker layout");
  await expect(banner).toContainText("Settings › Speakers");
  await expect(setup.getByRole("combobox", { name: /^Way of Out 1 / })).toHaveValue("sub");
  await noSidewaysScroll(page);
  if (test.info().project.name === "narrow") {
    await setup.screenshot({ path: test.info().outputPath("measure-imported-light.png") });
    await page.emulateMedia({ colorScheme: "dark" });
    await setup.screenshot({ path: test.info().outputPath("measure-imported-dark.png") });
    await page.emulateMedia({ colorScheme: "light" });
  }

  const sp = await speakersPage(session);
  await expect(sp.getByRole("status", { name: "Speaker layout", exact: true })).toContainText("set outside this app");
  await noSidewaysScroll(page);
  if (test.info().project.name === "narrow") {
    await sp.screenshot({ path: test.info().outputPath("speakers-imported-light.png") });
    await page.emulateMedia({ colorScheme: "dark" });
    await noSidewaysScroll(page);
    await sp.screenshot({ path: test.info().outputPath("speakers-imported-dark.png") });
  }
  await sp.getByRole("button", { name: "Confirm" }).click();
  await expect(sp.getByRole("status", { name: "Speaker layout", exact: true })).toHaveCount(0);
  const setup2 = await measureSetup(session);
  await expect(setup2.getByRole("status", { name: "Speaker layout", exact: true })).toHaveCount(0);
  expect((await call<{ confirmedByUser: boolean }>(page, "speakerLayout")).confirmedByUser).toBe(true);
  expect(errors).toEqual([]);
});

test("ROLEW: a view-only console shows the layout, not changes it", async ({ page }) => {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  // Venue B is this console's view-only installation.
  await one.getByRole("region", { name: "Venue B" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(session.getByRole("button", { name: /^Presets: / })).toBeVisible(); // -- WP INTD -- the session bar's presets (DL5)
  const sp = await speakersPage(session);
  await expect(sp.getByRole("combobox", { name: /^Role of OUT1 / })).toBeDisabled();
  await expect(sp.getByRole("button", { name: "Confirm" })).toBeDisabled();
  await noSidewaysScroll(page);
});
