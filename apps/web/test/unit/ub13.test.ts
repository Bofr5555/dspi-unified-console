// WP UB13 unit tests (UXP B13, D11): Settings › Appearance's "Outline controls" and control size, as kept,
// what the console carries for them, and the outline's contrast.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { CHOICES, DEFAULT_ARRANGEMENT, comfortableFor, loadArrangement, saveArrangement, withChoice } from "../../src/ui/arrangement/arrangement";
import { accessAttributes } from "../../src/ui/arrangement/access";

const memory = () => {
  const store = new Map<string, string>();
  return { store, kv: { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) } };
};

test("UB13: both off on a mouse desktop: no outline, Automatic", () => {
  assert.equal(DEFAULT_ARRANGEMENT.outline, "off");
  assert.equal(DEFAULT_ARRANGEMENT.density, "automatic");
  assert.deepEqual(accessAttributes(DEFAULT_ARRANGEMENT, false), { "data-outline": undefined, "data-density": undefined });
});

test("UB13: Automatic is Comfortable with a coarse pointer (a touch screen), Standard is never", () => {
  assert.equal(comfortableFor("automatic", true), true);
  assert.equal(comfortableFor("automatic", false), false);
  assert.equal(comfortableFor("comfortable", false), true);
  assert.equal(comfortableFor("standard", true), false);
  assert.equal(accessAttributes(DEFAULT_ARRANGEMENT, true)["data-density"], "comfortable");
  assert.equal(accessAttributes({ ...DEFAULT_ARRANGEMENT, density: "standard" }, true)["data-density"], undefined);
  assert.equal(accessAttributes({ ...DEFAULT_ARRANGEMENT, outline: "on" }, false)["data-outline"], "on");
});

test("UB13: the choices are kept in this browser; what is not one of them is not", () => {
  const { store, kv } = memory();
  const a = withChoice(withChoice(DEFAULT_ARRANGEMENT, "outline", "on")!, "density", "comfortable")!;
  assert.equal(withChoice(a, "outline", "thick"), null);
  assert.equal(withChoice(a, "density", "large"), null);
  saveArrangement(kv, a);
  assert.deepEqual(loadArrangement(kv), { ...DEFAULT_ARRANGEMENT, outline: "on", density: "comfortable" });
  store.set("dspi-web.arrangement", JSON.stringify({ outline: "yes", density: 3 }));
  assert.deepEqual(loadArrangement(kv), DEFAULT_ARRANGEMENT, "unreadable values: the defaults");
});

test("UB13: Settings › Appearance offers them in the desktop app's words", () => {
  const outline = CHOICES.find((c) => c.key === "outline");
  const density = CHOICES.find((c) => c.key === "density");
  assert.equal(outline?.label, "Outline controls");
  assert.deepEqual(outline?.values.map((v) => v.text), ["Off", "On"]);
  assert.equal(density?.label, "Control size");
  assert.deepEqual(density?.values.map((v) => v.text), ["Automatic", "Standard", "Comfortable"]);
});

// WCAG 2 relative luminance and contrast
const lum = (hex: string) => {
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((x) => (x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0]! + 0.7152 * c[1]! + 0.0722 * c[2]!;
};
const ratio = (a: string, b: string) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x! + 0.05) / (y! + 0.05);
};

test("UB13: the outline reaches 3:1 on every surface and on the control tone, light and dark (WCAG 1.4.11)", () => {
  const src = (f: string) => readFileSync(join(process.cwd(), f), "utf8");
  const outline = /--control-outline:\s*(#[0-9a-f]{6})/i.exec(src("src/ui/arrangement/access.css"))?.[1];
  assert.ok(outline, "access.css names --control-outline");
  const css = src("src/dspi-web.css");
  // Each token's first value is the light scheme's, its second the dark's
  const values = (token: string) => [...css.matchAll(new RegExp(`${token}:\\s*(#[0-9a-f]{6})`, "gi"))].map((m) => m[1]!);
  for (const token of ["--bg", "--panel", "--chrome", "--control"]) {
    const [light, dark] = values(token);
    assert.ok(light && dark, `${token} in both schemes`);
    for (const surface of [light, dark]) assert.ok(ratio(outline, surface) >= 3, `${outline} on ${token} ${surface}: ${ratio(outline, surface).toFixed(2)}`);
  }
});
