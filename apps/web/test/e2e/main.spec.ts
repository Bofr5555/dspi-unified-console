// WP WB6a e2e: the main window on the core's demo device (crates/dspi-wasm) -- since INTD the
// session bar's presets and the pages that hold the levels --,
// in Chromium, Firefox and Chromium at 375 px (the `narrow` project), plus a
// 375 px block that runs in every project (so Firefox too). Every control is
// used through the page and its effect read back twice: the page shows the
// device's value, and the core's own state (a direct call) agrees.

import { expect, test, type Locator, type Page } from "@playwright/test";
// -- WP INTD -- the presets are the session bar's popover (DL5), the levels Equaliser › Inputs' and
// Delay & gain's: the Overview's preset card and level cards are gone
import { openPage } from "../nav";
import { openPresets, presetsButton, presetsReady, setOutputGain, slotMenu, slotRow } from "../presets";

type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };
type State = {
  userVolumeDb: number;
  masterVolumeDb: number;
  userMute: boolean;
  bypass: boolean;
  activePresetSlot: number;
  presetOccupied: number;
  presetNames: string[];
  presetStartupMode: number;
  presetDefaultSlot: number;
  channelNames: string[];
  numInputChannels: number;
  inputSource: number;
  inputPreampDb: number[];
  outputGainDb: number[];
  outputMuted: boolean[];
};

/**
 * The core's state of the n-th session the embedding page opened. -- WP WB6b --
 * Read by polling after a page action: a slider shows its position at once and
 * the device's value once its writes are read back, so a read made in between
 * may come before the last write (it shares the Worker's queue).
 */
// -- BUG1 -- An output's name on the page is its label ("Out 3 · R"), which follows the
// routing and the filters while the device keeps the slot's default name: matched by its number.
const OUT = (o: number) => `Out ${o + 1}(?: · [^·]+){0,2}`;
const re = (s: string) => new RegExp(`^${s}$`);

async function coreState(page: Page, n = 0): Promise<State> {
  const snap = (await page.evaluate(
    (n) => (globalThis as unknown as Clients).dspiClients[n]!.call("state", null),
    n,
  )) as { state: State };
  return snap.state;
}

async function unsavedLines(page: Page, n = 0): Promise<string[]> {
  const r = (await page.evaluate(
    (n) => (globalThis as unknown as Clients).dspiClients[n]!.call("presetChanges", null),
    n,
  )) as { changes: string[] };
  return r.changes;
}

async function setRange(slider: Locator, value: number) {
  await slider.fill(String(value));
  await slider.dispatchEvent("change");
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

/** Console one, Venue A (control): the demo, session 0. */
async function openControl(page: Page) {
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await presetsReady(session);
  return session;
}

/** A row of the session bar's Save ▾ menu (Revert to Saved…, Factory Reset…). */
async function saveMenu(session: Locator, item: string) {
  await session.getByRole("button", { name: "Save options" }).click();
  await session.getByRole("menuitem", { name: item }).click();
}

const unsavedShown = (session: Locator) => expect(presetsButton(session)).toHaveAccessibleName(/, unsaved changes$/);
const nothingUnsaved = (session: Locator) => expect(presetsButton(session)).not.toHaveAccessibleName(/unsaved/);

test("WB6a volume: user and master volume, reset, mute and bypass write and read back", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openControl(page);
  const vol = session.getByRole("region", { name: "Volume" });

  // The user taper: position 128 of 200 is -12 dB.
  await setRange(vol.getByRole("slider", { name: "Volume", exact: true }), 128);
  await expect(vol.locator(".volume-value")).toHaveText("-12.0 dB");
  await expect.poll(async () => (await coreState(page)).userVolumeDb).toBe(-12);

  // The master volume, on the firmware's taper: 100 steps of 0.1 dB below 0 is -10 dB.
  await vol.getByRole("combobox", { name: "Volume shown" }).selectOption("master");
  const master = vol.getByRole("slider", { name: "Master volume", exact: true });
  await setRange(master, 148);
  await expect(vol.locator(".volume-value")).toHaveText("-10.0 dB");
  await expect.poll(async () => (await coreState(page)).masterVolumeDb).toBe(-10);
  await expect(master).toHaveAttribute("aria-valuetext", "-10.0 dB");
  // A double click puts it back at 0 dB.
  await master.dblclick();
  await expect(vol.locator(".volume-value")).toHaveText("0.0 dB");
  await expect.poll(async () => (await coreState(page)).masterVolumeDb).toBe(0);

  const mute = vol.getByRole("button", { name: "Mute" });
  await mute.click();
  await expect(mute).toHaveAttribute("aria-pressed", "true");
  await expect.poll(async () => (await coreState(page)).userMute).toBe(true);
  await mute.click();
  await expect(mute).toHaveAttribute("aria-pressed", "false");
  await expect.poll(async () => (await coreState(page)).userMute).toBe(false);

  const bypass = vol.getByRole("button", { name: "Bypass" });
  await bypass.click();
  await expect(bypass).toHaveAttribute("aria-pressed", "true");
  await expect.poll(async () => (await coreState(page)).bypass).toBe(true);
  await bypass.click();
  await expect(bypass).toHaveAttribute("aria-pressed", "false");
  await expect.poll(async () => (await coreState(page)).bypass).toBe(false);

  // The volume shown is this viewer's choice and kept (the Console's sidebar mode).
  await page.reload();
  await page.locator("#one").getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  await expect(page.locator("#one").getByRole("combobox", { name: "Volume shown" })).toHaveValue("master");
  expect(errors).toEqual([]);
});

