// WP UB7 e2e (UXP B7, C3): one speaker table, shown once, edited in one place, on the core's demo
// device. Settings › Speakers: the headings, the glossary's roles, one word for the suggestion, a
// speaker's name that sets what it is measured with, the override and off again. Measure and
// Auto-tune: the layout in one line with Edit in Speakers…, only tick boxes; Edit opens the table.
// At 375 px nothing scrolls sideways. The narrow project keeps pictures (dark and light).

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage, openSetting, partsOf } from "../nav";

type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };

async function call<T>(page: Page, method: string, params: unknown = null): Promise<T> {
  return (await page.evaluate(([m, p]) => (globalThis as unknown as Clients).dspiClients[0]!.call(m as string, p), [method, params] as const)) as T;
}

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(session.getByRole("button", { name: /^Presets: / })).toBeVisible();
  return session;
}

async function speakersPage(session: Locator): Promise<Locator> {
  await openPage(session, "Settings");
  await openSetting(session, "Speakers");
  const page = session.getByRole("region", { name: "Speakers" });
  await expect(page.getByRole("list", { name: "Speaker layout" })).toBeVisible();
  return page;
}

async function noSidewaysScroll(page: Page) {
  if (test.info().project.name !== "narrow") return;
  const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(over).toBeLessThanOrEqual(0);
}

async function pictures(page: Page, what: Locator, name: string) {
  if (!["narrow", "chromium"].includes(test.info().project.name)) return;
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await what.screenshot({ path: test.info().outputPath(`${name}-${scheme}.png`) });
  }
  await page.emulateMedia({ colorScheme: "light" });
}

// The report's pictures, before any of this package's assertions (so the same test takes them on the base)
test("UB7: pictures of Settings › Speakers, Measure and Auto-tune", async ({ page }) => {
  const session = await openDemo(page);
  const sp = await speakersPage(session);
  await pictures(page, sp, "pic-speakers");
  await openPage(session, "Measure");
  const setup = session.getByRole("region", { name: "Measurement setup" });
  await expect(setup).toBeVisible();
  await pictures(page, setup.locator("fieldset").filter({ hasText: "Outputs" }), "pic-measure-outputs");
  const screen = session.getByRole("region", { name: "Measure" });
  await partsOf(screen, "Measurement").getByRole("tab", { name: "Auto-tune" }).click();
  const tune = screen.getByRole("region", { name: "Tune layout" });
  await expect(tune.getByRole("list", { name: "Outputs" })).toBeVisible();
  await pictures(page, tune, "pic-tune-layout");
});

