// WP UFW unit tests (UXP A8, A10): the palette finds an entry by the words users type for it
// (synonyms, Settings' terms), the manual's answers open sections that are there (Qt's list is the
// same), the ways to a DSPi with none open, and one word for Ctrl+S everywhere.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { MENUS, actionText } from "../../src/ui/menus/actions";
import type { PaletteEntry } from "../../src/ui/menus/board";
import { HELP, helpEntries, noDeviceEntries, paletteEntries } from "../../src/ui/palette/entries";
import { search } from "../../src/ui/palette/search";
import { SHORTCUTS } from "../../src/screens/shortcuts/table";

const run = () => undefined;
const page = (id: string, title: string, kind: PaletteEntry["kind"] = "page"): PaletteEntry => ({ id, kind, title, where: "", keys: "", enabled: true, run });
const repo = (path: string) => readFileSync(join(process.cwd(), "../..", path), "utf8");

function entries(extra: PaletteEntry[]) {
  return paletteEntries({ current: null, session: null, handlers: {}, others: [], extra });
}

test("UFW palette: found by the words users type (synonyms, a Settings page's terms)", () => {
  const all = entries([
    page("page:xover", "Crossover"),
    page("page:eq", "PEQ"),
    page("page:installer", "Installer"),
    page("settings:speakers", "Speakers", "settings"),
    page("settings:global", "Global Parameters", "settings"),
    ...helpEntries(run),
  ]);
  const first = (q: string) => search(all, q)[0]?.id;
  assert.equal(first("high-pass"), "page:xover");
  assert.equal(first("HPF"), "page:xover");
  assert.equal(first("protection"), "page:installer");
  assert.equal(first("tweeter floor"), "page:installer");
  assert.equal(first("tweeter"), "settings:speakers", "Settings' own term, the shorter title of equals");
  assert.equal(first("subwoofer"), "settings:speakers");
  assert.equal(first("rename"), "settings:global");
  assert.equal(first("refused"), "help:refused");
  assert.equal(first("glossary"), "help:words");
  assert.equal(search(all, "high-pass")[0]!.matched, "high-pass", "it says what found it");
});

test("UFW palette: with no DSPi, the ways to one", () => {
  let opened = "";
  const all = entries(noDeviceEntries({ openDemo: () => (opened = "demo"), devices: () => (opened = "devices"), addHub: () => (opened = "hub") }));
  const demo = search(all, "demo")[0]!;
  assert.equal(demo.id, "web.demo");
  assert.equal(demo.title, "Open Demo");
  demo.run();
  assert.equal(opened, "demo");
  assert.equal(search(all, "usb")[0]!.title, "Connect on USB");
  assert.equal(search(all, "hub")[0]!.title, "Add a Hub…");
  const none = noDeviceEntries({ openDemo: null, devices: run, addHub: null }).map((e) => e.id);
  assert.deepEqual(none, ["web.usb"], "only what the page has");
});

test("UFW Help: each answer opens a section that is there, and Qt lists the same", () => {
  const slug = (heading: string) =>
    heading
      .trim()
      .toLowerCase()
      .replace(/[^\p{L}\p{N} _-]/gu, "")
      .replace(/ /g, "-");
  const qt = repo("apps/desktop/qt/qml/modern/CommandPalette.qml");
  const urls: string[] = [];
  for (const h of helpEntries((u) => urls.push(u))) h.run();
  assert.equal(urls.length, HELP.length);
  for (const h of HELP) {
    const [file, anchor] = h.path.split("#");
    const text = repo(`docs/${file}`);
    if (anchor) {
      const anchors = text.split("\n").filter((l) => /^#{1,6} /.test(l)).map((l) => slug(l.replace(/^#+ /, "")));
      assert.ok(anchors.includes(anchor), `${h.path}: no such heading`);
    }
    assert.ok(qt.includes(`url: "${h.path}"`), `Qt's palette has ${h.path}`);
    assert.ok(qt.includes(`qsTr("${h.title}")`), `Qt's palette says "${h.title}"`);
  }
  assert.ok(urls.every((u) => u.startsWith("https://github.com/") && u.includes("/blob/main/docs/")));
});

test("UFW words: Ctrl+S is Save to Device in the menus, the palette and the shortcut list; Factory Reset is the device's", () => {
  assert.equal(actionText("device.commit"), "Save to Device…");
  assert.equal(SHORTCUTS.find((r) => r.id === "device.commit")!.text, actionText("device.commit"));
  assert.ok(!MENUS.save!.flat().includes("device.factoryReset"));
  assert.ok(MENUS.device!.flat().includes("device.factoryReset"));
  // Its old name still finds it
  const all = entries([{ ...page("device.commit", "Save to Device…", "action") }]);
  assert.equal(search(all, "commit parameters")[0]?.id, "device.commit");
});
