// WP WB6e e2e: the scene bar on the core's demo device, in Chromium, Firefox and Chromium at
// 375 px: the ten preset slots as numbered buttons, the active one and its unsaved state
// marked; Recall loads a slot (asking first when something is unsaved: the core's
// unsaved-changes prompt, or for the active slot a plain question); Store stores the live
// settings in the slot chosen next, asked first; each read back from the core's own state.
// Off for a viewer; not in Simple; no sideways scroll at 375 px.

import { expect, test, type Locator, type Page } from "@playwright/test";

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
  const bar = session.getByRole("group", { name: "Preset slots" });
  await expect(bar).toBeVisible();
  return { session, bar };
}

/** A slot's button, by its number, whichever verb is on. */
const slot = (bar: Locator, n: number) => bar.getByRole("button", { name: new RegExp(`^(Recall|Store in) ${n}: `) });

async function setGain(session: Locator, db: number) {
  const s = session.getByRole("region", { name: "Outputs" }).getByRole("slider").first();
  await s.fill(String(db));
  await s.dispatchEvent("change");
}

async function copySlot(page: Page, source: number, dest: number) {
  await page.evaluate(
    async ([source, dest]) => {
      const c = (globalThis as unknown as Clients).dspiClients[0]!;
      const token = await c.call("confirmToken", { op: "copy_preset" });
      return c.call("presetCopy", { source, dest, token });
    },
    [source, dest] as const,
  );
}

test("WB6e: ten numbered slots, the active one marked, and its unsaved state once something changes", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, bar } = await open(page);
  const st = await coreState(page);
  const a = st.activePresetSlot;
  await expect(bar.getByRole("button", { name: /^Recall \d+: / })).toHaveCount(10);
  for (let i = 0; i < 10; i++) await expect(slot(bar, i + 1)).toContainText(String(i + 1));
  await expect(bar.locator("[aria-current=true]")).toHaveCount(1);
  await expect(slot(bar, a + 1)).toHaveAttribute("aria-current", "true");
  await expect(slot(bar, a + 1)).toHaveAccessibleName(`Recall ${a + 1}: ${st.presetNames[a]} (active)`);
  const empty = [...Array(10).keys()].find((i) => (st.presetOccupied & (1 << i)) === 0)!;
  await expect(slot(bar, empty + 1)).toHaveAccessibleName(`Recall ${empty + 1}: Empty`);
  await expect(bar.getByLabel("Active preset")).toHaveText(`${a + 1}: ${st.presetNames[a]}`);
  // Recall is the verb on; Store is off.
  await expect(bar.getByRole("button", { name: "Recall", exact: true })).toHaveAttribute("aria-pressed", "true");
  await expect(bar.getByRole("button", { name: "Store", exact: true })).toHaveAttribute("aria-pressed", "false");

  await setGain(session, -6);
  await expect(slot(bar, a + 1)).toHaveAccessibleName(/\(active, unsaved\)$/);
  await expect(bar.getByLabel("Active preset")).toHaveText(`${a + 1}: ${st.presetNames[a]} · Unsaved`);
  expect(errors).toEqual([]);
});

test("WB6e: Recall loads a slot; with unsaved changes it asks first (Cancel, Discard, Save)", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, bar } = await open(page);
  const start = (await coreState(page)).activePresetSlot;
  const other = start === 1 ? 2 : 1;
  await copySlot(page, start, other);

  // Nothing unsaved: one press loads it, no question.
  await slot(bar, other + 1).click();
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(other);
  await expect(slot(bar, other + 1)).toHaveAttribute("aria-current", "true");
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // Unsaved: the core's prompt; Cancel and Escape keep the slot and the change.
  await setGain(session, -6);
  await expect(slot(bar, other + 1)).toHaveAccessibleName(/unsaved/);
  const prompt = page.getByRole("dialog", { name: "Unsaved Changes" });
  await slot(bar, start + 1).click();
  await expect(prompt).toBeVisible();
  await expect(prompt.getByRole("listitem").first()).toContainText("-6.0 dB");
  await prompt.getByRole("button", { name: "Cancel" }).click();
  await expect(prompt).toHaveCount(0);
  await slot(bar, start + 1).click();
  await expect(prompt).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(prompt).toHaveCount(0);
  let s = await coreState(page);
  expect([s.activePresetSlot, s.outputGainDb[0]]).toEqual([other, -6]);

  // Save: stored in the slot left, then the recall.
  await slot(bar, start + 1).click();
  await prompt.getByRole("button", { name: "Save" }).click();
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(start);
  await expect(slot(bar, start + 1)).toHaveAttribute("aria-current", "true");
  await expect(slot(bar, start + 1)).not.toHaveAccessibleName(/unsaved/);
  expect((await coreState(page)).outputGainDb[0]).toBe(0);

  // Discard: the change goes, the other slot (with what Save stored) loads.
  await setGain(session, -3);
  await expect(slot(bar, start + 1)).toHaveAccessibleName(/unsaved/);
  await slot(bar, other + 1).click();
  await prompt.getByRole("button", { name: "Discard" }).click();
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(other);
  s = await coreState(page);
  expect(s.outputGainDb[0]).toBe(-6);
  // The Main window's picker follows the same slot.
  await expect(session.getByRole("region", { name: "Presets" }).getByRole("combobox", { name: "Preset" })).toHaveValue(String(other));
  expect(errors).toEqual([]);
});

