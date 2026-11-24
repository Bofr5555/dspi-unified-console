// WP WEBM1 unit tests: the Modern shell's sections (every screen of every mode has one place in the
// rail), the page a section opens, how it is arranged, one session per DSPi, where a session was
// left, the dock's sources (and, on the real core, the source's write).

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import type { DspState } from "../../src/core/protocol";
import { Session, type SessionState } from "../../src/ui/session";
import { SCREENS, lockedScreens, screensFor, withDeviceScreen, withHubScreen } from "../../src/ui/screens";
import { CONNECT, SECTIONS, loadNavigation, pageToOpen, sectionOf, sectionsFor, type SectionId } from "../../src/ui/shell/sections";
import { alreadyOpenText, holderOf, serialOf, transportName, type Holder } from "../../src/ui/shell/open-rule";
import { placeOf } from "../../src/ui/shell/places";
import { dockSources } from "../../src/ui/shell/Dock";
import { tabState } from "../../src/ui/shell/DeviceTabs";
import type { Source } from "../../src/fleet/types";

const ids = (x: { id: string }[]) => x.map((s) => s.id);

test("WEBM1 sections: every screen is a page of exactly one section, in the desktop's order", () => {
  const listed = SECTIONS.flatMap((s) => s.pages);
  assert.equal(new Set(listed).size, listed.length, "no page in two sections");
  for (const s of SCREENS) assert.ok(listed.includes(s.id), `${s.id} has a section`);
  for (const id of ["device", "hub", "hub-settings", "sender", CONNECT]) assert.ok(listed.includes(id), id);
  assert.deepEqual(ids([...SECTIONS]), ["overview", "equaliser", "routing", "effects", "measure", "analyse", "library", "system", "devices", "settings"]);
  assert.equal(sectionOf("generator"), "analyse");
  assert.equal(sectionOf("not a screen"), "settings", "a screen no section lists is still reachable");
});

test("WEBM1 sections: the pages of each mode; a mode's empty sections are not in the rail", () => {
  const of = (m: "simple" | "advanced" | "installer") => sectionsFor(withDeviceScreen(screensFor(m, false)));
  const simple = of("simple");
  assert.deepEqual(ids(simple), ["overview", "equaliser", "routing", "effects", "library", "devices", "settings"]);
  assert.deepEqual(ids(simple.find((s) => s.id === "library")!.pages), ["presets", "snapshots", "autoeq"], "snapshots: recall and A/B only; -- WP WEB7 -- Presets and AutoEQ in every mode, as Qt");
  assert.deepEqual(ids(simple.find((s) => s.id === "settings")!.pages), ["settings"]);
  const advanced = of("advanced");
  assert.deepEqual(ids(advanced), ids([...SECTIONS]));
  assert.deepEqual(ids(advanced.find((s) => s.id === "analyse")!.pages), ["analyser", "generator", "stats", "monitor"]);
  assert.deepEqual(ids(advanced.find((s) => s.id === "system")!.pages), ["system", "events"]);
  assert.deepEqual(ids(advanced.find((s) => s.id === "settings")!.pages), ["settings", "surfaces"]);
  assert.deepEqual(ids(of("installer").find((s) => s.id === "settings")!.pages), ["settings", "surfaces", "installer"]);
  // Nothing shown is lost: every screen of the mode is a page of some section, once.
  for (const m of ["simple", "advanced", "installer"] as const) {
    const shown = ids(withHubScreen(withDeviceScreen(screensFor(m, true)), m, true));
    const pages = sectionsFor(withHubScreen(withDeviceScreen(screensFor(m, true)), m, true)).flatMap((s) => ids(s.pages));
    assert.deepEqual([...pages].sort(), [...shown, CONNECT].sort(), m);
  }
  // Locked, the front page alone; with nothing open, the devices alone.
  assert.deepEqual(sectionsFor(lockedScreens()).map((s) => [s.id, ids(s.pages)]), [["overview", ["main"]], ["devices", [CONNECT]]]);
  assert.deepEqual(ids(sectionsFor([])), ["devices"]);
  assert.deepEqual(sectionsFor([]).map((s) => s.foot), [true]);
  // A host's own screen goes with what no section lists.
  assert.deepEqual(ids(sectionsFor([{ id: "x", label: "X" }, { id: "settings", label: "Settings" }]).find((s) => s.id === "settings")!.pages), ["settings", "x"]);
});

