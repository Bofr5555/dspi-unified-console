// WP WB5 e2e: the standalone page and the embedding page, in Chromium and
// Firefox (and Chromium at 375 px wide). The core is the stub until WB3; these
// tests are written against the contract, so they keep passing on the real core.

import { expect, test, type Page } from "@playwright/test";

const linkWords = (await import("../../../../ci/generated/web/link-words.json", { with: { type: "json" } })).default as {
  link: { key: string; text: string; action: string | null }[];
};
const words = (key: string) => linkWords.link.find((w) => w.key === key)!;

async function noPageErrors(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

test("standalone: the demo connects, a volume write reads back, meters move", async ({ page }) => {
  const errors = await noPageErrors(page);
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "DSPi Console" })).toBeVisible();
  const picker = page.getByRole("navigation", { name: "DSPi devices" });
  await picker.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();

  const session = page.getByRole("article", { name: "DSPi demo" });
  await expect(session.locator(".device-meta")).toContainText("firmware 1.1.6");
  await expect(session.locator(".volume-value")).toHaveText("-20.0 dB");

  const slider = session.getByRole("slider", { name: "Volume" });
  await slider.fill("-12.5");
  await slider.dispatchEvent("change");
  await expect(session.locator(".volume-value")).toHaveText("-12.5 dB");

  const meter = session.getByRole("meter", { name: "USB L" });
  const first = await meter.getAttribute("aria-valuenow");
  await expect.poll(async () => meter.getAttribute("aria-valuenow"), { timeout: 5000 }).not.toBe(first);

  await session.getByRole("button", { name: "Disconnect" }).click();
  await expect(page.getByText("Choose a DSPi to connect to.")).toBeVisible();
  expect(errors).toEqual([]);
});

test("standalone: USB and hubs say why they are not offered, in words without a device noun", async ({ page }) => {
  await page.goto("/");
  const usb = page.getByRole("listitem").filter({ hasText: "DSPi on USB" });
  await expect(usb.getByRole("button")).toHaveCount(0);
  const reason = (await usb.locator(".unavailable").textContent()) ?? "";
  expect(reason.length).toBeGreaterThan(10);
  expect(reason).not.toMatch(/\b(computer|phone|tablet|laptop|desktop)\b/i);
});

test("standalone: an added hub is listed and kept across a reload", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Add a hub").fill("studio.local:11915");
  await page.getByRole("button", { name: "Add" }).click();
  await expect(page.getByText("Hub studio.local:11915")).toBeVisible();
  await page.reload();
  await expect(page.getByText("Hub studio.local:11915")).toBeVisible();
});

test("standalone ?readonly: the volume control is off and says why", async ({ page }) => {
  await page.goto("/?readonly");
  await page.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article", { name: "DSPi demo" });
  await expect(session.getByText("Read only", { exact: true })).toBeVisible();
  await expect(session.getByRole("slider", { name: "Volume" })).toBeDisabled();
  await expect(session.locator(".read-only-reason")).toHaveText("You can view this DSPi but not change it");
});

test("About: Weeb Labs' links are marked as theirs, from assets/about.json", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "About" }).click();
  const about = page.getByRole("dialog", { name: "About" });
  await expect(about.getByRole("heading", { name: "Weeb Labs: links and support" })).toBeVisible();
  await expect(about.getByText("This console is made by Bofr5555, not Weeb Labs.", { exact: false })).toBeVisible();
  await expect(about.getByRole("link", { name: "Discord" })).toHaveAttribute("rel", /noopener/);
  await about.getByRole("button", { name: "Close" }).click();
  await expect(about).toHaveCount(0);
});

test("embedded: two consoles on one page, each with its user, roles and sessions", async ({ page }) => {
  const errors = await noPageErrors(page);
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  const two = page.locator("#two");
  await expect(one.getByText("Operator One")).toBeVisible();
  await expect(two.getByText("Operator Two")).toBeVisible();
  // The server manages the list: no add-hub form.
  await expect(page.getByLabel("Add a hub")).toHaveCount(0);

  // Console one: Venue A with control, Venue B view only, both open at once.
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  await expect(one.getByRole("article", { name: "DSPi demo" }).getByRole("slider", { name: "Volume" })).toBeEnabled();
  await one.getByRole("region", { name: "Venue B" }).getByRole("button", { name: "Connect" }).click();
  await expect(one.getByRole("tab")).toHaveCount(2);
  await expect(one.getByRole("tab", { name: "Venue B · DSPi demo" })).toHaveAttribute("aria-selected", "true");
  const venueB = one.getByRole("article", { name: "DSPi demo" });
  await expect(venueB.getByRole("slider", { name: "Volume" })).toBeDisabled();
  await expect(venueB.locator(".read-only-reason")).toHaveText("You can view this DSPi but not change it");

  // Console two's session is its own: a write there does not show in console one.
  await two.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const twoSession = two.getByRole("article", { name: "DSPi demo" });
  const slider = twoSession.getByRole("slider", { name: "Volume" });
  await slider.fill("-6");
  await slider.dispatchEvent("change");
  await expect(twoSession.locator(".volume-value")).toHaveText("-6.0 dB");
  await one.getByRole("tab").first().click();
  await expect(one.getByRole("article", { name: "DSPi demo" }).locator(".volume-value")).toHaveText("-20.0 dB");

  // Our styles are scoped: a button of the host page keeps the host's style.
  const hostButtonColour = await page.evaluate(() => {
    const b = document.createElement("button");
    document.body.appendChild(b);
    const c = getComputedStyle(b).color;
    b.remove();
    return c;
  });
  expect(hostButtonColour).toBe("rgb(255, 0, 0)");
  expect(errors).toEqual([]);
});

test("lost link: CG10's wording and its way out", async ({ page }) => {
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi demo" });
  await expect(session.locator(".volume-value")).toBeVisible();

  await page.evaluate(() =>
    (globalThis as unknown as { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] }).dspiClients[0]!.call(
      "simulate",
      { what: "link-lost" },
    ),
  );
  const reconnecting = words("reconnecting");
  const banner = session.locator(".banner");
  await expect(banner).toContainText(reconnecting.text);
  await expect(banner.getByRole("button", { name: reconnecting.action! })).toBeVisible();

  await page.evaluate(() =>
    (globalThis as unknown as { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] }).dspiClients[0]!.call(
      "simulate",
      { what: "link-closed" },
    ),
  );
  await expect(banner).toContainText(words("closed").text);
  await expect(banner.getByRole("button")).toHaveCount(0);
});

test("narrow width: no sideways scrolling", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  await expect(page.getByRole("article", { name: "DSPi demo" })).toBeVisible();
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
});
