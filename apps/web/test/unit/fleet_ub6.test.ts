// WP UB6 unit tests (UXP B6, D13): the Fleet of a hub-served console, the hub listed once, the strip's words,
// the hub page's counts, Identify not offered again where the firmware refused it, Hub Settings' one unsaved bar.

import { test } from "node:test";
import assert from "node:assert/strict";
import { fleetHubOf, sharedCount, withoutFleetEntry } from "../../src/screens/hub/FleetPage";
import { identifyRefused, refuseIdentify, type HubView } from "../../src/screens/hub/feed";
import { alarmCountText, deviceAlarms, fleetSummary } from "../../src/screens/hub/model";
import { hubSharesText } from "../../src/ui/shell/NoDevice";
import { CONNECT, FLEET, sectionsFor } from "../../src/ui/shell/sections";
import { changesText, mqttChanges, ruleChanges, targetChanges } from "../../src/screens/hub-settings/model";
import type { Installation } from "../../src/fleet/types";
import type { HubAlarmRecord, HubDeviceRecord } from "../../src/core/protocol";
import type { HubAlarmRuleRecord, HubMqttSettingsRecord, HubOutboundTargetRecord } from "../../src/core/hub-settings-protocol";

const URL = "ws://127.0.0.1:11915/dspi/v1";

/** The hub backend's list: the hub's inventory first, then this console's own with the serving hub's entry. */
function listed(devices: { serial: string; online: boolean }[]): Installation[] {
  return [
    { id: "hub", name: "DSPi Hub", editable: false, sources: devices.map((d) => ({ kind: "hub" as const, url: URL, deviceId: d.serial, name: d.serial, online: d.online })) },
    { id: "local", name: "This console", editable: true, sources: [{ kind: "demo" }, { kind: "usb" }, { kind: "hub", url: URL }, { kind: "hub", url: "ws://other:11915/dspi/v1" }] },
  ];
}

function device(serial: string, online: boolean): HubDeviceRecord {
  return {
    serial, handle: online ? 1 : null, online, name: serial, platform: "rp2350", fw: "1.1.6", fwBeta: false, outputs: 9, board: "", hwRev: "", chip: "", chipRev: null,
    location: "", note: "", tags: [], firstSeen: "", lastSeen: "", lastReset: null,
  };
}

function alarm(id: string, serial: string, state: string, severity = "warning", source = "device"): HubAlarmRecord {
  return { id, serial, state, severity, source, code: "temperature", message: "hot", count: 1, raisedAt: "2026-11-26T10:00:00Z", clearedAt: "", acknowledgedBy: "", noticeKey: "alarm.temperature" } as unknown as HubAlarmRecord;
}

test("UB6 D13: a hub-served page's Fleet is the hub backend's inventory installation, on the hub's address", () => {
  const hub = fleetHubOf(listed([{ serial: "A", online: true }]));
  assert.equal(hub?.inst.id, "hub");
  assert.equal(hub?.url, URL);
  // A hub that has seen no DSPi yet: its own entry gives the address
  assert.equal(fleetHubOf(listed([]))?.url, URL);
  // The standalone list (or a hub that wants its PIN first): no Fleet
  assert.equal(fleetHubOf([listed([])[1]!]), null);
});

test("UB6 B6: beside a Fleet the serving hub is listed once (its own entry goes, other hubs stay)", () => {
  const all = listed([{ serial: "A", online: true }]);
  const shown = withoutFleetEntry(all, fleetHubOf(all));
  const local = shown.find((i) => i.id === "local")!;
  assert.deepEqual(local.sources, [{ kind: "demo" }, { kind: "usb" }, { kind: "hub", url: "ws://other:11915/dspi/v1" }]);
  assert.equal(shown.find((i) => i.id === "hub")!.sources.length, 1);
  // On an https page the entry is wss:// for the same hub
  const tls = all.map((i) => (i.id === "local" ? { ...i, sources: i.sources.map((s) => (s.kind === "hub" && !s.deviceId ? { ...s, url: s.url.replace("ws:", "wss:") } : s)) } : i));
  assert.equal(withoutFleetEntry(tls, fleetHubOf(all)).find((i) => i.id === "local")!.sources.filter((s) => s.kind === "hub").length, 1);
  // No Fleet: nothing left out
  assert.deepEqual(withoutFleetEntry(all, null), all);
});

