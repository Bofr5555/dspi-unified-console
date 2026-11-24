// WP WB6e e2e: the device's preset slots on the core's demo device, in Chromium, Firefox and
// Chromium at 375 px. -- WP INTD -- They are the session bar's preset popover now (DL5, Qt's
// PresetPopover; the slot strip above Overview and Library is gone): the ten slots as rows, the
// active one and its unsaved state marked; Recall loads a slot (asking first when something is
// unsaved: the core's unsaved-changes prompt, or for the active slot a plain question); Save
// Current Settings Here stores the live settings in a slot, asked first; each read back from the
// core's own state. Off for a viewer; no sideways scroll at 375 px.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPresets, presetsButton, presetsReady, setOutputGain, slotMenu, slotRow } from "../presets";

type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };
type State = { activePresetSlot: number; presetOccupied: number; presetNames: string[]; outputGainDb: number[] };

async function coreState(page: Page, n = 0): Promise<State> {
  const snap = (await page.evaluate((n) => (globalThis as unknown as Clients).dspiClients[n]!.call("state", null), n)) as { state: State };
  return snap.state;
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

async function open(page: Page, venue = "Venue A") {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: venue }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await presetsReady(session);
  return session;
}

/** Slot `dest` a copy of the active one, through the page (an empty slot's Copy Here), so the page knows it is stored. */
async function copySlot(session: Locator, page: Page, dest: number) {
  await slotRow(await openPresets(session), dest + 1).getByRole("button", { name: "Copy Here" }).click();
  await expect.poll(async () => (await coreState(page)).presetOccupied & (1 << dest)).not.toBe(0);
}

test("WB6e: ten slots in the popover, the active one marked, and its unsaved state once something changes", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await open(page);
  const st = await coreState(page);
  const a = st.activePresetSlot;
  await expect(presetsButton(session)).toHaveAccessibleName(`Presets: ${a + 1}: ${st.presetNames[a]}`);
  let pop = await openPresets(session);
  await expect(pop.getByRole("listitem")).toHaveCount(10);
  await expect(pop.locator("[aria-current=true]")).toHaveCount(1);
  await expect(slotRow(pop, a + 1)).toHaveAttribute("aria-current", "true");
  await expect(slotRow(pop, a + 1)).toHaveAccessibleName(new RegExp(`^${a + 1}: ${st.presetNames[a]}, Active`));
  // The active slot's action is Save, a stored one's Recall, an empty one's Copy Here.
  await expect(slotRow(pop, a + 1).getByRole("button", { name: "Save", exact: true })).toBeVisible();
  const empty = [...Array(10).keys()].find((i) => (st.presetOccupied & (1 << i)) === 0)!;
  await expect(slotRow(pop, empty + 1)).toHaveAccessibleName(`${empty + 1}: Empty`);
  await expect(slotRow(pop, empty + 1).getByRole("button", { name: "Copy Here" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(pop).toHaveCount(0);
  await expect(presetsButton(session)).toBeFocused();

  await setOutputGain(session, -6);
  await expect(presetsButton(session)).toHaveAccessibleName(/, unsaved changes$/);
  pop = await openPresets(session);
  await expect(slotRow(pop, a + 1)).toHaveAccessibleName(/Active, with unsaved changes/);
  await expect(slotRow(pop, a + 1).getByRole("button", { name: "Save", exact: true })).toHaveClass(/primary/);
  expect(errors).toEqual([]);
});

test("WB6e: Recall loads a slot; with unsaved changes it asks first (Cancel, Discard, Save)", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await open(page);
  const start = (await coreState(page)).activePresetSlot;
  const other = start === 1 ? 2 : 1;
  await copySlot(session, page, other);
  const recallIn = async (n: number) => (await openPresets(session)).getByRole("button", { name: new RegExp(`^Recall ${n}: `) }).click();

  // Nothing unsaved: one press loads it, no question.
  await recallIn(other + 1);
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(other);
  await expect(slotRow(await openPresets(session), other + 1)).toHaveAttribute("aria-current", "true");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "Unsaved Changes" })).toHaveCount(0);

  // Unsaved: the core's prompt; Cancel and Escape keep the slot and the change.
  await setOutputGain(session, -6);
  await expect(presetsButton(session)).toHaveAccessibleName(/unsaved/);
  const prompt = page.getByRole("dialog", { name: "Unsaved Changes" });
  await recallIn(start + 1);
  await expect(prompt).toBeVisible();
  await expect(prompt.getByRole("listitem").first()).toContainText("-6.0 dB");
  await prompt.getByRole("button", { name: "Cancel" }).click();
  await expect(prompt).toHaveCount(0);
  await recallIn(start + 1);
  await expect(prompt).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(prompt).toHaveCount(0);
  let s = await coreState(page);
  expect([s.activePresetSlot, s.outputGainDb[0]]).toEqual([other, -6]);

  // Save: stored in the slot left, then the recall.
  await recallIn(start + 1);
  await prompt.getByRole("button", { name: "Save" }).click();
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(start);
  await expect(presetsButton(session)).not.toHaveAccessibleName(/unsaved/);
  expect((await coreState(page)).outputGainDb[0]).toBe(0);

  // Discard: the change goes, the other slot (with what Save stored) loads.
  await setOutputGain(session, -3);
  await expect(presetsButton(session)).toHaveAccessibleName(/unsaved/);
  await recallIn(other + 1);
  await prompt.getByRole("button", { name: "Discard" }).click();
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(other);
  s = await coreState(page);
  expect(s.outputGainDb[0]).toBe(-6);
  await expect(presetsButton(session)).toHaveAccessibleName(new RegExp(`^Presets: ${other + 1}: `));
  expect(errors).toEqual([]);
});

