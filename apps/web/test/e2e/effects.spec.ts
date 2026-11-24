// WP WB6h e2e: the effect screens on the core's demo device, in Chromium,
// Firefox and Chromium at 375 px. Every change is made through the page and
// read back twice: the page shows the device's value, and the core's own
// state (a direct call) agrees. A slider's drag and a crossfeed change under
// a named voicing are one undo step each; a read-only session shows it all
// with the controls off.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage, partsOf } from "../nav"; // -- WP WEBM3 -- partsOf

type State = Record<string, unknown> & {
  crossfeedEnabled: boolean;
  crossfeedPreset: number;
  crossfeedFreq: number;
  crossfeedFeed: number;
  loudnessIntensity: number;
  psybassOutputMask: number;
  tube: number[];
  subharm: number[];
  subharmSolo: boolean;
  upmix: number[];
};
type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[] };

async function core(page: Page, n = 0): Promise<State> {
  const snap = (await page.evaluate((n) => (globalThis as unknown as G).dspiClients[n]!.call("state", null), n)) as { state: State };
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

/** Console one, on the demo, at the Effects screen. */
async function openEffects(page: Page) {
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await openPage(session, "Effects");
  const screen = session.getByRole("region", { name: "Effects" });
  await expect(partsOf(screen, "Effects")).toBeVisible();
  return { session, screen };
}

function effect(screen: Locator, name: string) {
  return partsOf(screen, "Effects").getByRole("tab", { name: new RegExp(`^${name} (On|Off)$`) });
}

async function show(screen: Locator, name: string) {
  await effect(screen, name).click();
  const panel = screen.getByRole("tabpanel", { name });
  await expect(panel).toBeVisible();
  return panel;
}

async function type(field: Locator, text: string) {
  await field.click();
  await field.fill(text);
  await field.press("Enter");
}

test("WB6h: the demo lists all seven effects, each with its state; a switch turns one on, read back in the list and the core", async ({ page }) => {
  const errors = errorsOf(page);
  const { screen } = await openEffects(page);
  for (const name of ["Crossfeed", "Loudness", "Leveller", "Psychoacoustic bass", "Tube", "Subharmonic", "Upmixer"]) await expect(effect(screen, name)).toBeVisible();
  const was = (await core(page)).crossfeedEnabled;
  const panel = await show(screen, "Crossfeed");
  const sw = panel.getByRole("switch", { name: "Crossfeed", exact: true });
  await expect(sw).toHaveAttribute("aria-checked", String(was));
  await sw.click();
  await expect.poll(async () => (await core(page)).crossfeedEnabled).toBe(!was);
  await expect(sw).toHaveAttribute("aria-checked", String(!was));
  await expect(effect(screen, "Crossfeed")).toHaveText(new RegExp(`^Crossfeed ${!was ? "On" : "Off"}$`));
  // Each effect opens at its own panel, addressable by its id.
  await effect(screen, "Upmixer").click();
  await expect(screen.locator("#effects-upmix")).toBeVisible();
  expect(errors).toEqual([]);
});

test("WB6h P1XF: a named voicing shows its own values; a typed frequency makes it Custom from them, and one Undo puts the voicing back", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, screen } = await openEffects(page);
  const panel = await show(screen, "Crossfeed");
  const voicing = panel.getByRole("combobox", { name: "Crossfeed Voicing" });
  await voicing.selectOption({ label: "Chu Moy" });
  await expect.poll(async () => (await core(page)).crossfeedPreset).toBe(1);
  const freq = panel.getByRole("textbox", { name: "Crossfeed Cutoff frequency" });
  const feed = panel.getByRole("textbox", { name: "Crossfeed Feed level" });
  await expect(freq).toHaveValue("700 Hz");
  await expect(feed).toHaveValue("6.0 dB");
  await expect(panel.getByText("700 Hz / 6.0 dB")).toBeVisible();
  await type(freq, "900");
  await expect.poll(async () => {
    const s = await core(page);
    return [s.crossfeedPreset, s.crossfeedFreq, s.crossfeedFeed];
  }).toEqual([3, 900, 6]);
  await expect(voicing).toHaveValue("3");
  await expect(freq).toHaveValue("900 Hz");
  await expect(feed).toHaveValue("6.0 dB");
  await session.getByRole("button", { name: "Undo" }).click();
  await expect.poll(async () => (await core(page)).crossfeedPreset).toBe(1);
  await expect(freq).toHaveValue("700 Hz");
  expect(errors).toEqual([]);
});

