// WP P2WB2 unit tests: the hub settings page's model (which sections the caps give, the page among
// the session's screens, Admin-only edits, the records the forms send, secrets as `HubSecret`
// objects that never print or serialise their text) and the Snapshots screen's "On the hub" words
// (the core's message on the 61 440-byte cap). On the real core: without a hub every one of these
// calls answers `unsupported`, the page is not offered, and the oversized put is refused in the
// core's words with nothing sent.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { inspect } from "node:util";
import {
  HUB_SETTINGS_ROLE_KEY,
  HubSecret,
  actionText,
  hubSettingsShown,
  mayEdit,
  mintedFrom,
  mqttForSet,
  newAction,
  newRule,
  newTarget,
  outboundSetFrom,
  resultText,
  ruleFields,
  ruleForSet,
  sectionsOf,
  targetsForSet,
  targetsOf,
  whenText,
  withHubSettings,
  withPath,
  withRule,
} from "../../src/screens/hub-settings/model";
import { hubFailure, whereLabel } from "../../src/screens/snapshots/hub";
import type { HubCapsRecord, HubMqttSettingsRecord } from "../../src/core/hub-settings-protocol";
import { CoreCallError } from "../../src/core/protocol";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import { Session } from "../../src/ui/session";
import { noticeRow } from "../../src/ui/texts";
import type { ScreenDef } from "../../src/ui/screens";

const caps = (p: Partial<HubCapsRecord>): HubCapsRecord => ({
  onHub: true,
  inventory: false,
  identify: false,
  health: false,
  events: false,
  hubSnapshots: false,
  paramApi: false,
  hubStats: false,
  alarms: false,
  outbound: false,
  schedule: false,
  mqtt: false,
  apiKeys: false,
  rest: false,
  caps: [],
  ...p,
});

test("P2WB2 model: the sections are the hub's caps; an upstream hub (none of them) gets no page", () => {
  assert.deepEqual(sectionsOf(caps({ alarms: true, schedule: true, outbound: true, mqtt: true, apiKeys: true })), ["alarms", "schedule", "outbound", "mqtt", "apiKeys"]);
  assert.deepEqual(sectionsOf(caps({ mqtt: true })), ["mqtt"]);
  assert.equal(hubSettingsShown(caps({})), false, "an upstream WeebLabs hub has none of the families");
  assert.equal(hubSettingsShown(caps({ onHub: false, alarms: true })), false, "no hub session");
  assert.equal(hubSettingsShown(null), false);
  assert.equal(hubSettingsShown(caps({ schedule: true })), true);
});

test("P2WB2 model: the page is its own screen, before Settings, in Advanced and Installer only", () => {
  const View = () => null;
  const def: ScreenDef = { id: "hub-settings", label: "Hub Settings", View };
  const list: ScreenDef[] = ["main", "eq", "events", "settings"].map((id) => ({ id, label: id, View }));
  assert.deepEqual(withHubSettings(list, "advanced", true, def).map((x) => x.id), ["main", "eq", "events", "hub-settings", "settings"]);
  assert.deepEqual(withHubSettings(list, "installer", true, def).map((x) => x.id), ["main", "eq", "events", "hub-settings", "settings"]);
  assert.deepEqual(withHubSettings(list, "simple", true, def).map((x) => x.id), ["main", "eq", "events", "settings"]);
  assert.deepEqual(withHubSettings(list, "advanced", false, def).map((x) => x.id), ["main", "eq", "events", "settings"]);
  assert.equal(withHubSettings(withHubSettings(list, "advanced", true, def), "advanced", true, def).length, 5, "once");
});

test("P2WB2 model: only an Admin of the hub edits, and not in read-only mode; Control and Viewer read", () => {
  assert.equal(mayEdit("admin", ""), true);
  assert.equal(mayEdit("control", ""), false);
  assert.equal(mayEdit("viewer", ""), false);
  assert.equal(mayEdit("", ""), false, "before the hub says");
  assert.equal(mayEdit("admin", "Read-only mode"), false);
  assert.equal(HUB_SETTINGS_ROLE_KEY, "installer.hub_settings_role", "the sibling of installer.hub_role");
});