test("WB6a volume: the keyboard moves the slider and the device follows", async ({ page }) => {
  const session = await openControl(page);
  const slider = session.getByRole("slider", { name: "Volume", exact: true });
  await setRange(slider, 128);
  await expect(session.locator(".volume-value")).toHaveText("-12.0 dB");
  await slider.focus();
  await page.keyboard.press("ArrowLeft");
  // Position 127: -60 + 60 * sqrt(127/200), read back at the core's 0.1 dB.
  await expect(session.locator(".volume-value")).toHaveText("-12.2 dB");
  await expect.poll(async () => (await coreState(page)).userVolumeDb).toBe(-12.2);
});

test("WB6a presets: switching asks about unsaved changes (Cancel, Discard, Save), in the core's words", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openControl(page);
  const start = (await coreState(page)).activePresetSlot;
  const other = start === 1 ? 2 : 1;
  // A second stored slot to switch to (the demo stores only the one it runs).
  // -- WP INTD -- through the page (an empty slot's Copy Here), so the page knows the slot is stored
  await slotRow(await openPresets(session), other + 1).getByRole("button", { name: "Copy Here" }).click();
  await expect.poll(async () => (await coreState(page)).presetOccupied & (1 << other)).not.toBe(0);
  await expect(presetsButton(session)).toHaveAccessibleName(new RegExp(`^Presets: ${start + 1}: `));
  await nothingUnsaved(session);
  const recallIn = async (n: number) => (await openPresets(session)).getByRole("button", { name: new RegExp(`^Recall ${n}: `) }).click();

  // An output trim is a preset change.
  await setOutputGain(session, -6);
  await unsavedShown(session);
  const lines = await unsavedLines(page);
  expect(lines.some((l) => l.includes("-6.0 dB"))).toBe(true);

  // Cancel: nothing happens.
  await recallIn(other + 1);
  const prompt = page.getByRole("dialog", { name: "Unsaved Changes" });
  await expect(prompt).toBeVisible();
  await expect(prompt.getByRole("listitem").first()).toHaveText(lines[0]!);
  await prompt.getByRole("button", { name: "Cancel" }).click();
  await expect(prompt).toHaveCount(0);
  await expect(presetsButton(session)).toHaveAccessibleName(new RegExp(`^Presets: ${start + 1}: `));
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(start);

  // Escape is Cancel too.
  await recallIn(other + 1);
  await expect(prompt).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(prompt).toHaveCount(0);
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(start);

  // Discard: the other slot loads; the change is gone with it.
  await recallIn(other + 1);
  await prompt.getByRole("button", { name: "Discard" }).click();
  await expect(presetsButton(session)).toHaveAccessibleName(new RegExp(`^Presets: ${other + 1}: `));
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(other);
  await nothingUnsaved(session);

  // Save: the change is stored in the slot left, then the switch. An edit made
  // right after the load counts as unsaved (core fix CG33), so no wait is needed.
  await setOutputGain(session, -7.5);
  await unsavedShown(session);
  await recallIn(start + 1);
  await prompt.getByRole("button", { name: "Save" }).click();
  await expect(presetsButton(session)).toHaveAccessibleName(new RegExp(`^Presets: ${start + 1}: `));
  const st = await coreState(page);
  expect(st.activePresetSlot).toBe(start);
  expect(st.presetOccupied & (1 << other)).not.toBe(0);
  // And it is there when that slot is loaded again (no prompt: nothing unsaved).
  await recallIn(other + 1);
  await expect(presetsButton(session)).toHaveAccessibleName(new RegExp(`^Presets: ${other + 1}: `));
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(-7.5);
  expect(errors).toEqual([]);
});

