// WP WEB7 unit tests: Settings' pages as Qt Modern lists them (screens/settings/pages.ts): the four
// parts in Qt's order with their pages, the list beside the page (the part's pages; a search over
// every part under each part's heading, by title and by the words on the page), the page a part
// opens, the ids from outside (the palette, Help's old id), and the palette's pages.

import { test } from "node:test";
import assert from "node:assert/strict";
import { PART_TITLES, SETTINGS_PAGE_DEFS, isHeading, listRows, pageIdOf, pageMatches, pageOfPart, partsShown, type ListRow, type SettingsPageId } from "../../src/screens/settings/pages";
import { SETTINGS_PAGES, settingsPagesFor } from "../../src/screens/settings/SettingsScreen";

const ALL: SettingsPageId[] = SETTINGS_PAGE_DEFS.map((p) => p.id);
const titles = (rows: ListRow[]) => rows.map((r) => (isHeading(r) ? `# ${r.heading}` : r.title));

test("WEB7 settings: Qt's parts in Qt's order, each page's badge tint and icon", () => {
  assert.deepEqual(partsShown(ALL).map((p) => PART_TITLES[p]), ["Application", "Display", "Device", "Control"]);
  assert.deepEqual(titles(listRows(ALL, "application", "")), ["About", "Advanced", "Appearance"], "Core Voltage is no row (Qt's Hidden)");
  assert.deepEqual(titles(listRows(ALL, "display", "")), ["Graphing", "Spectrum Analyser"]);
  assert.deepEqual(titles(listRows(ALL, "device", "")), ["Pins", "Global Parameters", "Speakers"]);
  assert.deepEqual(titles(listRows(ALL, "control", "")), ["Control Surfaces", "Channel Groups", "Macros", "Auxiliary Outputs"]);
  for (const p of SETTINGS_PAGE_DEFS) {
    assert.match(p.tint, /^#[0-9a-f]{6}$/, p.id);
    assert.ok(p.icon.length > 0, p.id);
  }
  assert.equal(SETTINGS_PAGE_DEFS.find((p) => p.id === "global")!.tint, "#807701", "Qt's settingsTints");
  // A part with nothing shown is not offered (Simple has no Control)
  assert.deepEqual(partsShown(ALL.filter((p) => !["surfaces", "groups", "macros", "aux"].includes(p))), ["application", "display", "device"]);
});

test("WEB7 settings: the search finds pages in every part by title and by the words on them", () => {
  assert.deepEqual(titles(listRows(ALL, "device", "latency")), ["# Device", "Global Parameters"]);
  assert.deepEqual(titles(listRows(ALL, "device", "  GRID ")), ["# Display", "Graphing"], "case and spaces ignored");
  assert.deepEqual(titles(listRows(ALL, "application", "gpio")), ["# Device", "Pins", "# Control", "Control Surfaces"]);
  assert.deepEqual(listRows(ALL, "application", "nothing like this"), []);
  assert.ok(pageMatches(SETTINGS_PAGE_DEFS.find((p) => p.id === "advanced")!, "core voltage"));
  // A page not shown is not found
  assert.deepEqual(titles(listRows(ALL.filter((p) => p !== "pins"), "device", "gpio")), ["# Control", "Control Surfaces"]);
});

test("WEB7 settings: a part opens the page last shown there, else its first; ids from outside", () => {
  assert.equal(pageOfPart("device", ALL, undefined), "pins");
  assert.equal(pageOfPart("device", ALL, "speakers"), "speakers");
  assert.equal(pageOfPart("device", ALL.filter((p) => p !== "pins"), undefined), "global");
  assert.equal(pageOfPart("device", ALL, "graphing"), "pins", "a page of another part is not kept");
  assert.equal(pageOfPart("control", ["about"], undefined), null);
  assert.equal(pageIdOf("help"), "about", "Help's old id");
  assert.equal(pageIdOf("appearance"), "appearance");
  assert.equal(pageIdOf("nonsense"), null);
  assert.equal(pageIdOf(undefined), null);
});

test("WEB7 settings: the palette's pages carry Qt's part names; no Pins, Core Voltage or Control (mode-dependent)", () => {
  assert.deepEqual(
    SETTINGS_PAGES.map((p) => `${p.group}/${p.title}`),
    ["Application/About", "Application/Advanced", "Application/Appearance", "Display/Graphing", "Display/Spectrum Analyser", "Device/Global Parameters", "Device/Speakers"],
  );
  for (const p of SETTINGS_PAGES) assert.equal(pageIdOf(p.id), p.id);
  // Control's pages where the mode has them: the palette still finds Control Surfaces in Advanced
  assert.deepEqual(settingsPagesFor("simple"), SETTINGS_PAGES);
  assert.deepEqual(
    settingsPagesFor("advanced").filter((p) => p.group === "Control").map((p) => p.id),
    ["surfaces", "groups", "macros", "aux"],
  );
});
