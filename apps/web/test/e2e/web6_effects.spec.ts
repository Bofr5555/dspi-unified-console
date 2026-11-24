// WP WEB6 e2e: the Effects pages as Qt's Modern ones on the core's demo device: the graphs (the
// core's curves, "Disabled" while off), the tube's picture and type chips, the masks' "Presets ▾",
// "Apply preset ▾" as one undo step, Subharmonic's headroom cost and "Off" levels, the upmixer's
// status and Presence. Every change is read back from the core.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage, partsOf } from "../nav";

type State = Record<string, unknown> & {
  crossfeedEnabled: boolean;
  psybassEnabled: boolean;
  psybassOutputMask: number;
  psybassCutoffHz: number;
  psybassOriginalDb: number;
  psybassHarmonicsDb: number;
  numOutputChannels: number;
  tube: number[];
  subharm: number[];
  upmix: number[];
};
type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[] };

async function call(page: Page, method: string, params: unknown = null): Promise<unknown> {
  return page.evaluate(([m, p]) => (globalThis as unknown as G).dspiClients[0]!.call(m as string, p), [method, params] as const);
}

async function core(page: Page): Promise<State> {
  return ((await call(page, "state")) as { state: State }).state;
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

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

async function show(screen: Locator, name: string) {
  await partsOf(screen, "Effects").getByRole("tab", { name: new RegExp(`^${name} (On|Off)$`) }).click();
  const panel = screen.getByRole("tabpanel", { name });
  await expect(panel).toBeVisible();
  return panel;
}

async function switchOn(panel: Locator, name: string) {
  const sw = panel.locator(".fx-intro").getByRole("switch", { name, exact: true });
  if ((await sw.getAttribute("aria-checked")) !== "true") await sw.click();
  await expect(sw).toHaveAttribute("aria-checked", "true");
}

async function type(field: Locator, text: string) {
  await field.click();
  await field.fill(text);
  await field.press("Enter");
}

test("WEB6: an effect's graph says Disabled while it is off and draws the core's curves once it is on", async ({ page }) => {
  const errors = errorsOf(page);
  const { screen } = await openEffects(page);
  const cf = await show(screen, "Crossfeed");
  const graph = cf.getByTestId("fx-curve-graph");
  if ((await core(page)).crossfeedEnabled) await cf.locator(".fx-intro").getByRole("switch", { name: "Crossfeed", exact: true }).click();
  await expect(graph).toHaveAttribute("data-active", "false");
  await expect(graph).toContainText("Disabled");
  await switchOn(cf, "Crossfeed");
  await expect(graph).toHaveAttribute("data-active", "true");
  await expect(graph.locator('path[data-curve="direct"]')).toHaveCount(1);
  await expect(graph.locator('path[data-curve="crossfeed"]')).toHaveCount(1);
  await expect(graph).toContainText("Direct");
  // The curve is the core's: a feed change redraws it
  const before = await graph.locator('path[data-curve="crossfeed"]').getAttribute("d");
  await type(cf.getByRole("textbox", { name: "Crossfeed Feed Level" }), "9.5");
  await expect.poll(async () => graph.locator('path[data-curve="crossfeed"]').getAttribute("d")).not.toBe(before);

  const loud = await show(screen, "Loudness");
  await switchOn(loud, "Loudness");
  await expect(loud.getByTestId("fx-curve-graph").locator('path[data-curve="curve"]')).toHaveCount(1);
  await expect(loud.getByText("Curve at -40dB")).toBeVisible();

  const sub = await show(screen, "Subharmonic");
  await switchOn(sub, "Subharmonic");
  await expect(sub.getByTestId("fx-subharm-graph").locator("[data-sub]").first()).toBeAttached();
  expect(errors).toEqual([]);
});

test("WEB6: the tube's Basic view is the picture and Qt's grouped type chips; a chip loads that type", async ({ page }) => {
  const errors = errorsOf(page);
  const { screen } = await openEffects(page);
  const tube = await show(screen, "Tube");
  await expect(tube.getByTestId("fx-tube-tile")).toBeVisible();
  await expect(tube.getByRole("combobox", { name: "Tube Tube", exact: true })).toHaveCount(0);
  const power = tube.getByRole("radiogroup", { name: "Tube Power stages" });
  await power.getByRole("radio", { name: "EL34", exact: true }).click();
  await expect.poll(async () => (await core(page)).tube[2]).toBe(12);
  await expect(power.getByRole("radio", { name: "EL34", exact: true })).toHaveAttribute("aria-checked", "true");
  await expect(tube.locator(".fx-tube-name")).toHaveText("EL34");
  await expect(tube.locator(".fx-tube-picture")).toHaveAttribute("data-family", "5");
  await tube.getByRole("radiogroup", { name: "Tube Preamp triodes" }).getByRole("radio", { name: "12AX7", exact: true }).click();
  await expect.poll(async () => (await core(page)).tube[2]).toBe(1);
  await expect(tube.locator(".fx-tube-picture")).toHaveAttribute("data-family", "0");
  // The heater glows while the stage is on
  await switchOn(tube, "Tube");
  await expect(tube.locator(".fx-tube-picture")).toHaveAttribute("data-on", "true");
  // Advanced: the transfer curve and the harmonics at full scale
  await tube.getByRole("button", { name: "Advanced" }).click();
  await expect(tube.getByTestId("fx-tube-graph")).toHaveAttribute("data-active", "true");
  await expect(tube.getByTestId("fx-harmonics")).toContainText(/2nd\s+(none|-?\d+ dB)/);
  expect(errors).toEqual([]);
});

test("WEB6: a mask's Presets menu writes Qt's mask", async ({ page }) => {
  const errors = errorsOf(page);
  const { screen } = await openEffects(page);
  const bass = await show(screen, "Psy Bass");
  const outputs = bass.getByRole("region", { name: "Outputs" });
  await outputs.getByRole("button", { name: "Presets" }).click();
  await page.getByRole("menuitem", { name: "None" }).click();
  await expect.poll(async () => (await core(page)).psybassOutputMask).toBe(0);
  await outputs.getByRole("button", { name: "Presets" }).click();
  await page.getByRole("menuitem", { name: "Exclude sub (recommended)" }).click();
  const n = (await core(page)).numOutputChannels;
  await expect.poll(async () => (await core(page)).psybassOutputMask).toBe(((1 << n) - 1) & ~(1 << (n - 1)));
  await expect(outputs.getByRole("button", { name: "OUT1", exact: true })).toHaveAttribute("aria-pressed", "true");
  expect(errors).toEqual([]);
});

test("WEB6: Apply preset writes a starting point as one undo step", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, screen } = await openEffects(page);
  const bass = await show(screen, "Psy Bass");
  const was = await core(page);
  await bass.getByRole("region", { name: "Spectrum" }).getByRole("button", { name: "Apply preset" }).click();
  await page.getByRole("menuitem", { name: "Laptop / tablet - Tiny drivers, protect them" }).click();
  await expect.poll(async () => {
    const s = await core(page);
    return [s.psybassCutoffHz, s.psybassHarmonicsDb, s.psybassOriginalDb];
  }).toEqual([180, 6, -24]);
  await session.getByRole("button", { name: "Undo" }).click();
  await expect.poll(async () => {
    const s = await core(page);
    return [s.psybassCutoffHz, s.psybassHarmonicsDb, s.psybassOriginalDb];
  }).toEqual([was.psybassCutoffHz, was.psybassHarmonicsDb, was.psybassOriginalDb]);
  expect(errors).toEqual([]);
});