test("UB7: the one speaker table: headings, names that set the groups, the override", async ({ page }) => {
  const session = await openDemo(page);
  const sp = await speakersPage(session);
  const head = sp.locator(".speaker-table-head");
  if (test.info().project.name !== "narrow") await expect(head).toHaveText(/Output\s*Role\s*Speaker\s*Measured together/i);
  await expect(sp.getByRole("button", { name: "Suggest from Routing and Filters" })).toBeVisible();
  await expect(sp.getByRole("button", { name: /^Use the suggestion$/ })).toHaveCount(0);
  const role1 = sp.getByRole("combobox", { name: /^Role of OUT1 / });
  await expect(role1.locator("option")).toHaveText(["Full range", "Subwoofer", "Woofer", "Mid driver", "Tweeter"]);

  // The suggestion's speakers: two outputs of different speakers
  const proposal = await call<{ outputs: { output: number; speaker: string }[] }>(page, "speakerLayoutPropose");
  const a = proposal.outputs[0]!;
  const b = proposal.outputs.find((o) => o.speaker.trim().toLowerCase() !== a.speaker.trim().toLowerCase())!;
  expect(b, "the suggestion has two speakers").toBeTruthy();
  const together = (o: number) => sp.getByRole("listitem", { name: new RegExp(`^OUT${o + 1} `) }).locator(".speaker-table-together");
  await expect(together(a.output)).toHaveText(a.speaker);
  await expect(sp.getByRole("combobox", { name: /is measured with$/ })).toHaveCount(0);
  // Named like the other: measured with it
  const nameB = sp.getByRole("textbox", { name: new RegExp(`^Speaker of OUT${b.output + 1} `) });
  await nameB.fill(a.speaker.toLowerCase());
  await expect(together(b.output)).toHaveText(a.speaker);
  // A new name: on its own
  await nameB.fill("Rear");
  await expect(together(b.output)).toHaveText("Rear");
  await expect(together(a.output)).toHaveText(a.speaker);
  await noSidewaysScroll(page);
  await pictures(page, sp, "ub7-speakers");

  // The override: a picker per row; a pick puts it with the other speaker; off again, the names decide
  const byHand = sp.getByRole("checkbox", { name: "Choose which outputs are measured together" });
  await expect(byHand).not.toBeChecked();
  await byHand.check();
  const pickB = sp.getByRole("combobox", { name: new RegExp(`^OUT${b.output + 1} .* is measured with$`) });
  await expect(pickB).toBeVisible();
  await pickB.selectOption({ label: a.speaker });
  await expect(sp.getByRole("combobox", { name: new RegExp(`^OUT${a.output + 1} .* is measured with$`) }).locator("option:checked")).toHaveText(`${a.speaker} + Rear`);
  await noSidewaysScroll(page);
  await pictures(page, sp, "ub7-speakers-byhand");
  await byHand.uncheck();
  await expect(byHand).not.toBeChecked();
  await expect(together(b.output)).toHaveText("Rear");
  await expect(together(a.output)).toHaveText(a.speaker);
});

test("UB7: Measure and Auto-tune show the layout in one line and only tick boxes; Edit in Speakers… opens the table", async ({ page }) => {
  const session = await openDemo(page);
  const sp = await speakersPage(session);
  await sp.getByRole("button", { name: "Confirm" }).click();
  await expect(sp.getByRole("status", { name: "Speaker layout saved" })).toContainText("Confirmed");

  await openPage(session, "Measure");
  const setup = session.getByRole("region", { name: "Measurement setup" });
  const line = setup.getByRole("status", { name: "Speaker layout summary" });
  await expect(line).toContainText(/^L: .*Out 1\b.* · confirmed/);
  await expect(setup.getByRole("combobox", { name: /^Way of / })).toHaveCount(0);
  await expect(setup.getByRole("textbox", { name: /^Speaker of / })).toHaveCount(0);
  await expect(setup.getByRole("button", { name: "Propose from the Device" })).toHaveCount(0);
  // A tick is this run's: the layout stays as it was
  const before = await call<{ outputs: unknown[] }>(page, "speakerLayout");
  const first = setup.getByRole("list", { name: "Outputs to measure" }).getByRole("checkbox").first();
  await first.setChecked(!(await first.isChecked()));
  expect(await call<{ outputs: unknown[] }>(page, "speakerLayout")).toEqual(before);
  await noSidewaysScroll(page);
  await pictures(page, setup, "ub7-measure");

  const said = await line.innerText();
  const screen = session.getByRole("region", { name: "Measure" });
  await partsOf(screen, "Measurement").getByRole("tab", { name: "Auto-tune" }).click();
  const tune = screen.getByRole("region", { name: "Tune layout" });
  await expect(tune.getByRole("status", { name: "Speaker layout summary" })).toHaveText(said);
  await expect(tune.getByRole("combobox", { name: /^Way of / })).toHaveCount(0);
  await expect(tune.getByRole("button", { name: "Propose from the Device" })).toHaveCount(0);
  await noSidewaysScroll(page);
  await pictures(page, tune, "ub7-tune");

  await tune.getByRole("button", { name: "Edit in Speakers…" }).click();
  await expect(session.getByRole("region", { name: "Speakers" }).getByRole("list", { name: "Speaker layout" })).toBeVisible();
});
