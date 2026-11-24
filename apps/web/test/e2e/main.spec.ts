// WP WB6a e2e: the main window on the core's demo device (crates/dspi-wasm),
// in Chromium, Firefox and Chromium at 375 px (the `narrow` project), plus a
// 375 px block that runs in every project (so Firefox too). Every control is
// used through the page and its effect read back twice: the page shows the
// device's value, and the core's own state (a direct call) agrees.

import { expect, test, type Locator, type Page } from "@playwright/test";

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
  await expect(session.getByRole("region", { name: "Presets" })).toBeVisible();
  return session;
}

async function menu(session: Locator, item: string) {
  await session.getByRole("button", { name: "Preset actions" }).click();
  await session.getByRole("menuitem", { name: item }).click();
}

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
  const presets = session.getByRole("region", { name: "Presets" });
  const picker = presets.getByRole("combobox", { name: "Preset", exact: true });
  const start = (await coreState(page)).activePresetSlot;
  const other = start === 1 ? 2 : 1;
  // A second stored slot to switch to (the demo stores only the one it runs).
  await page.evaluate(
    // -- WP P1WB2 -- tokens are required on the web: the copy carries one, as the page's Copy to does
    async ([source, dest]) => {
      const c = (globalThis as unknown as Clients).dspiClients[0]!;
      const token = await c.call("confirmToken", { op: "copy_preset" });
      return c.call("presetCopy", { source, dest, token });
    },
    [start, other] as const,
  );
  await expect(picker).toHaveValue(String(start));
  await expect(presets.getByText("Unsaved changes")).toHaveCount(0);

  // An output trim is a preset change.
  const outputs = session.getByRole("region", { name: "Outputs" });
  await setRange(outputs.getByRole("slider").first(), -6);
  await expect(presets.getByText("Unsaved changes")).toBeVisible();
  const lines = await unsavedLines(page);
  expect(lines.some((l) => l.includes("-6.0 dB"))).toBe(true);

  // Cancel: nothing happens.
  await picker.selectOption(String(other));
  const prompt = page.getByRole("dialog", { name: "Unsaved Changes" });
  await expect(prompt).toBeVisible();
  await expect(prompt.getByRole("listitem").first()).toHaveText(lines[0]!);
  await prompt.getByRole("button", { name: "Cancel" }).click();
  await expect(prompt).toHaveCount(0);
  await expect(picker).toHaveValue(String(start));
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(start);

  // Escape is Cancel too.
  await picker.selectOption(String(other));
  await expect(prompt).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(prompt).toHaveCount(0);
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(start);

  // Discard: the other slot loads; the change is gone with it.
  await picker.selectOption(String(other));
  await prompt.getByRole("button", { name: "Discard" }).click();
  await expect(picker).toHaveValue(String(other));
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(other);
  await expect(presets.getByText("Unsaved changes")).toHaveCount(0);

  // Save: the change is stored in the slot left, then the switch. An edit made
  // right after the load counts as unsaved (core fix CG33), so no wait is needed.
  await setRange(outputs.getByRole("slider").first(), -7.5);
  await expect(presets.getByText("Unsaved changes")).toBeVisible();
  await picker.selectOption(String(start));
  await prompt.getByRole("button", { name: "Save" }).click();
  await expect(picker).toHaveValue(String(start));
  const st = await coreState(page);
  expect(st.activePresetSlot).toBe(start);
  expect(st.presetOccupied & (1 << other)).not.toBe(0);
  // And it is there when that slot is loaded again (no prompt: nothing unsaved).
  await picker.selectOption(String(other));
  await expect(picker).toHaveValue(String(other));
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(-7.5);
  expect(errors).toEqual([]);
});

