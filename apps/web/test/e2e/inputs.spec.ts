// WP WB6g e2e: the input pages on the core's demo device, in Chromium, Firefox
// and Chromium at 375 px. The trims by pair with the L/R link (M48): a linked
// pair moves together as one undo step; linking a pair that differs asks
// which side to keep; the link heads an input's EQ page and mirrors its band
// edits; the input sources' names are kept on the DSPi; the matrix's
// 8-channel layout carries each input's trim. Every change is made through the
// page and read back from the core's own state (a direct call).

import { expect, test, type Locator, type Page } from "@playwright/test";

type Band = { filterType: number; bypass: boolean; freq: number; q: number; gain: number; qp: number };
type State = { numInputChannels: number; inputPreampDb: number[]; filters: Band[][]; inputSource: number };
type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[] };

async function core(page: Page): Promise<State> {
  const snap = (await page.evaluate(() => (globalThis as unknown as G).dspiClients[0]!.call("state", null))) as { state: State };
  return snap.state;
}

async function names(page: Page): Promise<string[]> {
  const r = (await page.evaluate(() => (globalThis as unknown as G).dspiClients[0]!.call("inputSourceNames", null))) as { names: string[] };
  return r.names;
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

async function setRange(slider: Locator, value: number) {
  await slider.fill(String(value));
  await slider.dispatchEvent("change");
}

async function openInputs(page: Page, venue = "Venue A") {
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: venue }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await session.getByRole("tab", { name: "Inputs" }).click();
  const trims = session.getByRole("region", { name: "Input trims" });
  await expect(trims).toBeVisible();
  return { session, trims };
}

const trimOf = (where: Locator, n: number) => where.getByRole("slider", { name: new RegExp(`^IN${n} .* Input trim$`) });

test("WB6G M48: a linked pair's trims move together, one undo step; unlinked, alone", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, trims } = await openInputs(page);
  const pair = trims.getByRole("group", { name: "Inputs 1/2" });
  const link = pair.getByRole("button", { name: "Link 1/2" });
  await expect(link).toHaveAttribute("aria-pressed", "true"); // the Console's default
  const before = (await core(page)).inputPreampDb.slice(0, 2);

  await setRange(trimOf(pair, 1), -4);
  await expect.poll(async () => (await core(page)).inputPreampDb.slice(0, 2)).toEqual([-4, -4]);
  await expect(pair.getByRole("textbox", { name: "IN2 trim" })).toHaveValue("-4.0 dB");
  // One step puts both back.
  await session.getByRole("button", { name: "Undo" }).click();
  await expect.poll(async () => (await core(page)).inputPreampDb.slice(0, 2)).toEqual(before);

  // Unlinked: one side only; typed values too.
  await link.click();
  await expect(link).toHaveAttribute("aria-pressed", "false");
  const field = pair.getByRole("textbox", { name: "IN1 trim" });
  await field.fill("-2.5");
  await field.press("Enter");
  await expect.poll(async () => (await core(page)).inputPreampDb.slice(0, 2)).toEqual([-2.5, before[1]]);
  expect(errors).toEqual([]);
});

test("WB6G M48: linking a pair that differs asks which side to keep; Cancel leaves it, Keep IN2 copies IN2", async ({ page }) => {
  const { session, trims } = await openInputs(page);
  const pair = trims.getByRole("group", { name: "Inputs 1/2" });
  const link = pair.getByRole("button", { name: "Link 1/2" });
  await link.click();
  await expect(link).toHaveAttribute("aria-pressed", "false");
  await setRange(trimOf(pair, 2), -6);
  await expect.poll(async () => (await core(page)).inputPreampDb[1]).toBe(-6);
  const in1 = (await core(page)).inputPreampDb[0];

  await link.click();
  const dlg = page.getByRole("dialog", { name: "Inputs 1 and 2 don't match" });
  await expect(dlg).toContainText("These inputs have different input trims.");
  await dlg.getByRole("button", { name: "Cancel" }).click();
  await expect(link).toHaveAttribute("aria-pressed", "false");
  expect((await core(page)).inputPreampDb[0]).toBe(in1);

  await link.click();
  await page.getByRole("dialog", { name: "Inputs 1 and 2 don't match" }).getByRole("button", { name: "Keep IN2" }).click();
  await expect(link).toHaveAttribute("aria-pressed", "true");
  await expect.poll(async () => (await core(page)).inputPreampDb.slice(0, 2)).toEqual([-6, -6]);
  // The copy is one undo step: IN1 back as it was, still linked.
  await session.getByRole("button", { name: "Undo" }).click();
  await expect.poll(async () => (await core(page)).inputPreampDb.slice(0, 2)).toEqual([in1, -6]);
  // The link is kept for this DSPi: a new visit finds it.
  await page.reload();
  const again = await openInputs(page);
  await expect(again.trims.getByRole("button", { name: "Link 1/2" })).toHaveAttribute("aria-pressed", "true");
});

