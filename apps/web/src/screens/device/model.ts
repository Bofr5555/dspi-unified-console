// WP PGWEB: the device-info screen's words and the raw screen's parsing (PLAN-PG sections 1.1,
// 1.3, 1.6, 1.8, 1.9). Pure: no core calls here. The device info comes from the core's
// `deviceProtocol` (read once per connection, Session.state.protocol); the raw requests by id
// from `rawGetId`/`rawSetId`; the update verdict from `firmwareTarget` + `firmwareUpdateCheck`.

import { CoreCallError, type DeviceProtocolRecord, type FirmwareUpdateVerdictRecord } from "../../core/protocol";

/** The pages upstream allocates (the vendored registry) and ours (0xE0, the extension page). */
const PAGE_NAMES: Record<number, string> = {
  0x01: "SYS",
  0x02: "STORE",
  0x03: "INPUT",
  0x04: "OUTPUT",
  0x05: "ROUTE",
  0x06: "FILTER",
  0x07: "DYN",
  0x08: "FX",
  0x09: "MEAS",
  0x0a: "CS",
  0x0b: "XPORT",
  0x0c: "CLOCK",
  0xe0: "EXT",
  0xf0: "DIAG",
};

export const hex = (n: number, digits: number) => `0x${n.toString(16).toUpperCase().padStart(digits, "0")}`;

/** A page by number and name: `0x05 ROUTE`. */
export function pageName(page: number): string {
  const name = PAGE_NAMES[page];
  return name ? `${hex(page, 2)} ${name}` : hex(page, 2);
}

/** The generation row: which protocol the device speaks, as the probe decided it on this connection. */
export function generationWords(p: DeviceProtocolRecord): string {
  switch (p.generation) {
    case "paged":
      return `Firmware 1.1.7 or later: paged protocol ${p.protocolMajor}.${p.protocolMinor}`;
    case "legacy":
      return "Firmware 1.1.6 or older: the 1.1.6 protocol";
    default:
      return "Not said by this connection";
  }
}

/** The extension channel's row. */
export function envelopeWords(p: DeviceProtocolRecord): string {
  switch (p.extEnvelope) {
    case "tunnel":
      return "This fork's extensions, through page 0xE0 (the tunnel 0xE010)";
    case "legacy":
      return "This fork's extensions, on the 1.1.6 extension request (0x00)";
    default:
      return "None (upstream firmware)";
  }
}

/** The pages row: every page with its revision, or why there are none. */
export function pagesWords(p: DeviceProtocolRecord): string {
  if (p.generation !== "paged") return "None: the 1.1.6 protocol has no pages";
  if (!p.pages.length) return "Not listed on this connection";
  return p.pages.map((x) => `${pageName(x.page)} rev ${x.revision}`).join(", ");
}

/** The device's kind in words. */
export function kindWords(p: DeviceProtocolRecord): string {
  return p.deviceKind === "alinkSender" ? "Audio link sender" : "DSP";
}

// -- the raw screen --

/** The firmware's result codes (`VC_ERR_*`, `legacy_map.json`), with what each means. */
const RESULTS: Record<number, [string, string]> = {
  0x10: ["VC_ERR_UNKNOWN", "the firmware does not know this command"],
  0x11: ["VC_ERR_DIRECTION", "the command does not take this direction"],
  0x12: ["VC_ERR_LENGTH", "the length is wrong for this command"],
  0x13: ["VC_ERR_ADDRESS", "wValue names something that does not exist"],
  0x14: ["VC_ERR_VALUE", "a value is out of range"],
  0x15: ["VC_ERR_UNSUPPORTED", "this build does not support it"],
  0x16: ["VC_ERR_BLOCKED", "the firmware blocks it now"],
  0x17: ["VC_ERR_BUSY", "the device is busy; try again"],
  0x18: ["VC_ERR_STATE", "not possible in the device's current state"],
  0x19: ["VC_ERR_PIN_INVALID", "the pin is not valid"],
  0x1a: ["VC_ERR_PIN_IN_USE", "the pin is in use"],
};

/** A result code by name and meaning, `VC_ERR_VALUE (0x14): a value is out of range`. */
export function resultWords(status: number): string {
  const r = RESULTS[status];
  return r ? `${r[0]} (${hex(status, 2)}): ${r[1]}` : `result code ${hex(status, 2)}`;
}

/**
 * A raw-id call's failure in words, saying whether anything was sent. The core refuses
 * locally with nothing sent (`invalid`, `unsupported`, `read_only`); the firmware's own
 * refusal is `refused` with its status byte.
 */
