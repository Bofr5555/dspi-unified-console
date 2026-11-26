// WP UB13 e2e (UXP B13, D11): Settings › Appearance's "Outline controls" and control size. A mouse desktop
// starts with neither (the owner's look); the outline draws Qt's 3:1 line round fields, menus and buttons;
// Comfortable makes the controls 32 px and the PEQ table's "plays" disc a 44 px target; a touch screen
// (a coarse pointer) gets Comfortable by itself. Screenshots in both themes when SHOTS_DIR names a directory.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage, openSetting } from "../nav";

const OUTLINE = "rgb(130, 130, 130)"; // --control-outline, access.css

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
    await page.screenshot({ path: `${dir}/ub13-${project}-${name}-${scheme}.png` });
  }
  await page.emulateMedia({ colorScheme: null });
}

async function noSidewaysScroll(page: Page) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(0);
}

/** A demo device's PEQ page, OUT1's bands: Venue A's on the embedding page, or (`phone`) the Demo DSPi on
 *  the console's own page, whose viewport is the device's width. */
async function openEq(page: Page, phone = false): Promise<{ one: Locator; session: Locator; list: Locator }> {
  await page.goto(phone ? "/" : "/test/embed.html?one");
  const one = phone ? page.locator("body") : page.locator("#one");
  if (phone) await page.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  else await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(session).toHaveAttribute("data-status", "open");
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT1" }).click();
  const list = session.getByRole("region", { name: /^Bands of OUT1/ });
  await expect(list).toBeVisible();
  return { one, session, list };
}

/** Band 1's disc is a 44 px target, clear of its number; then centred for the picture. */
async function bigDisc(list: Locator) {
  const disc = list.getByRole("checkbox", { name: "Band 1 plays" });
  const box = await disc.boundingBox();
  const number = await list.getByRole("button", { name: "Band 1", exact: true }).boundingBox();
  expect(box?.width).toBeGreaterThanOrEqual(43.5);
  expect(box?.height).toBeGreaterThanOrEqual(43.5);
  expect(box!.x + box!.width).toBeLessThanOrEqual(number!.x + 0.5);
  await disc.evaluate((e) => e.scrollIntoView({ block: "center" }));
}

const borderColour = (l: Locator) => l.evaluate((e) => getComputedStyle(e).borderTopColor);
const height = async (l: Locator) => (await l.boundingBox())?.height ?? 0;

test("UB13: a mouse desktop has neither; Outline controls draws the 3:1 line, Comfortable the 32 px controls; both kept", async ({ page }) => {
  const errors = errorsOf(page);
  const { one, session, list } = await openEq(page);
  const root = one.locator(".dspi-web");
  const type = list.getByRole("combobox").first();
  const source = session.locator(".dock-source"); // a menu on the dock, the control tone
  const button = session.getByRole("button", { name: "Enable All", exact: true });
  await expect(root).not.toHaveAttribute("data-outline", /./);
  await expect(root).not.toHaveAttribute("data-density", /./);
  expect(await borderColour(source)).toBe("rgba(0, 0, 0, 0)");
  expect(await borderColour(button)).toBe("rgba(0, 0, 0, 0)");
  const sourceHeight = await height(source);
  const disc = list.getByRole("checkbox", { name: "Band 1 plays" });
  expect((await disc.boundingBox())?.width).toBeLessThan(20);
  await disc.evaluate((e) => e.scrollIntoView({ block: "center" }));
  await shot(page, "peq-off");

  await openPage(session, "Settings");
  await openSetting(session, "Appearance");
  const card = session.getByRole("region", { name: "Appearance" });
  await card.getByRole("group", { name: "Outline controls" }).getByRole("radio", { name: "On", exact: true }).check();
  await expect(root).toHaveAttribute("data-outline", "on");
  await card.getByRole("group", { name: "Control size" }).getByRole("radio", { name: "Comfortable", exact: true }).check();
  await expect(root).toHaveAttribute("data-density", "comfortable");
  await noSidewaysScroll(page);
  await shot(page, "appearance-both");

  await openPage(session, "EQ");
  expect(await borderColour(source)).toBe(OUTLINE);
  expect(await borderColour(button)).toBe(OUTLINE);
  expect(await type.evaluate((e) => getComputedStyle(e).boxShadow)).toContain(OUTLINE); // the drawn combo: inside
  expect(await height(source)).toBeGreaterThanOrEqual(Math.max(31.5, sourceHeight));
  await bigDisc(list);
  await noSidewaysScroll(page);
  await shot(page, "peq-both");

  // Kept in this browser
  await page.reload();
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  await expect(root).toHaveAttribute("data-outline", "on");
  await expect(root).toHaveAttribute("data-density", "comfortable");
  // Standard and off: the owner's look again
  await openPage(session, "Settings");
  await openSetting(session, "Appearance");
  await card.getByRole("group", { name: "Outline controls" }).getByRole("radio", { name: "Off", exact: true }).check();
  await card.getByRole("group", { name: "Control size" }).getByRole("radio", { name: "Standard", exact: true }).check();
  await expect(root).not.toHaveAttribute("data-outline", /./);
  await expect(root).not.toHaveAttribute("data-density", /./);
  expect(errors).toEqual([]);
});

test.describe("UB13 on a touch screen", () => {
  // A phone: Chromium's touch emulation makes its pointer coarse (Firefox has no isMobile)
  test.skip(({ browserName }) => browserName !== "chromium", "touch emulation is Chromium's");
  test.use({ hasTouch: true, isMobile: true, viewport: { width: 390, height: 844 } });

  test("UB13: Automatic is Comfortable on a touch screen; Standard is not", async ({ page }) => {
    const errors = errorsOf(page);
    const { one, session, list } = await openEq(page, true);
    const root = one.locator(".dspi-web");
    expect(await page.evaluate(() => matchMedia("(pointer: coarse)").matches)).toBe(true);
    await expect(root).toHaveAttribute("data-density", "comfortable");
    await expect(root).not.toHaveAttribute("data-outline", /./);
    await bigDisc(list);
    await noSidewaysScroll(page);
    await shot(page, "touch-peq");
    await openPage(session, "Settings");
    await openSetting(session, "Appearance");
    const card = session.getByRole("region", { name: "Appearance" });
    await card.getByRole("group", { name: "Control size" }).getByRole("radio", { name: "Standard", exact: true }).check();
    await expect(root).not.toHaveAttribute("data-density", /./);
    await openPage(session, "EQ");
    const disc = list.getByRole("checkbox", { name: "Band 1 plays" });
    expect((await disc.boundingBox())?.width).toBeLessThan(20);
    await disc.evaluate((e) => e.scrollIntoView({ block: "center" }));
    await shot(page, "touch-peq-standard"); // as before this package
    expect(errors).toEqual([]);
  });
});