test("WB6G M48: an input's EQ page has the link and the trim; a linked band edit reaches both inputs", async ({ page }) => {
  const { session } = await openInputs(page);
  await session.getByRole("tab", { name: "EQ" }).click();
  await session.locator(".eq-pick", { hasText: "IN1" }).click();
  const header = session.getByRole("region", { name: /^Input settings, IN1/ });
  await expect(header.getByRole("button", { name: "Link 1/2" })).toHaveAttribute("aria-pressed", "true");
  await setRange(trimOf(header, 1), -1.5);
  await expect.poll(async () => (await core(page)).inputPreampDb.slice(0, 2)).toEqual([-1.5, -1.5]);

  const list = session.getByRole("region", { name: /^Bands of IN1/ });
  const slot = (await core(page)).filters[0]!.findIndex((b) => b.filterType === 0);
  expect(slot).toBeGreaterThanOrEqual(0);
  await list.getByRole("combobox", { name: `Band ${slot + 1} type` }).selectOption("1");
  await expect.poll(async () => (await core(page)).filters.slice(0, 2).map((f) => f[slot]!.filterType)).toEqual([1, 1]);
  const gain = list.getByRole("textbox", { name: `Band ${slot + 1} gain` });
  await gain.fill("3.5");
  await gain.press("Enter");
  await expect.poll(async () => (await core(page)).filters.slice(0, 2).map((f) => f[slot]!.gain)).toEqual([3.5, 3.5]);
  // Unlinked from here: IN1 alone.
  await header.getByRole("button", { name: "Link 1/2" }).click();
  await gain.fill("-2");
  await gain.press("Enter");
  await expect.poll(async () => (await core(page)).filters[0]![slot]!.gain).toBe(-2);
  expect((await core(page)).filters[1]![slot]!.gain).toBe(3.5);
});

test("WB6G source names: typed, kept on the DSPi, the active source named; Escape drops a draft", async ({ page }) => {
  const errors = errorsOf(page);
  const { session } = await openInputs(page);
  const card = session.getByRole("region", { name: "Source names" });
  const spdif = card.getByRole("textbox", { name: "Name of S/PDIF" });
  await expect(spdif).toBeEnabled();
  await spdif.fill("Turntable");
  await spdif.press("Enter");
  await expect.poll(async () => (await names(page))[1]).toBe("Turntable");
  await expect(spdif).toHaveValue("Turntable");

  // The active source (USB on the demo) by its name.
  expect((await core(page)).inputSource).toBe(0);
  const usb = card.getByRole("textbox", { name: "Name of USB" });
  await usb.fill("Desk stream");
  await usb.blur();
  await expect.poll(async () => (await names(page))[0]).toBe("Desk stream");
  await expect(session.getByRole("region", { name: "Input trims" })).toContainText("Source: Desk stream");

  // Escape drops what was typed; an empty name gives the source its own name back.
  await usb.fill("Nope");
  await usb.press("Escape");
  await expect(usb).toHaveValue("Desk stream");
  await usb.fill("");
  await usb.press("Enter");
  await expect.poll(async () => (await names(page))[0]).toBe("");
  await expect(session.getByRole("region", { name: "Input trims" })).toContainText("Source: USB");
  expect(errors).toEqual([]);
});

test("WB6G read only: a view-only session shows the input pages with every control off", async ({ page }) => {
  const { session, trims } = await openInputs(page, "Venue B");
  await expect(trims.getByRole("button", { name: "Link 1/2" })).toBeDisabled();
  for (const s of await trims.getByRole("slider").all()) await expect(s).toBeDisabled();
  for (const t of await session.getByRole("region", { name: "Source names" }).getByRole("textbox").all()) await expect(t).toBeDisabled();
});

test("WB6G matrix: the 8-channel layout carries each input's trim, through the link", async ({ page }) => {
  const { session, trims } = await openInputs(page);
  const st = await core(page);
  test.skip(st.numInputChannels < 4, "the demo has two inputs: no 8-channel layout");
  // I2S with as many channels as the demo has, then a write from the page so it reads the state again.
  const n = Math.min(8, st.numInputChannels);
  await page.evaluate(async (n) => {
    const c = (globalThis as unknown as G).dspiClients[0]!;
    await c.call("write", { path: "in.i2s.channels", value: { int: n } });
    await c.call("write", { path: "in.source", value: { choice: 2 } });
  }, n);
  await setRange(trimOf(trims, 1), 0);
  await session.getByRole("tab", { name: "Matrix" }).click();
  const matrix = session.getByRole("region", { name: "Matrix Mixer" });
  const t3 = matrix.getByRole("textbox", { name: /^Input trim \(preamp\) for IN3 / });
  await expect(t3).toBeVisible();
  // IN3/IN4 are not linked by default: IN3 alone.
  await t3.fill("-3");
  await t3.press("Enter");
  await expect.poll(async () => (await core(page)).inputPreampDb.slice(2, 4)).toEqual([-3, st.inputPreampDb[3]]);
  // IN1, linked with IN2: both.
  const t1 = matrix.getByRole("textbox", { name: /^Input trim \(preamp\) for IN1 / });
  await t1.fill("-5");
  await t1.press("Enter");
  await expect.poll(async () => (await core(page)).inputPreampDb.slice(0, 2)).toEqual([-5, -5]);
});
