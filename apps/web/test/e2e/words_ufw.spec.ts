// WP UFW e2e (UXP A8, A10): the command palette finds a page by the words users type for it
// ("tweeter": Speakers), Help entries in it, and with no DSPi open the ways to one ("demo": Open
// Demo); the app menu holds the User Manual. Screenshots in both themes when SHOTS_DIR names a
// directory.

import { expect, test, type Page } from "@playwright/test";
import { rail } from "../nav";

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

async function shot(page: Page, name: string) {
  const dir = process.env.SHOTS_DIR;
  const project = test.info().project.name;
  if (!dir || project === "firefox") return;
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${dir}/ufw-${project}-${name}-${scheme}.png` });
  }
  await page.emulateMedia({ colorScheme: null });
}

const mod = process.platform === "darwin" ? "Meta" : "Control";
const palette = (page: Page) => page.getByRole("dialog", { name: "Command palette" });
const field = (page: Page) => palette(page).getByRole("combobox", { name: "Search settings, channels, commands" });
const first = (page: Page) => palette(page).getByRole("option").first();

test("UFW: the palette finds Speakers by \"tweeter\", the Crossover by \"high-pass\", the manual by \"refused\"", async ({ page }) => {
  const errors = errorsOf(page);
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(rail(session)).toBeVisible();
  await expect(session).toHaveAttribute("data-status", "open");
  await page.keyboard.press(`${mod}+k`);
  await expect(field(page)).toBeFocused();
  await field(page).fill("tweeter");
  await expect(first(page)).toContainText("Speakers");
  await shot(page, "palette-tweeter");
  await field(page).fill("high-pass");
  await expect(first(page)).toContainText("Crossover");
  await expect(first(page)).toContainText("high-pass");
  await field(page).fill("refused");
  await expect(first(page)).toContainText("Help: Why Was My Measurement Refused?");
  await field(page).fill("save to device");
  await expect(first(page)).toContainText("Save to Device…");
  await page.keyboard.press("Escape");
  await expect(palette(page)).toHaveCount(0);
  // The app menu: the manual
  await one.locator(".app-menu .menu > button").click();
  await expect(one.getByRole("menuitem", { name: "User Manual" })).toBeVisible();
  await page.keyboard.press("Escape");
  expect(errors).toEqual([]);
});

test("UFW: with no DSPi open the palette offers the ways to one; \"demo\" opens the demo", async ({ page }) => {
  const errors = errorsOf(page);
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await expect(one.getByRole("region", { name: "Venue A" })).toBeVisible();
  await page.keyboard.press(`${mod}+k`);
  await expect(field(page)).toBeFocused();
  await field(page).fill("demo");
  await expect(first(page)).toContainText("Open Demo");
  await shot(page, "palette-no-device");
  await page.keyboard.press("Enter");
  await expect(palette(page)).toHaveCount(0);
  const session = one.getByRole("article").first();
  await expect(session).toHaveAttribute("data-status", "open");
  expect(errors).toEqual([]);
});
