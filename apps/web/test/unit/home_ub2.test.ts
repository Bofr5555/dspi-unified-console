// WP UB2 unit tests (UXP B2, D4): Home, the remote, is Simple's first page and the locked user's
// only one; its Recall list, its source rule, its health line and a room's level; the web app
// manifest that makes Add to Home Screen an app.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { lockedScreens, screensFor } from "../../src/ui/screens";
import { sectionsFor } from "../../src/ui/shell/sections";
import { healthLine, recallRows, roomLevel, sourceChoosable } from "../../src/screens/home/model";

const ids = (list: readonly { id: string }[]) => list.map((x) => x.id);

test("UB2 B2: Simple opens on Home, the locked user has Home alone, Advanced and Installer have no Home", () => {
  assert.equal(screensFor("simple", false)[0]!.id, "home");
  assert.equal(screensFor("simple", true)[0]!.id, "home");
  assert.ok(!ids(screensFor("advanced", false)).includes("home"));
  assert.ok(!ids(screensFor("advanced", true)).includes("home"));
  assert.ok(!ids(screensFor("installer", false)).includes("home"));
  assert.deepEqual(ids(lockedScreens()), ["home"]);
  // Simple hides nothing permanently (UI-MODES section 2): the Dashboard is still a tab away
  assert.ok(ids(screensFor("simple", false)).includes("main"));
  // Home is the Overview section's first page
  const overview = sectionsFor(screensFor("simple", false)).find((s) => s.id === "overview")!;
  assert.deepEqual(ids(overview.pages).slice(0, 2), ["home", "main"]);
});

test("UB2 B2: the Recall list holds the stored presets and the one that plays, by name, never an empty slot", () => {
  const s = { activePresetSlot: 2, presetNames: ["Evening", "", "", "Night mode", "  "], presetOccupied: 0b11011 };
  assert.deepEqual(recallRows(s), [
    { slot: 0, text: "Evening", active: false },
    { slot: 1, text: "Preset 2", active: false },
    { slot: 2, text: "Empty", active: true },
    { slot: 3, text: "Night mode", active: false },
    { slot: 4, text: "Preset 5", active: false },
  ]);
  // A slot the device has not said (255): no row plays
  assert.ok(recallRows({ ...s, activePresetSlot: 255 }).every((r) => !r.active));
  assert.equal(recallRows({ ...s, activePresetSlot: 255 }).length, 4);
});

test("UB2 B2: the locked user sees the source as words, never a control the lock would refuse", () => {
  assert.equal(sourceChoosable(false, 3), true);
  assert.equal(sourceChoosable(true, 3), false);
  assert.equal(sourceChoosable(false, 1), false, "one source: nothing to choose");
});

test("UB2 B2: one health line and a room's level in the dock's words", () => {
  assert.equal(healthLine({ text: "OK", tone: "ok" }), "No problems");
  assert.equal(healthLine({ text: "2 problems", tone: "bad" }), "2 problems");
  assert.equal(healthLine({ text: "1 acknowledged", tone: "warn" }), "1 acknowledged");
  assert.equal(roomLevel([-20, -20]), "-20.0 dB");
  assert.equal(roomLevel([-24, null, -18]), "-24.0 to -18.0 dB");
  assert.equal(roomLevel([null, undefined]), "–");
});

test("UB2 B2: the page names a web app manifest with icons and the theme's greys", () => {
  const root = join(process.cwd());
  const html = readFileSync(join(root, "index.html"), "utf8");
  assert.match(html, /<link rel="manifest" href="manifest.json"/);
  assert.match(html, /<meta name="theme-color" content="#202020" media="\(prefers-color-scheme: dark\)"/);
  const m = JSON.parse(readFileSync(join(root, "manifest.json"), "utf8")) as { display: string; start_url: string; icons: { src: string; sizes: string }[] };
  assert.equal(m.display, "standalone");
  assert.equal(m.start_url, "./");
  for (const want of ["192x192", "512x512"]) {
    const icon = m.icons.find((i) => i.sizes === want);
    assert.ok(icon, want);
    assert.ok(readFileSync(join(root, "icons", icon.src)).length > 0, icon.src);
  }
});