test("WB6a presets: the slot menu renames, sets the default, copies and clears", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openControl(page);
  const presets = session.getByRole("region", { name: "Presets" });
  const picker = presets.getByRole("combobox", { name: "Preset", exact: true });
  // Work on slot 3, a copy of what runs, so Set as Default has something to do
  // (the demo starts on its default slot, 1).
  const start = (await coreState(page)).activePresetSlot;
  const active = start === 2 ? 4 : 2;
  const n = active + 1;
  await page.evaluate(
    // -- WP P1WB2 -- tokens are required on the web: the copy carries one, as the page's Copy to does
    async ([source, dest]) => {
      const c = (globalThis as unknown as Clients).dspiClients[0]!;
      const token = await c.call("confirmToken", { op: "copy_preset" });
      return c.call("presetCopy", { source, dest, token });
    },
    [start, active] as const,
  );
  await picker.selectOption(String(active));
  await expect(picker).toHaveValue(String(active));
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(active);

  // Rename…: the device's name, read back into the picker.
  await menu(session, "Rename…");
  const rename = page.getByRole("dialog", { name: "Rename Preset" });
  await rename.getByRole("textbox", { name: "Name" }).fill("Stage");
  await rename.getByRole("button", { name: "Rename" }).click();
  await expect(picker.locator(`option[value="${active}"]`)).toHaveText(`${n}: Stage`);
  await expect.poll(async () => (await coreState(page)).presetNames[active]).toBe("Stage");

  // Set as Default: the startup slot; the entry is then off.
  await menu(session, "Set as Default");
  await expect.poll(async () => {
    const s = await coreState(page);
    return [s.presetStartupMode, s.presetDefaultSlot];
  }).toEqual([0, active]);
  await session.getByRole("button", { name: "Preset actions" }).click();
  await expect(session.getByRole("menuitem", { name: "Set as Default" })).toBeDisabled();
  await page.keyboard.press("Escape");

  // Copy to…: the live state into slot 4, which takes a default name; the active slot stays.
  const dest = active === 3 ? 5 : 3;
  await menu(session, "Copy to…");
  const copy = page.getByRole("dialog", { name: "Copy to…" });
  await copy.getByRole("combobox").selectOption(String(dest));
  await copy.getByRole("button", { name: "Copy" }).click();
  await expect(picker.locator(`option[value="${dest}"]`)).toHaveText(`${dest + 1}: Preset ${dest + 1}`);
  let st = await coreState(page);
  expect(st.presetOccupied & (1 << dest)).not.toBe(0);
  expect(st.activePresetSlot).toBe(active);

  // Clear "…"…: asks, then empties the active slot.
  await menu(session, `Clear "${n}: Stage"…`);
  const clear = page.getByRole("dialog", { name: "Clear Preset?" });
  await expect(clear).toContainText("This cannot be undone.");
  await clear.getByRole("button", { name: "Clear" }).click();
  await expect(picker.locator(`option[value="${active}"]`)).toHaveText(`${n}: Empty`);
  await expect.poll(async () => (await coreState(page)).presetOccupied & (1 << active)).toBe(0);

  // Clear All Slots…: every slot.
  await menu(session, "Clear All Slots…");
  await page.getByRole("dialog", { name: "Clear All Presets?" }).getByRole("button", { name: "Clear All" }).click();
  await expect(picker.locator(`option[value="${dest}"]`)).toHaveText(`${dest + 1}: Empty`);
  st = await coreState(page);
  expect(st.presetOccupied).toBe(0);
  // With nothing stored, the menu has no Clear entries.
  await session.getByRole("button", { name: "Preset actions" }).click();
  await expect(session.getByRole("menuitem", { name: /Clear/ })).toHaveCount(0);
  await page.keyboard.press("Escape");
  expect(errors).toEqual([]);
});