test("WEBM1 sections: a section opens at the page last open there, while that is shown", () => {
  const analyse = sectionsFor(screensFor("advanced", false)).find((s) => s.id === "analyse")!;
  const last = new Map<SectionId, string>();
  assert.equal(pageToOpen(analyse, last), "analyser");
  last.set("analyse", "stats");
  assert.equal(pageToOpen(analyse, last), "stats");
  last.set("analyse", "gone");
  assert.equal(pageToOpen(analyse, last), "analyser");
});

test("WEBM1 arrangement: a rail unless this viewer has tabs; never a throw", () => {
  const store = (v: string | null) => ({ getItem: () => v, setItem: () => undefined });
  assert.equal(loadNavigation(null), "rail");
  assert.equal(loadNavigation(store(null)), "rail");
  assert.equal(loadNavigation(store('{"navigation":"tabs"}')), "tabs");
  assert.equal(loadNavigation(store('{"navigation":"sideways"}')), "rail");
  assert.equal(loadNavigation(store("{not json")), "rail");
  assert.equal(loadNavigation(store('"tabs"')), "rail");
  assert.equal(loadNavigation({ getItem: () => { throw new Error("refused"); }, setItem: () => undefined }), "rail");
});

// ------------------------------------------------------------------ one session per DSPi

type Status = SessionState["status"];
const holder = (source: Source, opts: { status?: Status; serial?: string; deviceId?: string; name?: string } = {}): Holder => ({
  source,
  deviceId: opts.deviceId ?? "",
  installationName: "Studio",
  state: {
    status: opts.status ?? "open",
    snapshot: opts.serial === undefined ? null : ({ device: { serial: opts.serial, name: opts.name ?? "Rack" } } as SessionState["snapshot"]),
  },
});
const hub = (deviceId?: string): Source => ({ kind: "hub", url: "ws://h:1/dspi/v1", deviceId });

test("WEBM1 one session per DSPi: over USB, through a hub, chosen from a hub's list; a lost one gives it up", () => {
  const usb = holder({ kind: "usb", serial: "A" }, { serial: "A" });
  assert.equal(holderOf([usb], "A"), usb);
  assert.equal(holderOf([usb], "B"), null);
  assert.equal(holderOf([usb], ""), null, "no serial, no rule");
  assert.equal(holderOf([usb], "A", usb), null, "its own session is no other");
  // Before anything answered: what the entry names, what the hub's list chose.
  assert.ok(holderOf([holder({ kind: "usb", serial: "A" }, { status: "opening" })], "A"));
  assert.ok(holderOf([holder(hub("A"), { status: "pairing" })], "A"));
  assert.ok(holderOf([holder(hub(), { status: "choosing", deviceId: "A" })], "A"));
  assert.equal(holderOf([holder(hub(), { status: "choosing" })], "A"), null);
  // The unit as it names itself.
  assert.ok(holderOf([holder(hub(), { serial: "A" })], "A"));
  for (const status of ["failed", "closed"] as Status[]) assert.equal(holderOf([holder({ kind: "usb", serial: "A" }, { status, serial: "A" })], "A"), null, status);
  // The demos all have one serial, and a file may have a real unit's: neither holds a unit.
  assert.equal(holderOf([holder({ kind: "demo" }, { serial: "A" }), holder({ kind: "document", fileName: "x" }, { serial: "A" })], "A"), null);
  assert.equal(serialOf({ kind: "demo" }), "");
  assert.equal(serialOf({ kind: "document", fileName: "A" }), "");
  assert.equal(serialOf({ kind: "usb" }), "");
  assert.equal(serialOf(hub("A")), "A");
  assert.equal(alreadyOpenText(usb), "This DSPi is already open: Studio · Rack, USB");
  assert.deepEqual([hub(), { kind: "usb" }, { kind: "demo" }, { kind: "document", fileName: "" }].map((s) => transportName(s as Source)), ["Hub", "USB", "Demo", "File"]);
});