export function rawRefusalWords(id: number, e: unknown): string {
  const name = hex(id, 4);
  if (!(e instanceof CoreCallError)) return `${name}: ${e instanceof Error ? e.message : String(e)}`;
  switch (e.code) {
    case "invalid":
      return `${name} was not sent: ${e.message.replace(/^invalid argument: /, "")}`;
    case "unsupported":
      return `${name} was not sent: this device speaks the 1.1.6 protocol, which has no pages (or this connection does not say it is paged).`;
    case "read_only":
      return `${name} was not sent: ${e.message}`;
    case "refused": {
      const m = /status (\d+)/.exec(e.message);
      const status = m ? Number(m[1]) : 0;
      return status ? `The device refused ${name}: ${resultWords(status)}.` : `The device refused ${name}; it gave no result code.`;
    }
    default:
      return `${name}: ${e.message}`;
  }
}

/** A parsed number, or why it cannot be taken whole. */
type Parsed = { ok: true; value: number } | { ok: false; why: string };

/**
 * A whole number written in decimal or as `0x` hex, within `min..max`. Anything that would be
 * cut to fit (a fraction, a sign, a number past `max`, trailing text) is refused, never truncated.
 */
export function parseWhole(text: string, what: string, min: number, max: number): Parsed {
  const t = text.trim();
  const m = /^(?:0[xX]([0-9a-fA-F]+)|([0-9]+))$/.exec(t);
  if (!m) return { ok: false, why: `${what}: “${t}” is not a whole number (decimal, or hex as 0x…)` };
  const value = m[1] !== undefined ? parseInt(m[1], 16) : parseInt(m[2]!, 10);
  if (!Number.isSafeInteger(value) || value > max || value < min)
    return { ok: false, why: `${what}: ${t} is outside ${hex(min, max > 0xff ? 4 : 2)}..${hex(max, max > 0xff ? 4 : 2)}; it would not fit and is not cut to fit` };
  return { ok: true, value };
}

/** A SET's payload: bytes separated by spaces or commas, each 0..0xFF; at most 65535. */
export function parseBytes(text: string): { ok: true; value: number[] } | { ok: false; why: string } {
  const parts = text.split(/[\s,]+/).filter(Boolean);
  if (parts.length > 0xffff) return { ok: false, why: `Data: ${parts.length} bytes is more than 65535; it is not cut to fit` };
  const out: number[] = [];
  for (const p of parts) {
    const b = parseWhole(p, "Data", 0, 0xff);
    if (!b.ok) return b;
    out.push(b.value);
  }
  return { ok: true, value: out };
}

/** A raw request by id, checked whole before anything is sent. */
export type RawRequest = { dir: "get"; id: number; value: number; len: number } | { dir: "set"; id: number; value: number; data: number[] };

export interface RawForm {
  dir: "get" | "set";
  id: string;
  value: string;
  len: string;
  data: string;
}

/** The form as a request, or why it is refused (nothing is sent then). */
export function parseRaw(f: RawForm): { ok: true; req: RawRequest } | { ok: false; why: string } {
  const id = parseWhole(f.id, "Command id", 0, 0xffff);
  if (!id.ok) return id;
  if (id.value >> 8 === 0) return { ok: false, why: `Command id: ${hex(id.value, 4)} is on page 0, a 1.1.6 opcode; a paged id is page << 8 | command (e.g. 0x0550)` };
  const value = f.value.trim() ? parseWhole(f.value, "wValue", 0, 0xffff) : ({ ok: true, value: 0 } as const);
  if (!value.ok) return value;
  if (f.dir === "get") {
    const len = parseWhole(f.len, "Length", 1, 0xffff);
    if (!len.ok) return len;
    return { ok: true, req: { dir: "get", id: id.value, value: value.value, len: len.value } };
  }
  const data = parseBytes(f.data);
  if (!data.ok) return data;
  return { ok: true, req: { dir: "set", id: id.value, value: value.value, data: data.value } };
}

/** A reply's bytes as hex. */
export function bytesHex(data: number[]): string {
  return data.length ? data.map((b) => b.toString(16).toUpperCase().padStart(2, "0")).join(" ") : "(no bytes)";
}

// -- the update check --

/** The rule that decided, in words; "" when only the voltage rule spoke. */
export function ruleWords(v: FirmwareUpdateVerdictRecord): string {
  switch (v.rule) {
    case "noBootsel":
      return "No BOOTSEL: this board has no BOOTSEL bootloader, so no image is offered to it, whatever is typed.";
    case "lineageLoss":
      return "Lineage: this board runs this fork's firmware, and the image may be upstream's. It would give that firmware away; the app that writes it asks for the typed confirmation.";
    default:
      return "";
  }
}

/** The verdict's headline. */
export function verdictWords(v: FirmwareUpdateVerdictRecord): string {
  if (v.verdict.allowed) return v.verdict.decision === "confirmed" ? "Allowed, as confirmed" : "Allowed: no known risk";
  return v.verdict.decision === "needsConfirmation" ? "Needs a typed confirmation" : "Refused";
}