test("WB6a presets: the slot menu renames, sets the startup slot, copies and clears", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openControl(page);
  // Work on slot 3, a copy of what runs, so Set as Startup Preset has something to do
  // (the demo starts on its default slot, 1).
  const start = (await coreState(page)).activePresetSlot;
  const active = start === 2 ? 4 : 2;
  const n = active + 1;
  // -- WP INTD -- through the page (an empty slot's Copy Here), so the page knows the slot is stored
  await slotRow(await openPresets(session), n).getByRole("button", { name: "Copy Here" }).click();
  await expect.poll(async () => (await coreState(page)).presetOccupied & (1 << active)).not.toBe(0);
  await (await openPresets(session)).getByRole("button", { name: new RegExp(`^Recall ${n}: `) }).click();
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(active);

  // Rename…: the device's name, read back into the popover and its button.
  await slotMenu(session, n, "Rename…");
  const rename = page.getByRole("dialog", { name: "Rename Preset" });
  await rename.getByRole("textbox", { name: "Name" }).fill("Stage");
  await rename.getByRole("button", { name: "Rename" }).click();
  await expect(presetsButton(session)).toHaveAccessibleName(`Presets: ${n}: Stage`);
  await expect.poll(async () => (await coreState(page)).presetNames[active]).toBe("Stage");

  // Set as Startup Preset: the startup slot, said on its row and under the list; the row is then off.
  await slotMenu(session, n, "Set as Startup Preset");
  await expect.poll(async () => {
    const s = await coreState(page);
    return [s.presetStartupMode, s.presetDefaultSlot];
  }).toEqual([0, active]);
  let pop = await openPresets(session);
  await expect(slotRow(pop, n)).toHaveAccessibleName(`${n}: Stage, Active · Startup`);
  await expect(pop).toContainText(`At power-on the device loads preset ${n}.`);
  await slotRow(pop, n).getByRole("button", { name: "More" }).click();
  await expect(slotRow(pop, n).getByRole("menuitem", { name: "Set as Startup Preset" })).toBeDisabled();
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");

  // Copy to…: slot n into slot 4, which takes a default name; the active slot stays.
  const dest = active === 3 ? 5 : 3;
  await slotMenu(session, n, "Copy to…");
  const copy = page.getByRole("dialog", { name: "Copy to…" });
  await expect(copy).toContainText("Copy the current settings to"); // -- WP REVEFIX --
  await copy.getByRole("combobox").selectOption(String(dest));
  await copy.getByRole("button", { name: "Copy" }).click();
  await expect(slotRow(await openPresets(session), dest + 1)).toHaveAccessibleName(`${dest + 1}: Preset ${dest + 1}`);
  let st = await coreState(page);
  expect(st.presetOccupied & (1 << dest)).not.toBe(0);
  expect(st.activePresetSlot).toBe(active);
  // -- WP REVEFIX -- (REVE B1) Copy to… is the active slot's alone, as Qt's: the device copies the live settings, so
  // from a stored slot that is not active it would write them over that slot. Its menu has no such row.
  pop = await openPresets(session);
  await slotRow(pop, dest + 1).getByRole("button", { name: "More" }).click();
  await expect(slotRow(pop, dest + 1).getByRole("menuitem", { name: "Recall", exact: true })).toBeVisible();
  await expect(slotRow(pop, dest + 1).getByRole("menuitem", { name: "Copy to…" })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  // An empty slot's Copy Here copies the active one into it.
  const here = [...Array(10).keys()].find((i) => (st.presetOccupied & (1 << i)) === 0)!;
  await slotRow(await openPresets(session), here + 1).getByRole("button", { name: "Copy Here" }).click();
  await expect.poll(async () => (await coreState(page)).presetOccupied & (1 << here)).not.toBe(0);

  // Clear…: asks, then empties the slot.
  await slotMenu(session, n, "Clear…");
  const clear = page.getByRole("dialog", { name: "Clear Preset?" });
  await expect(clear).toContainText(`Clear "${n}: Stage"`);
  await expect(clear).toContainText("This cannot be undone.");
  await clear.getByRole("button", { name: "Clear" }).click();
  await expect.poll(async () => (await coreState(page)).presetOccupied & (1 << active)).toBe(0);
  await expect(slotRow(await openPresets(session), n)).toHaveAccessibleName(new RegExp(`^${n}: Empty`));

  // Clear All Slots…: every slot.
  await slotMenu(session, dest + 1, "Clear All Slots…");
  await page.getByRole("dialog", { name: "Clear All Presets?" }).getByRole("button", { name: "Clear All" }).click();
  await expect.poll(async () => (await coreState(page)).presetOccupied).toBe(0);
  pop = await openPresets(session);
  await expect(slotRow(pop, dest + 1)).toHaveAccessibleName(`${dest + 1}: Empty`);
  // With nothing stored, the Clear rows are off.
  await slotRow(pop, dest + 1).getByRole("button", { name: "More" }).click();
  for (const item of ["Clear…", "Clear All Slots…"]) await expect(slotRow(pop, dest + 1).getByRole("menuitem", { name: item })).toBeDisabled();
  await page.keyboard.press("Escape");
  st = await coreState(page);
  expect(st.presetOccupied).toBe(0);
  expect(errors).toEqual([]);
});