test("P2WB2 secrets: a HubSecret reveals its text once asked, and never prints, serialises or inspects it", () => {
  const key = "K".repeat(43);
  const s = new HubSecret(key);
  assert.equal(s.reveal(), key);
  assert.equal(String(s), "set");
  assert.equal(`${s}`, "set");
  assert.equal(JSON.stringify({ s }), '{"s":"set"}');
  assert.ok(!inspect(s).includes(key), inspect(s));
  assert.ok(!inspect({ nested: s }).includes(key));
  assert.equal(String(new HubSecret("")), "not set");
  const minted = mintedFrom({ cid: 7, name: "Script", role: "control", plaintextOk: false, key });
  assert.ok(minted.key instanceof HubSecret);
  assert.equal(minted.key.reveal(), key);
  assert.ok(!JSON.stringify(minted).includes(key), "a minted key's record never serialises the key");
  const set = outboundSetFrom({
    targets: [{ ...newTarget(), id: "t1", name: "Desk", secret: "set" }],
    secrets: [{ target: "t1", secret: "ab".repeat(32) }],
  });
  assert.equal(set.secrets.length, 1);
  assert.equal(set.secrets[0]!.name, "Desk");
  assert.equal(set.secrets[0]!.secret.reveal(), "ab".repeat(32));
  assert.ok(!JSON.stringify(set).includes("ab".repeat(32)));
});

test("P2WB2 alarm rules: the fields a code has, a change, numbers typed", () => {
  const rules = [
    { code: "clip", on: true, afterS: null, perMinute: 10, muteOnStorm: false, aboveC: null, abovePct: null, forS: null },
    { code: "cpu", on: true, afterS: null, perMinute: null, muteOnStorm: null, aboveC: null, abovePct: 90, forS: 10 },
  ];
  assert.deepEqual(ruleFields(rules[0]!).map((f) => f.field), ["perMinute"]);
  assert.deepEqual(ruleFields(rules[1]!).map((f) => f.field), ["abovePct", "forS"]);
  const next = withRule(rules, "cpu", { on: false, abovePct: 80 });
  assert.equal(next[1]!.on, false);
  assert.equal(next[1]!.abovePct, 80);
  assert.equal(next[0], rules[0], "the others as they were");
});

test("P2WB2 schedules: a new rule, its words, the record schedule.set takes", () => {
  const r = newRule("DEMO0000DSPI0001");
  assert.deepEqual(r.targets, ["DEMO0000DSPI0001"]);
  assert.equal(whenText(r), "Mon, Tue, Wed, Thu, Fri at 18:00");
  assert.equal(actionText(r.actions[0]!), "Master volume to -20 dB");
  const once = ruleForSet({ ...r, once: "2026-12-24T17:00", nextRun: "x", last: { at: "y", ok: true, skipped: false } });
  assert.deepEqual(once.days, [], "a one-off has no days");
  assert.equal(once.time, "");
  assert.equal("nextRun" in once, false, "read-only fields left out");
  assert.equal("last" in once, false);
  assert.equal(whenText({ ...r, once: "2026-12-24T17:00" }), "Once, 2026-12-24 17:00");
  const mute = withPath(newAction("param.set"), "vol.mute");
  assert.equal(mute.flag, true);
  assert.equal(mute.value, null);
  assert.equal(actionText(mute), "Mute on");
  const out = withPath(mute, "out.mute");
  assert.deepEqual(out.idx, [0]);
  assert.equal(actionText({ ...out, flag: false }), "Output mute 1 off");
  assert.equal(actionText(newAction("preset.load")), "Load preset slot 1");
  assert.deepEqual(targetsOf(" A1, B2;C3  D4 "), ["A1", "B2", "C3", "D4"]);
  assert.equal(resultText({ serial: "S1", action: "0:param.set", ok: true, code: "", msg: "" }), "S1: 0:param.set: done");
  assert.equal(resultText({ serial: "S2", action: "0:param.set", ok: false, code: "offline", msg: "not attached" }), "S2: 0:param.set: offline: not attached");
});

test("P2WB2 notifications: a new webhook asks for a secret, is https, every kind; a kept secret is sent as keep", () => {
  const t = newTarget();
  assert.equal(t.secret, "new");
  assert.equal(t.url, "https://");
  assert.equal(t.plaintext, false);
  assert.equal(t.kinds.length, 5);
  const [sent] = targetsForSet([{ ...t, id: "t1", secret: "set", url: " http://192.168.1.5/hook ", plaintext: true, queued: 3, encrypted: false, trust: "", notice: "n" }]);
  assert.equal(sent!.secret, "", "the hub keeps the secret");
  assert.equal(sent!.url, "http://192.168.1.5/hook");
  assert.equal(sent!.plaintext, true, "the private-address rule is the hub's, on every send");
  assert.equal("queued" in sent!, false);
  assert.equal("encrypted" in sent!, false);
  assert.equal("notice" in sent!, false);
});