test("WB6e: recalling the active slot with unsaved changes asks, then reloads it", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, bar } = await open(page);
  const a = (await coreState(page)).activePresetSlot;
  // Clean: pressing the active slot does nothing and asks nothing.
  await slot(bar, a + 1).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  await setGain(session, -9);
  await expect(slot(bar, a + 1)).toHaveAccessibleName(/unsaved/);
  const q = page.getByRole("dialog", { name: "Recall Preset" });
  await slot(bar, a + 1).click();
  await expect(q).toContainText("The unsaved changes are lost.");
  await q.getByRole("button", { name: "Cancel" }).click();
  expect((await coreState(page)).outputGainDb[0]).toBe(-9);
  await slot(bar, a + 1).click();
  await q.getByRole("button", { name: "Recall" }).click();
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(0);
  await expect(slot(bar, a + 1)).not.toHaveAccessibleName(/unsaved/);
  expect(errors).toEqual([]);
});

test("WB6e: Store arms the bar; the next slot stores the live settings there, asked first", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, bar } = await open(page);
  const st = await coreState(page);
  const empty = [...Array(10).keys()].find((i) => (st.presetOccupied & (1 << i)) === 0)!;
  await setGain(session, -4.5);
  await expect(slot(bar, st.activePresetSlot + 1)).toHaveAccessibleName(/unsaved/);

  const storeBtn = bar.getByRole("button", { name: "Store", exact: true });
  // Armed, then disarmed by Escape and by Recall: nothing stored.
  await storeBtn.click();
  await expect(storeBtn).toHaveAttribute("aria-pressed", "true");
  await expect(slot(bar, empty + 1)).toHaveAccessibleName(`Store in ${empty + 1}: Empty`);
  await expect(bar.getByRole("status")).toContainText("Choose the slot");
  await slot(bar, empty + 1).focus();
  await page.keyboard.press("Escape");
  await expect(storeBtn).toHaveAttribute("aria-pressed", "false");
  await storeBtn.click();
  await bar.getByRole("button", { name: "Recall", exact: true }).click();
  await expect(slot(bar, empty + 1)).toHaveAccessibleName(/^Recall /);

  // Store, Cancel: asked, nothing written, and the bar is back on Recall.
  const q = page.getByRole("dialog", { name: "Store Preset" });
  await storeBtn.click();
  await slot(bar, empty + 1).click();
  await expect(q).toContainText(`Store the current settings in slot ${empty + 1}?`);
  await q.getByRole("button", { name: "Cancel" }).click();
  await expect(storeBtn).toHaveAttribute("aria-pressed", "false");
  expect((await coreState(page)).presetOccupied & (1 << empty)).toBe(0);

  // Store: the slot holds the live settings, named, and is the active one, with nothing unsaved.
  await storeBtn.click();
  await slot(bar, empty + 1).click();
  await q.getByRole("button", { name: "Store" }).click();
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(empty);
  const after = await coreState(page);
  expect(after.presetOccupied & (1 << empty)).not.toBe(0);
  expect(after.presetNames[empty]).toBe(`Preset ${empty + 1}`);
  await expect(slot(bar, empty + 1)).toHaveAccessibleName(`Recall ${empty + 1}: Preset ${empty + 1} (active)`);

  // Into an occupied slot: the question says it is replaced.
  const occupied = st.activePresetSlot;
  await storeBtn.click();
  await slot(bar, occupied + 1).click();
  await expect(q).toContainText(`“${st.presetNames[occupied]}” is replaced`);
  await q.getByRole("button", { name: "Store" }).click();
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(occupied);
  expect((await coreState(page)).outputGainDb[0]).toBe(-4.5);
  expect(errors).toEqual([]);
});

test("WB6e: a viewer sees the slots but cannot recall or store", async ({ page }) => {
  const errors = errorsOf(page);
  const { bar } = await open(page, "Venue B");
  await expect(bar.getByRole("button")).toHaveCount(12);
  for (const b of await bar.getByRole("button").all()) await expect(b).toBeDisabled();
  expect(errors).toEqual([]);
});

test("WB6e: Simple has no scene bar; at 375 px the bar wraps without sideways scroll", async ({ page }) => {
  const errors = errorsOf(page);
  await page.setViewportSize({ width: 375, height: 800 });
  const { session, bar } = await open(page);
  await expect(slot(bar, 10)).toBeVisible();
  const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(over).toBeLessThanOrEqual(0);
  const box = await bar.boundingBox();
  expect(box!.x + box!.width).toBeLessThanOrEqual(375);

  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: "Simple" }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await expect(session.getByRole("button", { name: "Mode: Simple" })).toBeVisible();
  await expect(bar).toHaveCount(0);
  expect(errors).toEqual([]);
});
