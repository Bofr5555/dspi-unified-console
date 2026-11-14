// WP P1DCFIX e2e: the review's fixups on the web (p1d-clients): the A/B key is a plain B, never
// while a text field has focus or a dialog is open; Simple has a Snapshots view with Recall and
// the A/B compare only; the rollback time stays Advanced's. On the core's demo device, in every
// project (Chromium, Firefox, Chromium at 375 px).

import { expect, test, type Locator, type Page } from "@playwright/test";

type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };

async function call<T>(page: Page, method: string, params: unknown = null): Promise<T> {
  return (await page.evaluate(([m, p]) => (globalThis as unknown as Clients).dspiClients[0]!.call(m as string, p), [method, params] as const)) as T;
}
const gain0 = async (page: Page) => (await call<{ state: { outputGainDb: number[] } }>(page, "state")).state.outputGainDb[0];

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(session.getByRole("region", { name: "Presets" })).toBeVisible();
  return session;
}

const tab = (session: Locator, name: string) => session.getByRole("tab", { name, exact: true });

async function setMode(session: Locator, page: Page, mode: "Simple" | "Advanced") {
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: mode }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await expect(session.getByRole("button", { name: `Mode: ${mode}` })).toBeVisible();
}

test("P1DCFIX Simple: Snapshots with Recall and A/B only; B toggles, not in a text field or a dialog", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  // Advanced: capture one (the rollback time is here, 30 s by default).
  await tab(session, "Snapshots").click();
  const screen = session.getByRole("region", { name: "Snapshots" });
  await expect(screen.getByLabel("Rollback time (s)")).toHaveValue("30");
  await call(page, "write", { path: "out.gain", indices: [0], value: { float: -5 } });
  await screen.getByLabel("Name").fill("Loud");
  await Promise.all([page.waitForEvent("download"), screen.getByRole("button", { name: "Capture" }).click()]);
  await call(page, "write", { path: "out.gain", indices: [0], value: { float: 0 } });

  // A plain B in a text field is the field's.
  await screen.getByLabel("Name").fill("");
  await screen.getByLabel("Name").focus();
  await page.keyboard.press("b");
  await expect(screen.getByLabel("Name")).toHaveValue("b");
  await screen.getByLabel("Name").fill("");

  // Simple: the list, Recall and Compare; no capture, open, download, remove, rollback choice or time; no Files.
  await setMode(session, page, "Simple");
  await expect(tab(session, "Files")).toHaveCount(0);
  await tab(session, "Snapshots").click();
  await expect(screen.getByRole("button", { name: "Capture" })).toHaveCount(0);
  await expect(screen.getByRole("button", { name: "Open Snapshot…" })).toHaveCount(0);
  await expect(screen.getByLabel("Rollback time (s)")).toHaveCount(0);
  const row = screen.getByRole("list", { name: "Held snapshots" }).getByRole("listitem", { name: "Loud" });
  await expect(row.getByRole("button", { name: "Recall", exact: true })).toBeVisible();
  await expect(row.getByRole("button", { name: "Compare" })).toBeVisible();
  for (const name of ["Recall with Rollback", "Download", /^Remove/]) await expect(row.getByRole("button", { name })).toHaveCount(0);

  // A/B in Simple: the bar and the plain B.
  await row.getByRole("button", { name: "Compare" }).click();
  const bar = session.getByRole("group", { name: "A/B compare" });
  await expect(bar).toContainText("Now A");
  await expect(bar.getByRole("button", { name: /switch/ })).toHaveAttribute("aria-keyshortcuts", "B");
  await page.locator("body").click({ position: { x: 1, y: 1 } });
  await page.keyboard.press("b");
  await expect(bar).toContainText("Now B");
  await expect.poll(() => gain0(page)).toBe(-5);
  // Not while a dialog is open: the Mode dialog keeps the key.
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await expect(dialog).toBeVisible();
  await page.keyboard.press("b");
  await page.waitForTimeout(300);
  await expect(bar).toContainText("Now B");
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await bar.getByRole("button", { name: "Keep A" }).click();
  await expect(bar).toHaveCount(0);
  await expect.poll(() => gain0(page)).toBe(0);

  // Recall in Simple (the demo: a plain recall).
  await row.getByRole("button", { name: "Recall", exact: true }).click();
  // Asked first only when the page shows unsaved changes.
  const asked = page.getByRole("dialog", { name: "Recall Snapshot" });
  const done = screen.getByRole("status", { name: "Result" }).filter({ hasText: "Recalled “Loud”" });
  await expect(asked.or(done)).toBeVisible();
  if (await asked.isVisible()) await asked.getByRole("button", { name: "Recall" }).click();
  await expect(done).toBeVisible();
  await expect.poll(() => gain0(page)).toBe(-5);
  expect(errors).toEqual([]);
});
