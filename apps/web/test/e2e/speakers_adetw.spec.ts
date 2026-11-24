// WP ADETW e2e: an auto-detected speaker layout on the core's demo device, in every project
// (Chromium, Firefox, Chromium at 375 px without sideways scroll). The layout a detection leaves
// (set through the core, as an agent's console would keep it) shows "Detected by AI, not
// confirmed" on the main screen and Settings › Speakers, each row's pill, the detection's summary
// and details; a corrected role turns its pill to "you"; Confirm makes the layout the person's and
// the main screen's line goes.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };

async function call<T>(page: Page, method: string, params: unknown = null): Promise<T> {
  return (await page.evaluate(([m, p]) => (globalThis as unknown as Clients).dspiClients[0]!.call(m as string, p), [method, params] as const)) as T;
}

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

async function speakersPage(session: Locator): Promise<Locator> {
  await openPage(session, "Settings");
  await session.getByRole("navigation", { name: "Settings pages" }).getByRole("button", { name: "Speakers" }).click();
  const page = session.getByRole("region", { name: "Speakers" });
  await expect(page.getByRole("list", { name: "Speaker layout" })).toBeVisible();
  return page;
}

async function noSidewaysScroll(page: Page) {
  if (test.info().project.name !== "narrow") return;
  const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(over).toBeLessThanOrEqual(0);
}

const found = (output: number, cls: string, confidence: string, role: string | null, note: string) => ({
  output,
  class: cls,
  confidence,
  role,
  floorHz: 1000,
  heard: cls === "nothing" ? "never" : "top",
  levelDbfs: -40,
  topFallDb: cls === "woofer" ? 21 : 1,
  lowTurnDb: null,
  fLowHz: cls === "woofer" ? 45 : null,
  lowestHz: cls === "woofer" ? 20 : 1000,
  stoppedAtHz: null,
  note,
  probedMs: Date.now(),
});

test("ADETW: an auto-detected layout shown on the main screen and Settings › Speakers; corrected and confirmed", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  const snap = await call<{ state: { numOutputChannels: number } }>(page, "state");
  const outputs = snap.state.numOutputChannels;
  await call(page, "speakerLayoutSet", {
    layout: {
      serial: "",
      outputs: [
        { output: 0, role: "low", speaker: "L", group: 0, source: "detected" },
        { output: 1, role: "high", speaker: "L", group: 0, source: "detected" },
      ],
      confirmedByUser: false,
      updatedMs: 0,
      notes: [],
      detected: {
        atMs: Date.now(),
        by: "Claude Code",
        placement: "listeningPosition",
        floorHz: 1000,
        microphone: "",
        outputs: [found(0, "woofer", "high", "low", "detect.woofer"), found(1, "tweeter", "medium", "high", "detect.tweeter_unsure"), found(2, "nothing", "high", null, "detect.nothing")],
        stopped: false,
        summary: "1 tweeter?, 1 woofer; nothing heard on Out 3.",
        deviceOutputs: outputs,
      },
    },
  });

  // The main screen says it (FUNC2c: an unconfirmed setup was invisible there).
  await openPage(session, "Settings");
  await openPage(session, "Main");
  const note = session.getByRole("status", { name: "Speaker layout", exact: true });
  await expect(note).toHaveText("Your speaker layout was detected by AI and is not confirmed. Check it in Settings › Speakers.");
  await noSidewaysScroll(page);

  const sp = await speakersPage(session);
  const banner = sp.getByRole("status", { name: "Speaker layout", exact: true });
  await expect(banner).toContainText("Detected by AI, not confirmed: Claude Code probed this DSPi's outputs today at");
  await expect(banner).toContainText("(microphone at the listening position)");
  await expect(sp.getByRole("status", { name: "Speaker layout state" })).toContainText(/^Detected by AI, not confirmed, today at /);
  await expect(sp.getByLabel(/^OUT1 .*: detected$/)).toBeVisible();
  await expect(sp.getByLabel(/^OUT2 .*: detected\?$/)).toBeVisible();
  // The demo's Out 3 is off (a real detection probes only outputs that are on): shown on asking.
  const more = sp.getByRole("button", { name: /^Show the outputs that are off/ });
  if (await more.count()) await more.click();
  await expect(sp.getByLabel(/^OUT3 .*: nothing heard$/)).toBeVisible();
  await expect(sp.getByRole("combobox", { name: /^Role of OUT3 / })).toBeDisabled();
  const details = sp.getByText("1 tweeter?, 1 woofer; nothing heard on Out 3.");
  await expect(details).toBeVisible();
  await details.click();
  const table = sp.getByRole("table", { name: "Speaker detection details" });
  await expect(table).toContainText("Woofer");
  await expect(table).toContainText("Tweeter?");
  await expect(table).toContainText("Nothing heard");
  await noSidewaysScroll(page);
  if (test.info().project.name === "narrow") {
    await sp.screenshot({ path: test.info().outputPath("speakers-detected-light.png") });
    await page.emulateMedia({ colorScheme: "dark" });
    await sp.screenshot({ path: test.info().outputPath("speakers-detected-dark.png") });
    await page.emulateMedia({ colorScheme: "light" });
  }

  // A corrected role is the person's; Confirm makes the layout theirs.
  await sp.getByRole("combobox", { name: /^Role of OUT2 / }).selectOption("mid");
  await expect(sp.getByLabel(/^OUT2 .*: you$/)).toBeVisible();
  await expect(sp.getByRole("status", { name: "Speaker layout state" })).toHaveText("Changed: not kept until you confirm.");
  await sp.getByRole("button", { name: "Confirm" }).click();
  await expect(sp.getByRole("status", { name: "Speaker layout saved" })).toContainText("Confirmed");
  await expect(banner).toHaveCount(0);
  await expect(sp.getByRole("status", { name: "Speaker layout state" })).toContainText(/^Confirmed today at /);
  const stored = await call<{ confirmedByUser: boolean; outputs: { output: number; role: string; source: string }[] }>(page, "speakerLayout");
  expect(stored.confirmedByUser).toBe(true);
  expect(stored.outputs.map((o) => [o.output, o.role, o.source])).toEqual([
    [0, "low", "detected"],
    [1, "mid", "user"],
  ]);
  await openPage(session, "Main");
  await expect(session.getByRole("region", { name: "Presets" })).toBeVisible();
  await expect(note).toHaveCount(0);
  expect(errors).toEqual([]);
});
