// WP WB6l e2e: the control-surface editor (CG20), the quick strip (M60) and the app
// shortcuts (M24), in Chromium, Firefox and Chromium at 375 px, on the core's demo device
// through the embedding page (Venue A controls it, Venue B views it only).
//
// - The editor lists the demo's controls by name; a card's row is the core's (its words
//   and options from `csDescribe`); a change is Pending until Apply, then live and unsaved
//   until Save; Revert asks and puts back what is stored; a new control works when added; a
//   group needs a member; a remote button is learned from the demo's remote and applied.
// - -- WP UWN -- (UXP C7) the Overview has no quick strip any more.
// - The keys: Ctrl+Shift+M opens the Matrix (Cmd on a Mac), Shift+? lists the shortcuts,
//   and neither acts while typing in a field.
// - View only: everything shows, no switch or Apply can be used, and the keys still move
//   between screens.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { appMenu, expectPage, openPage, openSetting } from "../nav";

type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[] };

function call<T>(page: Page, method: string, params: unknown = null, n = 0): Promise<T> {
  return page.evaluate(([n, method, params]) => (globalThis as unknown as G).dspiClients[n as number]!.call(method as string, params), [n, method, params] as const) as Promise<T>;
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

async function openSession(page: Page, venue = "Venue A") {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: venue }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await expect(session.locator(".volume-value")).toBeVisible();
  return session;
}

async function openSurfaces(session: Locator) {
  await openPage(session, "Control Surfaces");
  await expect(session.getByRole("group", { name: "Volume Knob", exact: true })).toBeVisible();
}

const card = (session: Locator, name: string) => session.getByRole("group", { name, exact: true });

async function noOverflow(page: Page) {
  const wide = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  expect(wide).toBeLessThanOrEqual(0);
}

type Row = { id: string; title: string; value?: number; options?: { value: number; text: string; disabled?: boolean; section?: string; owner?: string }[] };
type Snap = { bindings: Record<string, unknown>[]; status: { dirty: boolean }; ir: { protocol: number; code: number }[] };

test("WB6l CG20: a control's row is the core's, Pending until Apply, live until Save; Revert asks", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openSession(page);
  await openSurfaces(session);
  const knob = card(session, "Volume Knob");
  await expect(knob.locator(".cs-summary")).toContainText(/Rotary Encoder/);
  await expect(knob.locator(".cs-pill")).toHaveText("Active");
  await knob.getByRole("button", { name: "Expand Volume Knob" }).click();

  // The function row as the core describes it, and another function it offers.
  const snap = await call<Snap>(page, "csSnapshot");
  const d = await call<{ rows: Row[] }>(page, "csDescribe", { kind: "binding", slot: 0, draft: snap.bindings[0] });
  const noun = d.rows.find((r) => r.id === "noun")!;
  const other = noun.options!.find((o) => o.value !== noun.value && !o.disabled)!;
  const pick = knob.getByRole("combobox", { name: `Volume Knob ${noun.title}`, exact: true });
  await expect(pick).toHaveValue(String(noun.value));
  await pick.selectOption(String(other.value));
  await expect(knob.locator(".cs-pill")).toHaveText("Pending");
  expect((await call<Snap>(page, "csSnapshot")).bindings[0]!.noun).toBe(noun.value);
  await knob.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(knob.locator(".cs-pill")).toHaveText("Active");
  expect((await call<Snap>(page, "csSnapshot")).bindings[0]!.noun).toBe(other.value);
  // -- WP WEB7 -- Save and Revert on Qt's save bar, there only while something is unsaved
  const bar = session.getByRole("region", { name: "Control surfaces" });
  await expect(bar.locator(".cs-save-title")).toHaveText("Unsaved changes");

  // Revert asks, then the stored function is back.
  await bar.getByRole("button", { name: "Revert", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Revert Control Surfaces" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Revert" }).click();
  await expect(bar).toHaveCount(0);
  await expect(pick).toHaveValue(String(noun.value));

  // Apply again and Save: stored.
  await pick.selectOption(String(other.value));
  await knob.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(bar.locator(".cs-save-title")).toHaveText("Unsaved changes");
  await expect(bar.getByRole("button", { name: "Save", exact: true })).toHaveClass(/primary/);
  await bar.getByRole("button", { name: "Save", exact: true }).click();
  await expect(bar).toHaveCount(0);
  expect((await call<Snap>(page, "csSnapshot")).status.dirty).toBe(false);
  if (test.info().project.name === "narrow") await noOverflow(page);
  expect(errors).toEqual([]);
});

test("WB6l CG20: a new control works when added; a group needs a member; a macro runs", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openSession(page);
  await openSurfaces(session);
  // -- WP WEB7 -- Add Control is a menu of the types (Qt's CsAddButton), no select
  await expect(session.getByRole("combobox", { name: "Component type" })).toHaveCount(0);
  await session.getByRole("button", { name: "Add Control" }).click();
  await session.getByRole("menu", { name: "Add Control" }).getByRole("menuitem", { name: "Toggle Switch" }).click();
  const sw = card(session, "Toggle Switch");
  await expect(sw.locator(".cs-pill")).toHaveText("Active");
  await expect(sw.getByRole("button", { name: "Collapse Toggle Switch" })).toBeVisible();
  await sw.getByRole("button", { name: "Remove Toggle Switch" }).click();
  await expect(sw).toHaveCount(0);

  await openSetting(session, "Channel Groups"); // -- WP WEB7 -- a page of Settings › Control
  await expect(card(session, "Front Pair")).toBeVisible();
  await session.getByRole("button", { name: "Add Group" }).click();
  const g = card(session, "Group 2");
  await expect(g.getByText("Choose at least one member, then apply.")).toBeVisible();
  await expect(g.getByRole("button", { name: "Apply", exact: true })).toBeDisabled();
  await g.getByRole("checkbox").first().check();
  await g.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(g.getByRole("button", { name: "Apply", exact: true })).toBeDisabled();
  await expect(g.locator(".cs-message")).toHaveText("");

  await openSetting(session, "Macros");
  const night = card(session, "Night");
  await night.getByRole("button", { name: "Run Night" }).click();
  await expect(night.locator(".cs-summary")).toHaveText(/Running|step/);
  await expect(night.getByRole("button", { name: "Run Night" })).toBeVisible({ timeout: 10_000 });
  if (test.info().project.name === "narrow") await noOverflow(page);
  expect(errors).toEqual([]);
});