test("WEB6: Subharmonic in Qt's order, a band at its floor reads Off, the headroom cost is the device's", async ({ page }) => {
  const errors = errorsOf(page);
  const { screen } = await openEffects(page);
  const sub = await show(screen, "Subharmonic");
  const labels = await sub.locator(".fx-params > .ds-section").evaluateAll((els) => els.map((e) => e.getAttribute("aria-label")));
  expect(labels).toEqual(["Selectivity", "Sub ceiling", "LF boost", "Outputs"]);
  const low = sub.getByRole("textbox", { name: "Subharmonic 24 - 36 Hz" });
  await type(low, "-30");
  await expect.poll(async () => (await core(page)).subharm[2]).toBe(-30);
  await expect(low).toHaveValue("Off");
  await type(low, "6");
  await expect.poll(async () => (await core(page)).subharm[2]).toBe(6);
  await expect(low).toHaveValue("6.0 dB");
  await switchOn(sub, "Subharmonic");
  const db = Number(await call(page, "subharmHeadroom"));
  await expect(sub.getByTestId("fx-headroom")).toContainText(db > 0 ? `+${db.toFixed(1)} dB` : "none");
  expect(errors).toEqual([]);
});

test("WEB6: the upmixer's status, Presence under Centre Width, the engines Off first", async ({ page }) => {
  const errors = errorsOf(page);
  const { screen } = await openEffects(page);
  const up = await show(screen, "Upmixer");
  if ((await core(page)).upmix[0]) await up.locator(".fx-intro").getByRole("switch", { name: "Upmixer", exact: true }).click();
  await expect(up.getByRole("region", { name: "Status" })).toContainText("Idle: upmixer disabled");
  const centre = up.getByRole("region", { name: "Centre" });
  const sliders = await centre.getByRole("slider").evaluateAll((els) => els.map((e) => e.getAttribute("aria-label")));
  expect(sliders.slice(0, 3)).toEqual(["Upmixer Strength", "Upmixer Centre Width", "Upmixer Presence"]);
  await expect(up.getByRole("group", { name: "Upmixer Centre", exact: true }).getByRole("button")).toHaveText(["Off", "Sinner", "Logician"]);
  expect(errors).toEqual([]);
});