test("WB6a tools: Commit, Revert to Saved and Factory Reset ask first, then the core does them", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openControl(page);
  const presets = session.getByRole("region", { name: "Presets" });
  const gain = session.getByRole("region", { name: "Outputs" }).getByRole("slider").first();

  // Commit: the live state into the active slot.
  await setRange(gain, -2);
  await expect(presets.getByText("Unsaved changes")).toBeVisible();
  await menu(session, "Commit Parameters…");
  const commit = page.getByRole("dialog", { name: "Save Preset" });
  await expect(commit).toContainText(`preset slot ${(await coreState(page)).activePresetSlot + 1}`);
  await commit.getByRole("button", { name: "Save" }).click();
  await expect(presets.getByText("Preset saved successfully")).toBeVisible();
  await expect(presets.getByText("Unsaved changes")).toHaveCount(0);
  expect(await unsavedLines(page)).toEqual([]);

  // Revert: back to what was committed. Cancel first changes nothing.
  await setRange(gain, -9);
  await expect(presets.getByText("Unsaved changes")).toBeVisible();
  await menu(session, "Revert to Saved…");
  await page.getByRole("dialog", { name: "Revert to Saved" }).getByRole("button", { name: "Cancel" }).click();
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(-9);
  await menu(session, "Revert to Saved…");
  await page.getByRole("dialog", { name: "Revert to Saved" }).getByRole("button", { name: "Revert" }).click();
  await expect(presets.getByText("Unsaved changes")).toHaveCount(0);
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(-2);
  await expect(session.getByRole("region", { name: "Outputs" }).locator(".ch-value").first()).toHaveText("-2.0 dB");

  // Factory Reset: the defaults (bypass off, trims at 0). // -- WP WB6L -- exact: the quick strip has "Bypass Master EQ"
  await session.getByRole("button", { name: "Bypass", exact: true }).click();
  await expect.poll(async () => (await coreState(page)).bypass).toBe(true);
  await menu(session, "Factory Reset…");
  await page.getByRole("dialog", { name: "Factory Reset" }).getByRole("button", { name: "Reset" }).click();
  await expect(session.getByRole("button", { name: "Bypass", exact: true })).toHaveAttribute("aria-pressed", "false");
  const st = await coreState(page);
  expect(st.bypass).toBe(false);
  expect(st.outputGainDb[0]).toBe(0);
  expect(errors).toEqual([]);
});

test("WB6a channels: inputs and outputs are renamed, trimmed and muted, each read back", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openControl(page);
  const st0 = await coreState(page);
  const inputs = session.getByRole("region", { name: "Inputs" });
  const outputs = session.getByRole("region", { name: "Outputs" });
  // The live inputs, as the Console lists them: on USB, what the device streams.
  expect(st0.inputSource).toBe(0);
  const status = (await page.evaluate(() => (globalThis as unknown as Clients).dspiClients[0]!.call("meters", null))) as {
    activeInputChannels: number;
  };
  await expect(inputs.getByRole("listitem")).toHaveCount(Math.max(2, Math.min(status.activeInputChannels, st0.numInputChannels)));

  // Rename the first input (Enter in the field saves).
  const first = st0.channelNames[0]!;
  await inputs.getByRole("button", { name: `Rename ${first}` }).click();
  const dlg = page.getByRole("dialog", { name: `Rename ${first}` });
  await dlg.getByRole("textbox", { name: "Name" }).fill("Left in");
  await page.keyboard.press("Enter");
  await expect(inputs.getByRole("button", { name: "Rename Left in" })).toBeVisible();
  await expect.poll(async () => (await coreState(page)).channelNames[0]).toBe("Left in");

  // The input's trim (`pre`, the registry's "Input trim").
  const trim = inputs.getByRole("slider", { name: "Left in Input trim" });
  await setRange(trim, -4);
  await expect(inputs.locator(".ch-value").first()).toHaveText("-4.0 dB");
  await expect.poll(async () => (await coreState(page)).inputPreampDb[0]).toBe(-4);
  // A secondary click puts it back at 0 dB, as on the Console.
  await trim.click({ button: "right" });
  await expect(inputs.locator(".ch-value").first()).toHaveText("0.0 dB");
  await expect.poll(async () => (await coreState(page)).inputPreampDb[0]).toBe(0);

  // The first output: trim and mute.
  const gain = outputs.getByRole("slider", { name: re(`${OUT(0)} Output gain`) });
  await setRange(gain, -3.5);
  await expect(outputs.locator(".ch-value").first()).toHaveText("-3.5 dB");
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(-3.5);
  const mute = outputs.getByRole("button", { name: re(`Mute ${OUT(0)}`) });
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
  await setRange(session.getByRole("region", { name: "Outputs" }).getByRole("slider").first(), -5);
  // -- WP WB6F -- exact: the status rail says "· unsaved changes" too
  await expect(session.getByText("Unsaved changes", { exact: true })).toBeVisible();
  await session.getByRole("button", { name: "Disconnect" }).click();
  const prompt = page.getByRole("dialog", { name: "Unsaved Changes" });
  await prompt.getByRole("button", { name: "Cancel" }).click();
  await expect(session).toBeVisible();
  await session.getByRole("button", { name: "Disconnect" }).click();
  await prompt.getByRole("button", { name: "Discard" }).click();
  await expect(page.locator("#one").getByText("Choose a DSPi to connect to.")).toBeVisible();
});

