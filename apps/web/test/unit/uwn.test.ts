// WP UWN unit tests (UXP A15, A16, A17, C7): the web's names, recovery and crossover words.

import { test } from "node:test";
import assert from "node:assert/strict";
import { bandTypeName } from "../../src/screens/eq/BandList";
import { T, setTypeNames } from "../../src/screens/eq/model";
import { saveQuestion } from "../../src/ui/screens";
import { connectionWord } from "../../src/screens/system/StatusRail";
import { tabWords } from "../../src/ui/shell/DeviceTabs";
import { saveName } from "../../src/ui/shell/SessionBar";
import { chooserRows, detailAdds } from "../../src/ui/components";
import { identifyFailure } from "../../src/screens/hub/HubScreen";
import { isOtherHub } from "../../src/ui/session";

test("UWN A17: the band table says the inspector's shapes and slopes, not the core's Peaking / Low Cut (12dB)", () => {
  setTypeNames([
    { code: 0, name: "Off" },
    { code: 1, name: "Peaking" },
    { code: 5, name: "Low Cut (12dB)" },
    { code: 11, name: "Linkwitz Transform" },
  ]);
  assert.equal(bandTypeName(T.peaking), "Bell");
  assert.equal(bandTypeName(T.highPass), "Low Cut 12 dB/oct");
  assert.equal(bandTypeName(T.highPass1), "Low Cut 6 dB/oct");
  assert.equal(bandTypeName(T.lowPass), "High Cut 12 dB/oct");
  assert.equal(bandTypeName(T.lowShelf1), "Low Shelf 6 dB/oct");
  assert.equal(bandTypeName(T.notch), "Notch");
  assert.equal(bandTypeName(T.allPass), "All Pass 2nd Order");
  assert.equal(bandTypeName(T.allPass1), "All Pass 1st Order");
  // Off and the Linkwitz Transform keep the core's names
  assert.equal(bandTypeName(T.flat), "Off");
  assert.equal(bandTypeName(T.linkwitz), "Linkwitz Transform");
});

test("UWN A17: Save to Device names the preset it writes, not a slot number", () => {
  const s = { activePresetSlot: 0, presetNames: ["Demo", ""], presetOccupied: 0b1 };
  assert.equal(saveQuestion(s), "Save the current settings to preset 1: Demo?");
  assert.equal(saveQuestion({ ...s, activePresetSlot: 1 }), "Save the current settings to preset 2, empty until now?");
  assert.equal(saveQuestion({ ...s, presetNames: ["  "] }), "Save the current settings to preset 1: Preset 1?");
});

test("UWN A16: the status rail's words and dot come from one state: a link that is away reads Reconnecting", () => {
  const bound = { phase: "bound" as const };
  assert.equal(connectionWord({ status: "open", link: null }), "Connected");
  assert.equal(connectionWord({ status: "open", link: bound }), "Connected");
  assert.equal(connectionWord({ status: "open", link: { phase: "reconnecting" } }), "Reconnecting");
  assert.equal(connectionWord({ status: "choosing", link: { phase: "choose_device" } }), "Connecting");
  assert.equal(connectionWord({ status: "failed", link: null }), "Not connected");
});