test("WB6l CG20: a remote button learned from the demo's remote, applied with the receiver", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openSession(page);
  await openSurfaces(session);
  const rx = card(session, "IR Remote");
  await rx.getByRole("button", { name: "Expand IR Remote" }).click();
  const before = (await call<Snap>(page, "csSnapshot")).ir.filter((c) => c.protocol !== 0).length;
  await rx.getByRole("button", { name: "Add Remote Button" }).click();
  const remotes = rx.getByRole("group", { name: "Remote buttons" });
  const fresh = remotes.getByRole("group", { name: `Remote button ${before + 1}` });
  await expect(fresh.getByText("Not learned")).toBeVisible();
  await fresh.getByRole("button", { name: "Learn" }).click();
  await expect(fresh.getByText("Point the remote at the receiver and press the button to learn.")).toBeVisible();
  expect(await call<boolean>(page, "csDemoLearn", { protocol: 1, code: 0x20df10ef })).toBe(true);
  await expect(fresh.getByText("NEC 0x20DF10EF")).toBeVisible();
  await expect(fresh.getByText("Learned a NEC code. Apply to keep it.")).toBeVisible();
  await rx.getByRole("button", { name: "Apply", exact: true }).last().click();
  await expect.poll(async () => (await call<Snap>(page, "csSnapshot")).ir.some((c) => c.code === 0x20df10ef)).toBe(true);
  expect(errors).toEqual([]);
});

// -- WP UWN -- (UXP C7) the Overview's nine icon-only quick-strip buttons are gone (Qt's Overview has none):
// their pages are the rail's and the palette's, the effects' switches are the Effects tile's chips and pages
test("UWN C7: the Overview has no quick strip; its effects are the Effects tile's", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openSession(page);
  await expect(session.getByRole("region", { name: "Effects" })).toBeVisible();
  await expect(session.getByRole("toolbar", { name: "Quick access" })).toHaveCount(0);
  await expect(session.getByRole("button", { name: "Matrix Mixer" })).toHaveCount(0);
  if (test.info().project.name === "narrow") await noOverflow(page);
  expect(errors).toEqual([]);
});

test("WB6l M24: Ctrl+Shift+M opens the Matrix, Shift+? lists the keys, never while typing", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openSession(page);
  const mod = (await page.evaluate(() => /mac/i.test(navigator.platform))) ? "Meta" : "Control";
  await page.keyboard.press(`${mod}+Shift+KeyM`);
  await expectPage(session, "Matrix");

  await page.keyboard.press("Shift+Slash");
  const list = page.getByRole("dialog", { name: "Keyboard Shortcuts" });
  await expect(list).toBeVisible();
  await expect(list.getByText("Matrix Mixer…")).toBeVisible();
  await expect(list.getByText("Save to Device…")).toBeVisible(); // -- WP UFW -- (UXP A10, D1)
  await expect(list.getByText("Recall Preset 1-9")).toBeVisible();
  // No effects screen on this build: its keys are not listed.
  await list.getByRole("button", { name: "Close" }).click();
  await expect(list).toHaveCount(0);

  // Typing in a field: the key is the field's.
  await openPage(session, "Control Surfaces");
  const name = session.getByRole("textbox", { name: "Name of Volume Knob" });
  await name.focus();
  await page.keyboard.press(`${mod}+Shift+KeyM`);
  await page.keyboard.press("Shift+Slash");
  await expectPage(session, "Control Surfaces");
  await expect(list).toHaveCount(0);
  // The app menu opens the same list (-- WP UWN -- the quick strip's button is gone: UXP C7).
  await appMenu(page.locator("#one"), "Keyboard Shortcuts");
  await expect(list).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(list).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("WB6l read only: the editor shows but writes nothing; the keys still move", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openSession(page, "Venue B");
  await openSurfaces(session);
  const knob = card(session, "Volume Knob");
  await knob.getByRole("button", { name: "Expand Volume Knob" }).click();
  await expect(knob.getByRole("textbox", { name: "Name of Volume Knob" })).toBeDisabled();
  for (const c of await knob.getByRole("combobox").all()) await expect(c).toBeDisabled();
  await expect(knob.getByRole("button", { name: "Apply", exact: true })).toBeDisabled();
  await expect(knob.getByRole("button", { name: "Remove Volume Knob" })).toBeDisabled();
  await expect(session.getByRole("button", { name: "Add Control" })).toBeDisabled();
  // -- WP WEB7 -- nothing unsaved: no save bar (Qt's shows only while something is pending)
  await expect(session.getByRole("region", { name: "Control surfaces" })).toHaveCount(0);
  // Past the page, the core refuses before anything is sent.
  const r = await call<{ ok: boolean }>(page, "csApply", { op: { op: "save" } });
  expect(r.ok).toBe(false);
  const mod = (await page.evaluate(() => /mac/i.test(navigator.platform))) ? "Meta" : "Control";
  await page.keyboard.press(`${mod}+Shift+KeyM`);
  await expectPage(session, "Matrix");
  expect(errors).toEqual([]);
});