test("UB6 B6: the strip says what the hub shares, never 'connect one over USB'", () => {
  assert.equal(hubSharesText(2), "This hub shares 2 DSPis.");
  assert.equal(hubSharesText(1), "This hub shares 1 DSPi.");
  assert.equal(hubSharesText(0), "This hub shares no DSPi right now.");
  assert.equal(hubSharesText(null), "Asking the hub which DSPis it shares…");
  const all = listed([{ serial: "A", online: true }, { serial: "B", online: true }, { serial: "C", online: false }]);
  // From the list until the feed has read the inventory, then the feed's
  assert.equal(sharedCount(fleetHubOf(all), null), 2);
  const view = { caps: { inventory: true }, devices: [device("A", true), device("C", false)] } as unknown as HubView;
  assert.equal(sharedCount(fleetHubOf(all), view), 1);
  assert.equal(sharedCount(null, null), null);
});

test("UB6 D13: Devices' first page is the Fleet only when there is one", () => {
  const devices = (fleet: boolean) => sectionsFor([], fleet).find((s) => s.id === "devices")!.pages.map((p) => p.id);
  assert.deepEqual(devices(true), [FLEET, CONNECT]);
  assert.deepEqual(devices(false), [CONNECT]);
});

test("UB6 B6: the Fleet's counts and a row's alarms (the audio link's and cleared ones not counted)", () => {
  const devices = [device("A", true), device("B", true), device("C", false)];
  const alarms = [alarm("1", "A", "raised", "critical"), alarm("2", "A", "acknowledged"), alarm("3", "B", "cleared"), alarm("4", "B", "raised", "warning", "alink")];
  assert.deepEqual(fleetSummary(devices, alarms), { online: 2, offline: 1, raised: 1, critical: true });
  assert.deepEqual(deviceAlarms(alarms, "A").map((a) => a.id), ["1", "2"]);
  assert.deepEqual(deviceAlarms(alarms, "B"), []);
  assert.equal(alarmCountText(1), "1 alarm");
  assert.equal(alarmCountText(3), "3 alarms");
  assert.equal(alarmCountText(0), "");
});

test("UB6 B6: a DSPi whose firmware refused Identify is remembered for the visit, on the Fleet and the hub page alike", () => {
  assert.equal(identifyRefused("UB6-BENCH"), "");
  refuseIdentify("UB6-BENCH", "Identify Bench: not available.");
  assert.equal(identifyRefused("UB6-BENCH"), "Identify Bench: not available.");
  assert.equal(identifyRefused("UB6-OTHER"), "");
});

test("UB6 B6: Hub Settings counts its changes against the hub's copy, one per rule, target or field", () => {
  const r = (code: string, on: boolean, aboveC?: number) => ({ code, on, ...(aboveC === undefined ? {} : { aboveC }) }) as unknown as HubAlarmRuleRecord;
  const saved = [r("temperature", true, 60), r("clip", false)];
  assert.equal(ruleChanges(saved, saved), 0);
  assert.equal(ruleChanges(saved, [r("temperature", true, 55), r("clip", true)]), 2);
  assert.equal(ruleChanges(saved, [r("temperature", true, 60), r("clip", false)]), 0);
  assert.equal(ruleChanges(null, saved), 0);

  const t = (id: string, name: string, extra: Partial<HubOutboundTargetRecord> = {}) =>
    ({ id, name, kind: "webhook", url: "https://x", enabled: true, kinds: ["alarm"], secret: "set", plaintext: false, caPem: "", pinSha256: "", ...extra }) as HubOutboundTargetRecord;
  const was = [t("a", "Ops"), t("b", "Pager")];
  assert.equal(targetChanges(was, was), 0);
  // The hub's report (last send, queue) is not a change
  assert.equal(targetChanges(was, [t("a", "Ops", { queued: 3, last: { at: "now", ok: true, status: 200, msg: "" } }), t("b", "Pager")]), 0);
  // One renamed, one removed, one added, kinds changed on none
  assert.equal(targetChanges(was, [t("a", "Ops desk"), t("", "New")]), 3);
  assert.equal(targetChanges(was, [t("a", "Ops", { kinds: ["alarm", "restart"] }), t("b", "Pager")]), 1);

  const m: HubMqttSettingsRecord = {
    enabled: false, broker: "", port: null, clientId: "dspi", username: "", password: "not set", baseTopic: "dspi", haEnabled: false, haKey: null,
    tls: { mode: "off", caPem: "", pinSha256: "", clientCertPem: "", clientKeyPem: "not set", clientCertFile: "", clientKeyFile: "", allowPlaintextPassword: false },
  };
  const none = { typed: "", clear: false };
  assert.equal(mqttChanges(m, m, none, none), 0);
  assert.equal(mqttChanges(m, { ...m, enabled: true, broker: "mqtt.local", tls: { ...m.tls, mode: "on" } }, { typed: "pw", clear: false }, none), 4);
  assert.equal(changesText(1), "1 change");
  assert.equal(changesText(3), "3 changes");
});