test("WB6a tools: Save to device, Revert to Saved and Factory Reset ask first, then the core does them", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openControl(page);

  // Save to device: the live state into the active slot.
  await setOutputGain(session, -2);
  await unsavedShown(session);
  await session.getByRole("button", { name: /^Save to device/ }).click();
  const commit = page.getByRole("dialog", { name: "Save Preset" });
  await expect(commit).toContainText(`preset slot ${(await coreState(page)).activePresetSlot + 1}`);
  await commit.getByRole("button", { name: "Save" }).click();
  await nothingUnsaved(session);
  expect(await unsavedLines(page)).toEqual([]);

  // Revert: back to what was saved. Cancel first changes nothing.
  await setOutputGain(session, -9);
  await unsavedShown(session);
  await saveMenu(session, "Revert to Saved…");
  await page.getByRole("dialog", { name: "Revert to Saved" }).getByRole("button", { name: "Cancel" }).click();
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(-9);
  await saveMenu(session, "Revert to Saved…");
  await page.getByRole("dialog", { name: "Revert to Saved" }).getByRole("button", { name: "Revert" }).click();
  await nothingUnsaved(session);
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(-2);
  const gain = session.getByRole("region", { name: "Delay and gain of every output" }).getByRole("row", { name: /^OUT1 / }).getByRole("textbox", { name: /gain$/ });
  await expect(gain).toHaveValue("-2.0 dB");

  // Factory Reset: the defaults (bypass off, trims at 0). // -- WP WB6L -- exact: the quick strip has "Bypass Master EQ"
  await session.getByRole("button", { name: "Bypass", exact: true }).click();
  await expect.poll(async () => (await coreState(page)).bypass).toBe(true);
  await saveMenu(session, "Factory Reset…");
  await page.getByRole("dialog", { name: "Factory Reset" }).getByRole("button", { name: "Reset" }).click();
  await expect(session.getByRole("button", { name: "Bypass", exact: true })).toHaveAttribute("aria-pressed", "false");
  const st = await coreState(page);
  expect(st.bypass).toBe(false);
  expect(st.outputGainDb[0]).toBe(0);
  expect(errors).toEqual([]);
});

test("WB6a channels: an input renamed (EQ's Channel menu) and trimmed (Inputs), an output's gain and mute (Delay & gain), each read back", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openControl(page);
  const st0 = await coreState(page);
  expect(st0.inputSource).toBe(0);

  // Rename the first input from the EQ page's Channel menu (Enter in the field saves).
  await openPage(session, "EQ");
  const first = st0.channelNames[0]!;
  await session.getByRole("group", { name: "Channels" }).getByRole("button", { name: new RegExp(`^${first} IN1$`) }).click();
  await session.locator(".eq-tools").getByRole("button", { name: "Channel" }).click();
  await session.getByRole("menuitem", { name: "Rename", exact: true }).click();
  const dlg = page.getByRole("dialog", { name: `Rename ${first}` });
  await dlg.getByRole("textbox", { name: "Name" }).fill("Left in");
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await coreState(page)).channelNames[0]).toBe("Left in");

  // The input's trim (`pre`, the registry's "Input trim") on Equaliser › Inputs.
  await openPage(session, "Inputs");
  const trim = session.getByRole("region", { name: "Input trims" }).getByRole("slider", { name: "IN1 Left in Input trim" });
  await setRange(trim, -4);
  await expect.poll(async () => (await coreState(page)).inputPreampDb[0]).toBe(-4);

  // The first output: gain and mute on Delay & gain.
  await setOutputGain(session, -3.5);
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(-3.5);
  const mute = session.getByRole("region", { name: "Delay and gain of every output" }).getByRole("button", { name: re(`Mute OUT1 ${OUT(0)}`) });
  await mute.click();
  await expect(mute).toHaveAttribute("aria-pressed", "true");
  await expect.poll(async () => (await coreState(page)).outputMuted[0]).toBe(true);
  await mute.click();
  await expect(mute).toHaveAttribute("aria-pressed", "false");
  await expect.poll(async () => (await coreState(page)).outputMuted[0]).toBe(false);
  expect(errors).toEqual([]);
});