test("WEBM1 one session per DSPi: a hub's list does not open a DSPi the page refuses, and says why", async () => {
  const calls: string[] = [];
  const core = {
    subscribe: () => () => undefined,
    dispose: async () => undefined,
    call: async (m: string) => {
      calls.push(m);
      if (m === "linkConnect") return { needsPairing: false, hubName: "Rack hub", hubId: "h", tokenRefused: false };
      if (m === "linkDevices") return [{ serial: "A", name: "Rack", state: "online" }];
      if (m === "open") return { device: { serial: "A", name: "Rack" }, state: {} };
      throw new Error("not in this fake");
    },
  } as unknown as CoreClient;
  const s = new Session("i/hub~1", "i", "Studio", hub(), "control", core);
  let held = "This DSPi is already open: Studio · Rack, USB";
  s.refuseOpen = (serial) => (serial === "A" ? held : "");
  await s.open({ kind: "hub", url: "ws://h:1/dspi/v1", deviceId: "" });
  assert.equal(s.state.status, "choosing");
  await s.choose("A");
  assert.equal(s.state.status, "choosing");
  assert.equal(s.state.error, held);
  assert.equal(s.deviceId, "", "it holds nothing");
  assert.ok(!calls.includes("open"), "nothing was opened");
  held = "";
  await s.choose("A");
  assert.equal(s.state.status, "open");
  assert.equal(s.deviceId, "A");
});

test("WEBM1 a session's place is its own, and is kept while the session is", () => {
  const core = { subscribe: () => () => undefined } as unknown as CoreClient;
  const a = new Session("a", "i", "I", { kind: "demo" }, "control", core);
  const b = new Session("b", "i", "I", { kind: "demo" }, "control", core);
  placeOf(a).screen = "matrix";
  placeOf(a).last.set("library", "files");
  assert.equal(placeOf(a).screen, "matrix");
  assert.equal(placeOf(b).screen, null);
  assert.equal(placeOf(b).last.size, 0);
});

test("WEBM1 a tab's dot: an open session whose link is away is not shown as connected", () => {
  assert.equal(tabState({ status: "open", link: null }), "open");
  assert.equal(tabState({ status: "open", link: { phase: "bound" } }), "open");
  assert.equal(tabState({ status: "open", link: { phase: "reconnecting" } }), "away");
  assert.equal(tabState({ status: "pairing", link: { phase: "needs_pairing" } }), "pairing");
  assert.equal(tabState({ status: "failed", link: { phase: "bound" } }), "failed");
});

// ------------------------------------------------------------------ the dock

test("WEBM1 the dock's sources: what the device offers, and the one it plays when that is none of them", () => {
  const s = (inputSource: number) => ({ inputSource, spdifInputsEnabled: 1, i2sInputChannels: 0 }) as unknown as DspState;
  assert.deepEqual(dockSources(s(0)), [0, 1]);
  assert.deepEqual(dockSources(s(7)), [0, 1, 7], "an audio link's");
});

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

test("WEBM1 on the demo: the dock's source is written by its path and read back", { skip, timeout: 60_000 }, async () => {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const snap = await core.call("open", { kind: "demo" });
  assert.equal(snap.state.inputSource, 0);
  assert.deepEqual(dockSources(snap.state).slice(0, 2), [0, 1]);
  assert.deepEqual(await core.call("write", { path: "in.source", value: { choice: 1 } }), { choice: 1 });
  assert.equal((await core.call("state", undefined)).state.inputSource, 1);
});