test("UWN A15: a tab is the device's name over where it is and its firmware; the site only when several; a twin takes its unit or location", () => {
  const f = { name: "DSPi Demo", transport: "Hub", hubName: "DSPi Hub", location: "", unit: "", firmware: "1.1.6", file: "", installation: "" };
  assert.deepEqual(tabWords(f, false), { title: "DSPi Demo", meta: "DSPi Hub · firmware 1.1.6" });
  assert.deepEqual(tabWords({ ...f, transport: "Demo", hubName: "" }, false), { title: "DSPi Demo", meta: "Demo · firmware 1.1.6" });
  assert.deepEqual(tabWords({ ...f, installation: "Site A" }, false), { title: "DSPi Demo", meta: "Site A · DSPi Hub · firmware 1.1.6" });
  assert.deepEqual(tabWords({ ...f, location: "Bar" }, false), { title: "DSPi Demo", meta: "Bar · firmware 1.1.6" });
  // Two of one name: the unit's name on the audio link, else the location in the title
  assert.deepEqual(tabWords({ ...f, unit: "Main left", location: "Main room" }, true), { title: "Main left", meta: "Main room · firmware 1.1.6" });
  assert.deepEqual(tabWords({ ...f, location: "Terrace" }, true), { title: "DSPi Demo · Terrace", meta: "DSPi Hub · firmware 1.1.6" });
  assert.deepEqual(tabWords({ ...f, transport: "File", hubName: "", firmware: "", file: "show.dspipreset" }, false), { title: "DSPi Demo", meta: "File · show.dspipreset" });
});

test("UWN A15: Save to Device's accessible name has its count in words, never a bare digit", () => {
  assert.equal(saveName(0), "Save to Device");
  assert.equal(saveName(1), "Save to Device, 1 unsaved change");
  assert.equal(saveName(3), "Save to Device, 3 unsaved changes");
});

test("UWN A15: the hub chooser's rows: the location from the inventory, the serial whole, remembered offline DSPis with when last seen", () => {
  const link = [
    { handle: 1, serial: "DEMO-ALINK-A", name: "DSPi Demo", firmware: "1.1.6", platform: "rp2350", state: "online" },
    { handle: 2, serial: "DEMO-ALINK-B", name: "DSPi Demo", firmware: "1.1.6", platform: "rp2350", state: "online" },
  ];
  const inv = (serial: string, location: string, online: boolean) =>
    ({ serial, handle: null, online, name: serial === "DEMO-OFFLINE-0042" ? "Bar" : "DSPi Demo", platform: "", fw: "", fwBeta: null, outputs: 9, board: "", hwRev: "", chip: "", chipRev: null, location, note: "", tags: [], firstSeen: "", lastSeen: online ? "" : "not a time", lastReset: null }) as const;
  const rows = chooserRows(link, [inv("DEMO-ALINK-A", "Main room", true), inv("DEMO-OFFLINE-0042", "Bar", false)]);
  assert.deepEqual(
    rows.map((r) => [r.name, r.meta, r.online, r.spoken]),
    [
      ["DSPi Demo", "Main room · online · DEMO-ALINK-A", true, "DSPi Demo, Main room, DEMO-ALINK-A"],
      ["DSPi Demo", "online · DEMO-ALINK-B", true, "DSPi Demo, DEMO-ALINK-B"],
      ["Bar", "Bar · offline, last seen not a time · DEMO-OFFLINE-0042", false, "Bar, Bar, DEMO-OFFLINE-0042"],
    ],
  );
});

test("UWN A16: a failure's line joins the core's notice once: the detail only when it adds something", () => {
  const n = { title: "Not connected", reason: "The DSPi is not connected, or it went away." };
  assert.equal(detailAdds(n, "Device not connected"), false);
  assert.equal(detailAdds(n, ""), false);
  assert.equal(detailAdds(n, "USB transfer timed out"), true);
});

test("UWN A16: Identify's failure is one sentence naming the action and the DSPi, in the core's words", () => {
  const notice = { key: "unsupported", kind: "warning" as const, title: "Not available", reason: "The connected firmware does not have this feature.", action: "Update the DSPi's firmware to use it.", detail: "" };
  assert.equal(identifyFailure("Bench", { text: "Not available", bad: true, notice }), "Identify Bench: not available. The connected firmware does not have this feature.");
  assert.equal(identifyFailure("Bench", { text: "Timed out.", bad: true, notice: null }), "Identify Bench: timed out.");
});

test("UWN A16: a redial that reaches another hub is told apart from a plain loss", () => {
  assert.equal(isOtherHub("network error: another hub answers at this address"), true);
  assert.equal(isOtherHub("network error: connection refused"), false);
  assert.equal(isOtherHub(""), false);
});
