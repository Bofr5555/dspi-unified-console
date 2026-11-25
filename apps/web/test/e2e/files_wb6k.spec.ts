// WP WB6K e2e: the Files screen's filter files, saves to the device and AutoEQ database, and the
// AutoEQ panel's Browse and Favorite Profiles, on the core's demo device (crates/dspi-wasm), in
// Chromium, Firefox and Chromium at 375 px. Every action is the page's; its effect is read back
// from the core. The Download fetches a local fixture (test/pages/autoeq-fixture.json): no
// internet. And the `.dspipreset`'s input pair-link mask both ways.

import { readFileSync } from "node:fs";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

type Band = { filterType: number; freq: number; q: number; gain: number };
type State = { filters: Band[][] };
type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };

async function call<T>(page: Page, method: string, params: unknown = null): Promise<T> {
  return (await page.evaluate(([m, p]) => (globalThis as unknown as Clients).dspiClients[0]!.call(m as string, p), [method, params] as const)) as T;
}
const coreState = async (page: Page) => (await call<{ state: State }>(page, "state")).state;

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    // A Download of a missing address logs the browser's own 404 line.
    if (m.type() === "error" && !/WebSocket|404|Failed to load resource/i.test(m.text())) errors.push(m.text());
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


async function noSidewaysScroll(page: Page) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(0);
}

const setBand = (page: Page, channel: number, gain: number) => call(page, "filter", { channel, band: 0, filterType: 1, freq: 1000, q: 1, gain });

test("WB6K M28: Export Filters downloads the bands; Import Filters… on the chosen channels puts them back, one undo step", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await setBand(page, 0, 5);
  await setBand(page, 1, 3);
  await openPage(session, "Files");
  const files = session.getByRole("region", { name: "Files" });
  const card = files.getByRole("region", { name: "Filters" });
  const [dl] = await Promise.all([page.waitForEvent("download"), card.getByRole("button", { name: "Export Filters…" }).click() /* -- WP WEB7 -- actions.json's words */]);
  expect(dl.suggestedFilename()).toBe("DSPi Filters.txt");
  const text = readFileSync(await dl.path(), "utf8");
  expect(text.length).toBeGreaterThan(0);

  await setBand(page, 0, 0);
  await setBand(page, 1, -1);
  await card.getByLabel("Filter file to import").setInputFiles({ name: "DSPi Filters.txt", mimeType: "text/plain", buffer: Buffer.from(text) });
  const picker = files.getByRole("region", { name: "Import filters" });
  await expect(picker).toContainText("A DSPi filter file.");
  const boxes = picker.getByRole("checkbox");
  expect(await boxes.count()).toBeGreaterThan(2);
  // Only the first channel.
  for (let i = 1; i < (await boxes.count()); i++) await boxes.nth(i).uncheck();
  await expect(boxes.first()).toBeChecked();
  if (test.info().project.name === "narrow") await noSidewaysScroll(page);
  await picker.getByRole("button", { name: "Import", exact: true }).click();
  await expect(files.getByRole("status", { name: "Result" })).toContainText("Imported filters from “DSPi Filters.txt”");
  await expect(picker).toHaveCount(0);
  await expect.poll(async () => (await coreState(page)).filters[0]![0]!.gain).toBe(5);
  expect((await coreState(page)).filters[1]![0]!.gain).toBe(-1);
  // One step: one undo takes it back.
  await call(page, "undo");
  await expect.poll(async () => (await coreState(page)).filters[0]![0]!.gain).toBe(0);

  // A file with nothing this device can use: said before any picker.
  await card.getByLabel("Filter file to import").setInputFiles({ name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from("hello") });
  await expect(card.getByRole("alert")).toContainText("notes.txt: No filters this device can use were found in the file.");
  await expect(picker).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("WB6K M30: Save Master Volume and Save Output Configuration store the live values", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  // -- WP WEB7 -- Qt's Save ▾ rows (they were a card on Files); what they did in a notice
  for (const [row, said] of [
    ["Save Master Volume", /Master volume saved \(.+ dB\)\. It will be applied on next boot\./],
    ["Save Output Configuration", /Output configuration saved\. It will be applied on next boot\./],
  ] as const) {
    await session.getByRole("button", { name: "Save options" }).click();
    await session.getByRole("menuitem", { name: row }).click();
    const told = page.getByRole("dialog", { name: row });
    await expect(told).toContainText(said);
    await told.getByRole("button", { name: "OK" }).click();
    await expect(told).toHaveCount(0);
  }
  if (test.info().project.name === "narrow") await noSidewaysScroll(page);
  expect(errors).toEqual([]);
});

/** -- WP WEB7 -- Update Database… is Library › AutoEQ's ▾ (Qt's), a dialog with the database's panel. */
async function updateDatabase(session: Locator, page: Page): Promise<Locator> {
  await openPage(session, "AutoEQ");
  await session.getByRole("toolbar", { name: "AutoEQ tools" }).getByRole("button", { name: "More" }).click();
  await session.getByRole("menuitem", { name: "Update Database…" }).click();
  return page.getByRole("dialog", { name: "Update Database" }).getByRole("region", { name: "AutoEQ database" });
}

