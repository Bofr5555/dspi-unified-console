// WP PGWEB unit tests: the device-info screen's words per generation (PLAN-PG 1.1, 1.3, 1.9), the
// raw screen's parsing (nothing that would be truncated is taken), the core's refusals in words
// (nothing sent, or the firmware's VC_ERR_* by name), the update verdict's rule words, the
// rollout's reason words, and the device screen's place in the tab row.

import { test } from "node:test";
import assert from "node:assert/strict";
import { CoreCallError, type DeviceProtocolRecord, type FirmwareUpdateVerdictRecord } from "../../src/core/protocol";
import {
  envelopeWords,
  generationWords,
  pagesWords,
  parseBytes,
  parseRaw,
  parseWhole,
  rawRefusalWords,
  resultWords,
  ruleWords,
  verdictWords,
} from "../../src/screens/device/model";
import { stateWord, updateReasonWords } from "../../src/screens/hub/rollout-model";
import { screensFor, withDeviceScreen } from "../../src/ui/screens";

const legacy: DeviceProtocolRecord = {
  generation: "legacy",
  protocolMajor: 0,
  protocolMinor: 0,
  pages: [],
  extEnvelope: "legacy",
  boardId: 0,
  boardName: "DSPi",
  deviceKind: "dsp",
};
const paged: DeviceProtocolRecord = {
  ...legacy,
  generation: "paged",
  protocolMajor: 1,
  protocolMinor: 0,
  pages: [
    { page: 1, revision: 1 },
    { page: 5, revision: 2 },
    { page: 0xe0, revision: 1 },
  ],
  extEnvelope: "tunnel",
};

test("wp_pgweb: the generation, pages and extension rows per generation", () => {
  assert.equal(generationWords(legacy), "Firmware 1.1.6 or older: the 1.1.6 protocol");
  assert.equal(generationWords(paged), "Firmware 1.1.7 or later: paged protocol 1.0");
  assert.equal(generationWords({ ...legacy, generation: "unknown" }), "Not said by this connection");
  assert.equal(pagesWords(legacy), "None: the 1.1.6 protocol has no pages");
  assert.equal(pagesWords(paged), "0x01 SYS rev 1, 0x05 ROUTE rev 2, 0xE0 EXT rev 1");
  assert.equal(pagesWords({ ...paged, pages: [] }), "Not listed on this connection");
  assert.match(envelopeWords(legacy), /1\.1\.6 extension request \(0x00\)/);
  assert.match(envelopeWords(paged), /page 0xE0 \(the tunnel 0xE010\)/);
  assert.equal(envelopeWords({ ...legacy, extEnvelope: "noChannel" }), "None (upstream firmware)");
});

test("wp_pgweb: raw input that would be truncated is refused, never cut", () => {
  assert.deepEqual(parseWhole("0x0550", "id", 0, 0xffff), { ok: true, value: 0x0550 });
  assert.deepEqual(parseWhole("17", "id", 0, 0xffff), { ok: true, value: 17 });
  for (const bad of ["0x10000", "65536", "-1", "1.5", "0x", "12ab", "1e3", ""]) assert.equal(parseWhole(bad, "id", 0, 0xffff).ok, false, bad);
  assert.equal(parseBytes("0x00 0x80, 255").ok, true);
  assert.deepEqual(parseBytes("0x00 0x80, 255"), { ok: true, value: [0, 0x80, 255] });
  const cut = parseBytes("0x1FF");
  assert.equal(cut.ok, false);
  assert.match(cut.ok ? "" : cut.why, /not cut to fit/);
  // A page-0 id is a 1.1.6 opcode: refused, not sent as page 0
  const p0 = parseRaw({ dir: "get", id: "0x50", value: "", len: "4", data: "" });
  assert.equal(p0.ok, false);
  assert.match(p0.ok ? "" : p0.why, /page 0/);
  assert.equal(parseRaw({ dir: "get", id: "0x0550", value: "", len: "0", data: "" }).ok, false);
  assert.equal(parseRaw({ dir: "get", id: "0x0550", value: "0x10000", len: "4", data: "" }).ok, false);
  assert.deepEqual(parseRaw({ dir: "get", id: "0x0550", value: "", len: "4", data: "" }), { ok: true, req: { dir: "get", id: 0x0550, value: 0, len: 4 } });
  assert.deepEqual(parseRaw({ dir: "set", id: "0x0550", value: "2", len: "", data: "1 2" }), { ok: true, req: { dir: "set", id: 0x0550, value: 2, data: [1, 2] } });
});

test("wp_pgweb: the core's refusals of a raw-id call in words", () => {
  assert.equal(
    rawRefusalWords(0x0130, new CoreCallError("invalid", "invalid argument: SYS.ENTER_BOOTLOADER restarts the device")),
    "0x0130 was not sent: SYS.ENTER_BOOTLOADER restarts the device",
  );
  assert.match(rawRefusalWords(0x0550, new CoreCallError("unsupported", "unsupported")), /was not sent: this device speaks the 1\.1\.6 protocol/);
  assert.equal(
    rawRefusalWords(0x0550, new CoreCallError("refused", "device refused the request (status 20)")),
    "The device refused 0x0550: VC_ERR_VALUE (0x14): a value is out of range.",
  );
  assert.equal(rawRefusalWords(0x0550, new CoreCallError("refused", "device refused the request")), "The device refused 0x0550; it gave no result code.");
  assert.equal(resultWords(0x99), "result code 0x99");
});

test("wp_pgweb: the update verdict's rules in words, and the rollout's reasons", () => {
  const v = (rule: FirmwareUpdateVerdictRecord["rule"], allowed: boolean, decision: FirmwareUpdateVerdictRecord["verdict"]["decision"]): FirmwareUpdateVerdictRecord => ({
    verdict: { allowed, decision, reason: "safe", message: "m", confirmationPhrase: "", imageMv: 0, requiredMv: 1150 },
    rule,
    imageLineage: "upstream",
    imageGeneration: "legacy",
    generationChange: "",
  });
  assert.match(ruleWords(v("noBootsel", false, "refused")), /^No BOOTSEL/);
  assert.match(ruleWords(v("lineageLoss", false, "needsConfirmation")), /^Lineage/);
  assert.equal(ruleWords(v("none", true, "clear")), "");
  assert.equal(verdictWords(v("none", true, "clear")), "Allowed: no known risk");
  assert.equal(verdictWords(v("lineageLoss", false, "needsConfirmation")), "Needs a typed confirmation");
  assert.equal(verdictWords(v("noBootsel", false, "refused")), "Refused");
  assert.equal(stateWord("excluded"), "Excluded");
  assert.match(updateReasonWords("no_bootsel"), /^No BOOTSEL/);
  assert.match(updateReasonWords("lineage_loss"), /^Lineage/);
  assert.equal(updateReasonWords("platform"), "");
});

test("wp_pgweb: the device screen is in every mode, just before Settings", () => {
  for (const m of ["simple", "advanced", "installer"] as const) {
    const ids = withDeviceScreen(screensFor(m, false)).map((s) => s.id);
    assert.deepEqual(ids.slice(-2), ["device", "settings"], m);
    assert.equal(withDeviceScreen(withDeviceScreen(screensFor(m, false))).filter((s) => s.id === "device").length, 1);
  }
});