test("WB6a: Disconnect with unsaved changes asks first", async ({ page }) => {
  const session = await openControl(page);
  await setOutputGain(session, -5);
  await unsavedShown(session);
  await session.getByRole("button", { name: "Disconnect" }).click();
  const prompt = page.getByRole("dialog", { name: "Unsaved Changes" });
  await prompt.getByRole("button", { name: "Cancel" }).click();
  await expect(session).toBeVisible();
  await session.getByRole("button", { name: "Disconnect" }).click();
  await prompt.getByRole("button", { name: "Discard" }).click();
  await expect(page.locator("#one").getByText("Choose a DSPi to connect to.")).toBeVisible();
});

test("WB6a read only: a view-only session shows the Overview with every control off", async ({ page }) => {
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue B" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await presetsReady(session);
  await expect(session.locator(".read-only-reason")).toHaveText("You can view this DSPi but not change it");
  for (const slider of await session.getByRole("slider").all()) await expect(slider).toBeDisabled();
  // The presets can be looked at, not used.
  const pop = await openPresets(session);
  for (const b of await pop.getByRole("button", { name: /^(Recall .*|Save|Copy Here)$/ }).all()) await expect(b).toBeDisabled();
  await page.keyboard.press("Escape");
  for (const name of ["Mute", "Bypass"]) await expect(session.getByRole("button", { name, exact: true })).toBeDisabled();
  // Which volume is shown is the viewer's own choice, not a write.
  await expect(session.getByRole("combobox", { name: "Volume shown" })).toBeEnabled();
});

test.describe("375 px, in every browser", () => {
  test.use({ viewport: { width: 375, height: 800 } });

  test("WB6a narrow: one column, no sideways scroll, and nothing grows as states change", async ({ page }) => {
    // The standalone page (the embedding test page puts two consoles side by side).
    await page.goto("/");
    await page.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
    const session = page.getByRole("article", { name: "DSPi" });
    await presetsReady(session);
    const screen = session.locator(".main-screen");
    await expect(screen).toBeVisible();
    // The elements past the right edge, named, so a failure says which.
    const overflow = () =>
      page.evaluate(() => {
        const w = document.documentElement.clientWidth;
        const wide = Array.from(document.querySelectorAll<HTMLElement>(".dspi-web *"))
          .filter((e) => e.getBoundingClientRect().right > w + 0.5)
          .map((e) => `${e.tagName.toLowerCase()}.${e.className} ${Math.round(e.getBoundingClientRect().right)}`);
        return document.documentElement.scrollWidth > w ? wide.slice(0, 8) : [];
      });
    expect(await overflow()).toEqual([]);

    // Unsaved, muted, a preset renamed long: still no sideways scroll, and the bar keeps its height.
    const bar = session.getByRole("group", { name: "Device actions" });
    const barHeight = (await bar.boundingBox())!.height;
    await setOutputGain(session, -6);
    await unsavedShown(session);
    await session.getByRole("button", { name: "Mute", exact: true }).click();
    // The standalone page has no test hook: the active slot from the button ("Presets: 1: Demo")
    const n = Number(/^Presets: (\d+):/.exec((await presetsButton(session).getAttribute("aria-label")) ?? "")![1]);
    await slotMenu(session, n, "Rename…");
    await page.getByRole("dialog", { name: "Rename Preset" }).getByRole("textbox").fill("Long preset name, cut short");
    await page.keyboard.press("Enter");
    await expect(presetsButton(session)).toHaveAccessibleName(/Long preset name, cut short/);
    expect(Math.abs((await bar.boundingBox())!.height - barHeight)).toBeLessThan(0.5);
    expect(await overflow()).toEqual([]);

    // The popover fits the window; keyboard: it opens on its first control, Escape gives the focus back.
    const pop = await openPresets(session);
    const box = (await pop.boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(375);
    expect(await overflow()).toEqual([]);
    await expect(pop.getByRole("button").first()).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(presetsButton(session)).toBeFocused();
  });
});