test("WB6K M32: Update Database: the built-in one, Download (the page's GET, a local fixture), Import File refused, Reset to Built-in", async ({ page, baseURL }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  const card = await updateDatabase(session, page);
  const current = card.getByRole("status", { name: "Current database" });
  await expect(current).toContainText(/Current database: .*, \d{3,} profiles \(the built-in one\)\./, { timeout: 15_000 });
  await expect(card.getByRole("button", { name: "Reset to Built-in" })).toHaveCount(0);

  // A missing address: the server's status, in the core's words; nothing changes.
  await card.getByLabel("Address of a database file").fill(`${baseURL}test/no-such-database.json`);
  await card.getByRole("button", { name: "Download", exact: true }).click();
  await expect(card.getByRole("alert", { name: "Database result" })).toContainText("Download: not done. The server answered with status 404");
  await expect(current).toContainText("(the built-in one)");

  await card.getByLabel("Address of a database file").fill(`${baseURL}test/autoeq-fixture.json`);
  await card.getByRole("button", { name: "Download", exact: true }).click();
  await expect(card.getByRole("status", { name: "Database result" })).toContainText("Download: 3 profiles.");
  await expect(current).toContainText("Current database: 2026-11-01, 3 profiles (your own).");

  // Import File: a file that is no database is refused and the one before stays.
  await card.getByLabel("AutoEQ database file").setInputFiles({ name: "empty.json", mimeType: "application/json", buffer: Buffer.from("{}") });
  await expect(card.getByRole("alert", { name: "Database result" })).toContainText("Import File: not done.");
  await expect(current).toContainText("3 profiles (your own)");

  // Kept for the next visit, in the page's storage.
  await page.reload();
  const again = await openDemo(page);
  const card2 = await updateDatabase(again, page);
  await expect(card2.getByRole("status", { name: "Current database" })).toContainText("3 profiles (your own)");
  await card2.getByRole("button", { name: "Reset to Built-in" }).click();
  await expect(card2.getByRole("status", { name: "Current database" })).toContainText("(the built-in one)", { timeout: 15_000 });
  await expect(card2.getByRole("button", { name: "Reset to Built-in" })).toHaveCount(0);
  if (test.info().project.name === "narrow") await noSidewaysScroll(page);
  expect(errors).toEqual([]);
});

test("WB6K M31: Browse Profiles and Favorite Profiles in the AutoEQ panel; a found profile previews; Clear Favorites", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await openPage(session, "EQ");
  await session.getByRole("button", { name: "AutoEQ…" }).click();
  const panel = session.getByRole("region", { name: "AutoEQ" });
  await panel.getByLabel("Search profiles").fill("sennheiser hd 600");
  const found = panel.getByRole("list", { name: "Profiles found" });
  await expect(found.getByRole("button", { name: "Sennheiser HD 600", exact: true })).toBeVisible({ timeout: 15_000 });
  await found.getByRole("button", { name: "Favorite Sennheiser HD 600", exact: true }).click();
  await expect(found.getByRole("button", { name: "Favorite Sennheiser HD 600", exact: true })).toHaveAttribute("aria-pressed", "true");
  await found.getByRole("button", { name: "Sennheiser HD 600", exact: true }).click();
  await expect(panel.locator(".autoeq-name")).toHaveText("Sennheiser HD 600");
  await panel.getByRole("button", { name: "Preview" }).click();
  await expect(panel.getByRole("status", { name: "AutoEQ preview" })).toContainText("Preview of Sennheiser HD 600");
  // With no search, the favourites.
  await panel.getByLabel("Search profiles").fill("");
  await expect(panel.getByRole("list", { name: "Favorite Profiles" })).toContainText("Sennheiser HD 600");
  await panel.getByLabel("Search profiles").fill("zzzz no such headphone");
  await expect(panel).toContainText("No profile matches “zzzz no such headphone”.");
  if (test.info().project.name === "narrow") await noSidewaysScroll(page);

  const card = await updateDatabase(session, page);
  await expect(card.getByRole("list", { name: "Favorite Profiles" })).toContainText("Sennheiser HD 600");
  await card.getByRole("button", { name: "Clear Favorites" }).click();
  await expect(card).toContainText("No favorites yet.");
  await expect(card.getByRole("button", { name: "Clear Favorites" })).toBeDisabled();
  expect(errors).toEqual([]);
});

test("WB6K: the .dspipreset carries the input pair links, and an import with routing restores them", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await openPage(session, "Inputs");
  const link = session.getByRole("region", { name: "Input trims" }).getByRole("button", { name: "Link 1/2" });
  await expect(link).toHaveAttribute("aria-pressed", "true");

  await openPage(session, "Files");
  const files = session.getByRole("region", { name: "Files" });
  // -- WP WEB7 -- Qt's Export Device Configuration…: the name asked in a dialog
  await files.getByRole("button", { name: "Export Device Configuration…" }).click();
  const [dl] = await Promise.all([page.waitForEvent("download"), page.getByRole("dialog", { name: "Export Device Configuration" }).getByRole("button", { name: "Export" }).click()]);
  const text = readFileSync(await dl.path(), "utf8");
  expect(JSON.parse(text).global.inputPairLinked[0]).toBe(true);

  await openPage(session, "Inputs");
  await link.click();
  await expect(link).toHaveAttribute("aria-pressed", "false");

  await openPage(session, "Files");
  await files.getByLabel("Configuration file to import").setInputFiles({ name: "links.dspipreset", mimeType: "application/json", buffer: Buffer.from(text) });
  const preview = files.getByRole("region", { name: "Import preview" });
  await preview.getByRole("checkbox", { name: /Routing and matrix/ }).check();
  await preview.getByRole("button", { name: "Import", exact: true }).click();
  await expect(files.getByRole("status", { name: "Result" })).toContainText("Imported “");
  await openPage(session, "Inputs");
  await expect(link).toHaveAttribute("aria-pressed", "true");
  expect(errors).toEqual([]);
});
