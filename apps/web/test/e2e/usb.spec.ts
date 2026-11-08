// WP WB4a e2e: USB through WebUSB on the real core, in headless Chromium, over a
// WebUSB mock (test/pages/usb.html, webusb-mock.ts). The core opens no demo:
// each session is a USB session in its own Worker, its control transfers and
// notification endpoint answered by the core's simulated DSPi behind the mock.
// WebUSB is Chromium's only (WEB-STUDY section 6), so Firefox is skipped here;
// console.spec.ts checks that Firefox says why.

import { expect, test, type Page } from "@playwright/test";

test.skip(({ browserName }) => browserName !== "chromium", "WebUSB is Chromium's");

type Mock = { usbMock: { pick(s: string | null): void; unplug(s: string): void; renameElsewhere(s: string, n: string): void } };
const mock = <K extends keyof Mock["usbMock"]>(page: Page, what: K, ...args: Parameters<Mock["usbMock"][K]>) =>
  page.evaluate(
    ([what, args]) => (((globalThis as unknown as Mock).usbMock[what] as (...a: unknown[]) => void)(...args)),
    [what, args] as const,
  );

async function noPageErrors(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

test("WB4a: two DSPis on USB at once: the picker, an allowed DSPi, write with read-back, a notification, unplug", async ({ page }) => {
  const errors = await noPageErrors(page);
  await page.goto("/test/usb.html?usb=MOCKA001,MOCKB002&permitted=MOCKB002");
  const picker = page.getByRole("navigation", { name: "DSPi devices" });
  const usbRow = picker.locator("li.source-usb");
  const rowA = picker.getByRole("listitem").filter({ hasText: "DSPi on USB · MOCKA001" });
  const rowB = picker.getByRole("listitem").filter({ hasText: "DSPi on USB · MOCKB002" });

  // Connect asks the browser's picker; the DSPi the page may already open is an entry of its own.
  await expect(usbRow.getByRole("button", { name: "Connect" })).toBeEnabled();
  await expect(rowB.getByRole("button", { name: "Connect" })).toBeEnabled();
  await expect(rowA).toHaveCount(0);

  // Closing the picker without a choice opens nothing.
  await mock(page, "pick", null);
  await usbRow.getByRole("button", { name: "Connect" }).click();
  await expect(page.getByText("Choose a DSPi to connect to.")).toBeVisible();

  // Choose A: its session opens over USB, and A is listed as allowed now.
  await mock(page, "pick", "MOCKA001");
  await usbRow.getByRole("button", { name: "Connect" }).click();
  const session = page.locator("article.session");
  await expect(session.locator(".device-meta")).toHaveText("RP2350 · firmware 1.1.6 · MOCKA001");
  await expect(rowA.getByRole("button")).toHaveText("Open");
  await expect(session.locator(".volume-value")).toHaveText("0.0 dB");

  // A write, read back from the device.
  // -- WP WB6a -- the main window's slider is the Console's taper: position 128 of 200 is -12 dB,
  // 50 is -30 dB, 98 is -18 dB
  const slider = session.getByRole("slider", { name: "Volume" });
  await slider.fill("128");
  await slider.dispatchEvent("change");
  await expect(session.locator(".volume-value")).toHaveText("-12.0 dB");

  // A change at the device arrives as a notification.
  await mock(page, "renameElsewhere", "MOCKA001", "Stage left");
  await expect(session.locator(".device-name")).toHaveText("Stage left");

  // B at the same time, in its own session.
  await rowB.getByRole("button", { name: "Connect" }).click();
  const tabs = page.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab");
  await expect(tabs).toHaveCount(2);
  await expect(session.locator(".device-meta")).toHaveText("RP2350 · firmware 1.1.6 · MOCKB002");
  await expect(session.locator(".volume-value")).toHaveText("0.0 dB");
  await slider.fill("50");
  await slider.dispatchEvent("change");
  await expect(session.locator(".volume-value")).toHaveText("-30.0 dB");
  await tabs.filter({ hasText: "Stage left" }).click();
  await expect(session.locator(".volume-value")).toHaveText("-12.0 dB");

  // Unplug A: its session ends in the core's words, it leaves the list, B goes on.
  await mock(page, "unplug", "MOCKA001");
  await expect(session.getByRole("alert")).toHaveText("Device not connected");
  await expect(rowA).toHaveCount(0);
  await tabs.filter({ hasText: "This console · DSPi" }).click();
  await expect(session.locator(".device-meta")).toContainText("MOCKB002");
  await slider.fill("98");
  await slider.dispatchEvent("change");
  await expect(session.locator(".volume-value")).toHaveText("-18.0 dB");

  await session.getByRole("button", { name: "Disconnect" }).click();
  await expect(tabs).toHaveCount(0);
  expect(errors).toEqual([]);
});