test("WB6a read only: a view-only session shows the main window with every control off", async ({ page }) => {
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue B" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await expect(session.getByRole("region", { name: "Presets" })).toBeVisible();
  await expect(session.locator(".read-only-reason")).toHaveText("You can view this DSPi but not change it");
  for (const slider of await session.getByRole("slider").all()) await expect(slider).toBeDisabled();
  await expect(session.getByRole("combobox", { name: "Preset", exact: true })).toBeDisabled();
  await expect(session.getByRole("button", { name: "Preset actions" })).toBeDisabled();
  for (const name of ["Mute", "Bypass"]) await expect(session.getByRole("button", { name, exact: true })).toBeDisabled();
  for (const b of await session.locator(".ch-name, .ch-mute").all()) await expect(b).toBeDisabled();
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
    await expect(session.getByRole("region", { name: "Presets" })).toBeVisible();
    const screen = session.locator(".main-screen");
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
    // One column: the cards are stacked.
    const boxes = await Promise.all(
      ["Presets", "Inputs", "Outputs"].map((n) => session.getByRole("region", { name: n }).boundingBox()),
    );
    for (let i = 1; i < boxes.length; i++) expect(boxes[i]!.y).toBeGreaterThan(boxes[i - 1]!.y + boxes[i - 1]!.height - 1);
    const height = (await screen.boundingBox())!.height;

    // Unsaved, muted, renamed long, menu open: the same height, still no sideways scroll.
    const outputs = session.getByRole("region", { name: "Outputs" });
    await setRange(outputs.getByRole("slider").first(), -6);
    // -- WP WB6F -- exact: the status rail says "· unsaved changes" too
    await expect(session.getByText("Unsaved changes", { exact: true })).toBeVisible();
    await outputs.locator(".ch-mute").first().click();
    await expect(outputs.locator(".ch-mute").first()).toHaveAttribute("aria-pressed", "true");
    await session.getByRole("button", { name: "Mute", exact: true }).click();
    const name = session.getByRole("region", { name: "Inputs" }).locator(".ch-name").first();
    await name.click();
    await page.getByRole("dialog").getByRole("textbox").fill("A very long input name, cut");
    await page.keyboard.press("Enter");
    await expect(name).toContainText("A very long");
    expect(Math.abs((await screen.boundingBox())!.height - height)).toBeLessThan(0.5);
    await session.getByRole("button", { name: "Preset actions" }).click();
    await expect(session.getByRole("menu")).toBeVisible();
    expect(await overflow()).toEqual([]);
    // Keyboard: the menu opened on its first item; Escape gives focus back to its button.
    await expect(session.getByRole("menuitem", { name: "Save", exact: true })).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(session.getByRole("menuitem", { name: "Rename…" })).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(session.getByRole("button", { name: "Preset actions" })).toBeFocused();
    expect(Math.abs((await screen.boundingBox())!.height - height)).toBeLessThan(0.5);
  });
});
