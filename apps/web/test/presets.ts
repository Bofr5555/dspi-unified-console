// -- WP INTD -- The session bar's preset popover (DL5, ui/shell/PresetPopover.tsx) and an unsaved
// change, for the e2e tests: the Overview's preset card, its level cards and the slot strip are gone,
// so a test opens the popover and changes an output's gain on Equaliser › Delay & Gain.

import { expect, type Locator } from "@playwright/test";
import { openPage } from "./nav";

/** The popover's button: "Presets: 1: Demo", ", unsaved changes" while the active slot has them. */
export const presetsButton = (session: Locator) => session.getByRole("button", { name: /^Presets: / });

/** The open popover. */
export const presetsPopover = (session: Locator) => session.getByRole("dialog", { name: "Presets" });

/** Open the popover (when it is not open) and give it. */
export async function openPresets(session: Locator): Promise<Locator> {
  const pop = presetsPopover(session);
  if (!(await pop.count())) await presetsButton(session).click();
  await expect(pop).toBeVisible();
  return pop;
}

/** A slot's row in the popover, by its number (1..10). */
export const slotRow = (pop: Locator, n: number) => pop.getByRole("listitem", { name: new RegExp(`^${n}: `) });

/** Run one of a slot's menu rows ("Rename…", "Set as Startup Preset", "Copy to…", "Clear…", …). */
export async function slotMenu(session: Locator, n: number, item: string): Promise<void> {
  const pop = await openPresets(session);
  await slotRow(pop, n).getByRole("button", { name: "More" }).click();
  await slotRow(pop, n).getByRole("menuitem", { name: item, exact: true }).click();
}

/** The session is open: its bar's preset control is there. */
export async function presetsReady(session: Locator): Promise<void> {
  await expect(presetsButton(session)).toBeVisible();
}

/** Output `out`'s gain typed on Equaliser › Delay & Gain (a preset change), and read back on the page. */
export async function setOutputGain(session: Locator, db: number, out = 0): Promise<void> {
  if ((await session.locator(".page[data-page]").getAttribute("data-page")) !== "delays") await openPage(session, "Delay & Gain");
  const row = session.getByRole("region", { name: "Delay and gain of every output" }).getByRole("row", { name: new RegExp(`^OUT${out + 1} `) });
  const field = row.getByRole("textbox", { name: /gain$/ });
  await field.fill(String(db));
  await field.press("Enter");
  await expect(field).toHaveValue(new RegExp(`^${db < 0 ? "[-−]" : "\\+?"}${Math.abs(db).toFixed(1).replace(".", "\\.")} dB$`));
}