test("P2WB2 MQTT: the password and client key are sent only when typed or cleared; tls whole; the HA key", () => {
  const s: HubMqttSettingsRecord = {
    enabled: true,
    broker: " broker.lan ",
    port: null,
    clientId: "dspi",
    username: "u",
    password: "set",
    baseTopic: "dspi/h",
    tls: { mode: "on", caPem: "PEM", pinSha256: "", clientCertPem: "", clientKeyPem: "set", clientCertFile: "", clientKeyFile: "", allowPlaintextPassword: false },
    haEnabled: true,
    haKey: 9,
    notice: "once",
  };
  const kept = mqttForSet(s, { typed: "", clear: false }, { typed: "", clear: false });
  assert.equal(kept.password, "", "kept by the hub");
  assert.equal(kept.tls.clientKeyPem, "set", "kept by the hub");
  assert.equal(kept.tls.caPem, "PEM", "tls is replaced whole: every field sent back");
  assert.equal(kept.broker, "broker.lan");
  assert.equal(kept.haKey, 9);
  assert.equal("notice" in kept, false);
  assert.equal(mqttForSet(s, { typed: "pw", clear: false }, { typed: "", clear: false }).password, "pw");
  assert.equal(mqttForSet(s, { typed: "", clear: true }, { typed: "", clear: false }).password, "not set");
  assert.equal(mqttForSet(s, { typed: "", clear: false }, { typed: "", clear: true }).tls.clientKeyPem, "");
  assert.equal(mqttForSet({ ...s, tls: { ...s.tls, mode: "" } }, { typed: "", clear: false }, { typed: "", clear: false }).tls.mode, "off");
});

test("P2WB2 shared snapshots: the core's where in words; a refusal in the core's own message", () => {
  assert.equal(whereLabel("hub"), "On the hub");
  assert.equal(whereLabel("both"), "Here and on the hub");
  assert.equal(whereLabel("local"), "Here");
  const e = new CoreCallError("link", "the snapshot is 70000 bytes compact; the hub takes at most 61440", "dlink.conflict");
  const line = hubFailure(e);
  assert.ok(line.includes("The snapshot is 70000 bytes compact; the hub takes at most 61440"), line);
});

// -- On the real core --

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

test("P2WB2 core: off a hub every settings call is unsupported, no page; the oversized put is the core's refusal", { skip }, async () => {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const client = {
    call: (m: never, p: never) => core.call(m, p),
    subscribe: (fn: never) => {
      core.onEvent(fn);
      return () => undefined;
    },
    dispose: async () => undefined,
  } as unknown as CoreClient;
  const session = new Session("s", "i", "Here", { kind: "demo" }, "control", client);
  await session.open({ kind: "demo" });
  const c = await session.core.call("hubCaps", undefined);
  assert.equal(c.onHub, false);
  assert.equal(hubSettingsShown(c), false, "the demo (no hub) shows no hub settings page");
  assert.ok(noticeRow(HUB_SETTINGS_ROLE_KEY), "the core's row for the read-only page");
  assert.equal(noticeRow(HUB_SETTINGS_ROLE_KEY)!.title, noticeRow("installer.hub_role")!.title, "the sibling's title");
  const unsupported = (e: unknown) => e instanceof CoreCallError && e.code === "unsupported";
  await assert.rejects(session.core.call("hubAlarmRulesGet", undefined), unsupported);
  await assert.rejects(session.core.call("hubSchedules", undefined), unsupported);
  await assert.rejects(session.core.call("hubScheduleRunNow", { id: "r1" }), unsupported);
  await assert.rejects(session.core.call("hubOutboundGet", undefined), unsupported);
  await assert.rejects(session.core.call("hubOutboundTest", { id: "t1" }), unsupported);
  await assert.rejects(session.core.call("hubMqttGet", undefined), unsupported);
  await assert.rejects(session.core.call("hubApiKeys", undefined), unsupported);
  await assert.rejects(session.core.call("hubApiKeyMint", { name: "k", role: "viewer" }), unsupported);
  await assert.rejects(session.core.call("hubSnapshotsGet", { id: "1" }), unsupported);
  const merged = await session.core.call("snapshotListMerged", {});
  assert.deepEqual(merged.entries, []);
  assert.equal(merged.hubListed, false);
  // Over the cap: the core's own words, before any hub is asked.
  const big = JSON.stringify({ meta: { kind: "snapshot" }, pad: "x".repeat(61_440) });
  const refused = await session.core.call("hubSnapshotsPut", { text: big }).then(
    () => null,
    (e: unknown) => e,
  );
  assert.ok(refused instanceof CoreCallError, String(refused));
  assert.equal(refused.noticeKey, "dlink.conflict");
  assert.ok(refused.message.length > 0);
  assert.ok(hubFailure(refused).toLowerCase().includes(refused.message.toLowerCase()), hubFailure(refused));
  await session.close();
});