test("WB6h: a slider's drag is one undo step; keys step it", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, screen } = await openEffects(page);
  const panel = await show(screen, "Loudness");
  const slider = panel.getByRole("slider", { name: "Loudness Intensity" });
  const before = (await core(page)).loudnessIntensity;
  await slider.scrollIntoViewIfNeeded();
  const box = (await slider.boundingBox())!;
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + box.width * 0.2, y);
  await page.mouse.down();
  for (let i = 1; i <= 8; i++) await page.mouse.move(box.x + box.width * (0.2 + i * 0.07), y);
  await page.mouse.up();
  await expect.poll(async () => (await core(page)).loudnessIntensity).toBeGreaterThan(before + 20);
  await expect.poll(async () => Number(await slider.inputValue())).toBe((await core(page)).loudnessIntensity);
  await session.getByRole("button", { name: "Undo" }).click();
  await expect.poll(async () => (await core(page)).loudnessIntensity).toBe(before);
  await slider.focus();
  await page.keyboard.press("ArrowRight");
  await expect.poll(async () => (await core(page)).loudnessIntensity).toBe(before + 1);
  expect(errors).toEqual([]);
});

test("WB6h: an output chip toggles its bit of the effect's mask; a choice and a typed value read back", async ({ page }) => {
  const errors = errorsOf(page);
  const { screen } = await openEffects(page);
  const bass = await show(screen, "Psychoacoustic bass");
  const mask0 = (await core(page)).psybassOutputMask;
  const chip = bass.getByRole("group", { name: "Psychoacoustic bass Outputs" }).getByRole("button", { name: "OUT1", exact: true });
  await expect(chip).toHaveAttribute("aria-pressed", String((mask0 & 1) !== 0));
  await chip.click();
  await expect.poll(async () => (await core(page)).psybassOutputMask).toBe((mask0 ^ 1) >>> 0);
  await expect(chip).toHaveAttribute("aria-pressed", String((mask0 & 1) === 0));
  await type(bass.getByRole("textbox", { name: "Psychoacoustic bass Speaker limit" }), "120 Hz");
  await expect.poll(async () => (await core(page)).psybassCutoffHz).toBe(120);

  const up = await show(screen, "Upmixer");
  await up.getByRole("combobox", { name: "Upmixer Surround mode" }).selectOption({ label: "Logician" });
  await expect.poll(async () => (await core(page)).upmix[2]).toBe(2);
  await type(up.getByRole("textbox", { name: "Upmixer Centre presence" }), "2.5");
  await expect.poll(async () => (await core(page)).upmix[13]).toBe(2.5);
  expect(errors).toEqual([]);
});

test("WB6h M38: the tube's Basic view has the type, drive, mix and outputs, Advanced every parameter; a type loads its character", async ({ page }) => {
  const errors = errorsOf(page);
  const { screen } = await openEffects(page);
  const tube = await show(screen, "Tube");
  await expect(tube.getByRole("slider", { name: "Tube Drive" })).toBeVisible();
  await expect(tube.getByRole("slider", { name: "Tube Bias" })).toHaveCount(0);
  await tube.getByRole("button", { name: "Advanced" }).click();
  await expect(tube.getByRole("slider", { name: "Tube Bias" })).toBeVisible();
  await expect(tube.getByRole("combobox", { name: "Tube Rectifier" })).toBeVisible();
  const type0 = tube.getByRole("combobox", { name: "Tube Tube", exact: true });
  await type0.selectOption({ label: "12AX7 / ECC83" });
  await expect.poll(async () => (await core(page)).tube[2]).toBe(1);
  const bias1 = (await core(page)).tube[4];
  await type0.selectOption({ index: 3 });
  await expect.poll(async () => (await core(page)).tube[2]).toBe(3);
  await expect.poll(async () => (await core(page)).tube[4]).not.toBe(bias1);
  await tube.getByRole("button", { name: "Basic" }).click();
  await expect(tube.getByRole("slider", { name: "Tube Bias" })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("WB6h: the subharmonic synthesizer's solo is switched off on leaving it, as the Console's window does", async ({ page }) => {
  const errors = errorsOf(page);
  const { screen } = await openEffects(page);
  const sub = await show(screen, "Subharmonic");
  const solo = sub.getByRole("button", { name: "Subharmonic Solo" }); // -- WP WEB2 -- a toggle chip (Qt's ToggleChip)
  await solo.click();
  await expect.poll(async () => (await core(page)).subharmSolo).toBe(true);
  await show(screen, "Tube");
  await expect.poll(async () => (await core(page)).subharmSolo).toBe(false);
  expect(errors).toEqual([]);
});

test("WB6h: a read-only session shows the effects with every control off", async ({ page }) => {
  const errors = errorsOf(page);
  await page.goto("/?readonly");
  await page.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article", { name: "DSPi" });
  await openPage(session, "Effects");
  const screen = session.getByRole("region", { name: "Effects" });
  const panel = await show(screen, "Loudness");
  await expect(panel.getByRole("switch", { name: "Loudness", exact: true })).toBeDisabled();
  await expect(panel.getByRole("slider", { name: "Loudness Intensity" })).toBeDisabled();
  const cf = await show(screen, "Crossfeed");
  await expect(cf.getByRole("combobox", { name: "Crossfeed Voicing" })).toBeDisabled();
  expect(errors).toEqual([]);
});