test("WB6e: recalling the active slot with unsaved changes asks, then reloads it", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await open(page);
  const a = (await coreState(page)).activePresetSlot;
  // Clean: the active slot's menu has Recall off (it is what runs).
  await slotRow(await openPresets(session), a + 1).getByRole("button", { name: "More" }).click();
  await expect(session.getByRole("menuitem", { name: "Recall", exact: true })).toBeDisabled();
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");

  await setOutputGain(session, -9);
  await expect(presetsButton(session)).toHaveAccessibleName(/unsaved/);
  const q = page.getByRole("dialog", { name: "Recall Preset" });
  await slotMenu(session, a + 1, "Recall");
  await expect(q).toContainText("The unsaved changes are lost.");
  await q.getByRole("button", { name: "Cancel" }).click();
  expect((await coreState(page)).outputGainDb[0]).toBe(-9);
  await slotMenu(session, a + 1, "Recall");
  await q.getByRole("button", { name: "Recall" }).click();
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(0);
  await expect(presetsButton(session)).not.toHaveAccessibleName(/unsaved/);
  expect(errors).toEqual([]);
});

test("WB6e: Save Current Settings Here stores the live settings in a slot, asked first", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await open(page);
  const st = await coreState(page);
  const empty = [...Array(10).keys()].find((i) => (st.presetOccupied & (1 << i)) === 0)!;
  await setOutputGain(session, -4.5);
  await expect(presetsButton(session)).toHaveAccessibleName(/unsaved/);

  // Cancel: asked, nothing written.
  const q = page.getByRole("dialog", { name: "Store Preset" });
  await slotMenu(session, empty + 1, "Save Current Settings Here");
  await expect(q).toContainText(`Store the current settings in slot ${empty + 1}?`);
  await q.getByRole("button", { name: "Cancel" }).click();
  expect((await coreState(page)).presetOccupied & (1 << empty)).toBe(0);

  // Store: the slot holds the live settings, named, and is the active one, with nothing unsaved.
  await slotMenu(session, empty + 1, "Save Current Settings Here");
  await q.getByRole("button", { name: "Store" }).click();
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(empty);
  const after = await coreState(page);
  expect(after.presetOccupied & (1 << empty)).not.toBe(0);
  expect(after.presetNames[empty]).toBe(`Preset ${empty + 1}`);
  await expect(presetsButton(session)).toHaveAccessibleName(`Presets: ${empty + 1}: Preset ${empty + 1}`);

  // Into an occupied slot: the question says it is replaced.
  const occupied = st.activePresetSlot;
  await slotMenu(session, occupied + 1, "Save Current Settings Here");
  await expect(q).toContainText(`“${st.presetNames[occupied]}” is replaced`);
  await q.getByRole("button", { name: "Store" }).click();
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(occupied);
  expect((await coreState(page)).outputGainDb[0]).toBe(-4.5);
  expect(errors).toEqual([]);
});

test("WB6e: a viewer sees the slots but cannot recall, save or copy", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await open(page, "Venue B");
  const pop = await openPresets(session);
  await expect(pop.getByRole("listitem")).toHaveCount(10);
  for (const name of [/^Recall /, "Save", "Copy Here"]) for (const b of await pop.getByRole("button", { name }).all()) await expect(b).toBeDisabled();
  expect(errors).toEqual([]);
});

test("WB6e: at 375 px the popover fits without sideways scroll, in every mode", async ({ page }) => {
  const errors = errorsOf(page);
  await page.setViewportSize({ width: 375, height: 800 });
  const session = await open(page);
  const pop = await openPresets(session);
  await expect(slotRow(pop, 10)).toBeAttached();
  const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(over).toBeLessThanOrEqual(0);
  const box = await pop.boundingBox();
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(375);
  await page.keyboard.press("Escape");

  // Simple keeps the presets (they were the Main window's picker there).
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: "Simple" }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await expect(session.getByRole("button", { name: "Mode: Simple" })).toBeVisible();
  await expect(presetsButton(session)).toBeVisible();
  expect(errors).toEqual([]);
});
