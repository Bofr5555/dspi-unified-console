// The input pages' model (WP WB6g): the L/R link of an input pair (M48), the
// input trims through it, and the input sources' built-in titles. The link is
// the console's, as on every client (the core's pair setters say so): one bit
// per pair (inputs 2p and 2p+1), kept per device serial in this page's
// storage, pair 1/2 linked until the user says otherwise (Qt's
// `inputLinks/<serial>`, the Console's `linkedInputPairs`). Every write is the
// core's: a linked pair's trim is one call (`inputPairPreamp`) and one undo
// step; linking a pair that differs copies the kept side onto the other in
// one undo group. No I/O here but the session's calls, so the unit tests
// reach the rest.

import type { DspState, FilterParams } from "../../core/protocol";
import type { KeyValueStore } from "../../fleet/local";
import { safeLocalStorage } from "../../fleet/local";
import type { Session } from "../../ui/session";

/** Where the link bits are kept: `{serial: mask}`. */
export const LINKS_KEY = "dspi-web.inputLinks";
/** Pair 1/2 linked, as the Console starts. */
export const DEFAULT_LINKS = 1;
/** Four pairs: inputs 1..8. */
const ALL_PAIRS = 0xf;

function readAll(storage: KeyValueStore | null): Record<string, number> {
  try {
    const v = JSON.parse(storage?.getItem(LINKS_KEY) ?? "{}") as unknown;
    return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, number>) : {};
  } catch {
    return {};
  }
}

/** The link bits kept for `serial`, or the default. */
export function loadLinks(storage: KeyValueStore | null, serial: string): number {
  const m = readAll(storage)[serial];
  return typeof m === "number" && Number.isInteger(m) ? m & ALL_PAIRS : DEFAULT_LINKS;
}

/** Keep `mask` for `serial`; without storage it holds for this visit. */
export function saveLinks(storage: KeyValueStore | null, serial: string, mask: number): void {
  if (!serial) return;
  const all = readAll(storage);
  all[serial] = mask & ALL_PAIRS;
  try {
    storage?.setItem(LINKS_KEY, JSON.stringify(all));
  } catch {
    // Not kept: the link holds for this visit.
  }
}

/** Both halves of `input`'s pair are inputs of this device (fewer suspend a link, never forget it). */
export function pairAvailable(s: Pick<DspState, "numInputChannels">, input: number): boolean {
  return input >= 0 && Math.max(input, input ^ 1) < s.numInputChannels;
}

/** "1/2", "3/4": the pair's inputs as the channel list numbers them. */
export function pairLabel(input: number): string {
  const a = input & ~1;
  return `${a + 1}/${a + 2}`;
}

/** A band as the link compares it (the Console's `inputPairMismatch`): `qp` only for the Linkwitz Transform. */
function sameBand(x: FilterParams, y: FilterParams): boolean {
  return (
    x.filterType === y.filterType &&
    x.freq === y.freq &&
    x.q === y.q &&
    x.gain === y.gain &&
    !!x.bypass === !!y.bypass &&
    (x.filterType !== LINKWITZ_TRANSFORM || x.qp === y.qp)
  );
}

/** The firmware's Linkwitz Transform type. */
const LINKWITZ_TRANSFORM = 13;

/** What differs between the two halves of `input`'s pair: their PEQ bands, their trims (kept to 0.1 dB). */
export function pairMismatch(s: DspState, input: number): { bands: boolean; preamp: boolean } {
  if (!pairAvailable(s, input)) return { bands: false, preamp: false };
  const partner = input ^ 1;
  const n = s.maxBands || 10;
  const a = s.filters?.[input] ?? [];
  const b = s.filters?.[partner] ?? [];
  let bands = false;
  for (let i = 0; i < n && !bands; i++) {
    const x = a[i];
    const y = b[i];
    if (x && y) bands = !sameBand(x, y);
    else bands = !!x !== !!y;
  }
  const preamp = Math.abs((s.inputPreampDb[input] ?? 0) - (s.inputPreampDb[partner] ?? 0)) > 0.05;
  return { bands, preamp };
}

/** The question asked before linking a pair that differs, in the Console's words. */
export function mismatchQuestion(input: number, m: { bands: boolean; preamp: boolean }): { title: string; text: string; keepThis: string; keepOther: string } {
  const partner = input ^ 1;
  const differences = m.bands && m.preamp ? "different filters and input trims" : m.bands ? "different filters" : "different input trims";
  return {
    title: `Inputs ${input + 1} and ${partner + 1} don't match`,
    text:
      `These inputs have ${differences}. Linking mirrors future edits across both channels, but it cannot merge settings that already differ. ` +
      "Choose which channel's filters and trim to copy onto the other. This overwrites the other channel (Undo puts it back).",
    keepThis: `Keep IN${input + 1}`,
    keepOther: `Keep IN${partner + 1}`,
  };
}

/** The serial a session's links are kept under, as the mode is (src/ui/screens.tsx). */
export function serialOf(session: Session): string {
  return session.state.snapshot?.device.serial || session.deviceId || session.id;
}

/**
 * One session's pair links: read from the page's storage for its device, and
 * told to every view of the session when they change (the Inputs page, the
 * EQ's input header, the matrix trims, the main window's trims).
 */
export class InputLinks {
  private readonly listeners = new Set<() => void>();
  private cache: { serial: string; mask: number } | null = null;

  constructor(
    private readonly session: Session,
    private readonly storage: KeyValueStore | null = safeLocalStorage(),
  ) {}

  /** The link bits of the session's device now. */
  get mask(): number {
    const serial = serialOf(this.session);
    if (this.cache?.serial !== serial) this.cache = { serial, mask: loadLinks(this.storage, serial) };
    return this.cache.mask;
  }

  isLinked(input: number): boolean {
    return input >= 0 && input < 8 && (this.mask & (1 << (input >> 1))) !== 0;
  }

  /** `input`'s partner while their pair is linked and both are inputs of the device, else -1. */
  partner(input: number): number {
    const s = this.session.state.snapshot?.state;
    return s && this.isLinked(input) && pairAvailable(s, input) ? input ^ 1 : -1;
  }

  /** Set or clear `input`'s pair's bit; nothing is written to the device. */
  set(input: number, on: boolean): void {
    if (input < 0 || input >= 8) return;
    const bit = 1 << (input >> 1);
    const next = on ? this.mask | bit : this.mask & ~bit;
    if (next === this.mask) return;
    const serial = serialOf(this.session);
    this.cache = { serial, mask: next };
    saveLinks(this.storage, serial, next);
    for (const l of [...this.listeners]) l();
  }

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }
}

const stores = new WeakMap<Session, InputLinks>();

/** The session's links, one store per session. */
export function inputLinks(session: Session): InputLinks {
  let l = stores.get(session);
  if (!l) {
    l = new InputLinks(session);
    stores.set(session, l);
  }
  return l;
}

/**
 * An input's trim through the link: a linked pair is the core's pair call
 * (both sides, one undo step), a lone input the registry's `pre`. Answers
 * whether it went through.
 */
export function writeTrim(session: Session, input: number, db: number): Promise<boolean> {
  if (inputLinks(session).partner(input) >= 0) return session.change("inputPairPreamp", { input, db });
  return session.write("pre", { float: db }, [input]);
}

/**
 * Link `input`'s pair. `keep`: the side whose bands and trim are copied onto
 * the other first (the question's answer; null when the pair already
 * matches), one undo step ("Link Inputs"). The bit is set once that went
 * through; a refusal leaves the pair unlinked and the session's error says why.
 */
export async function linkPair(session: Session, input: number, keep: number | null): Promise<boolean> {
  const links = inputLinks(session);
  const s = session.state.snapshot?.state;
  if (!s || !pairAvailable(s, input)) return false;
  if (keep !== null) {
    const from = keep;
    const to = keep ^ 1;
    const ok = await session.group("Link Inputs", async () => {
      // As the device has them now (another console may have changed them).
      await session.refresh();
      const now = session.state.snapshot?.state ?? s;
      const n = now.maxBands || 10;
      for (let b = 0; b < n; b++) {
        const band = now.filters?.[from]?.[b];
        const there = now.filters?.[to]?.[b];
        if (!band || (there && sameBand(band, there))) continue;
        if (!(await session.change("filter", { channel: to, band: b, ...band }))) return false;
      }
      const db = now.inputPreampDb[from] ?? 0;
      if (Math.abs((now.inputPreampDb[to] ?? 0) - db) > 0.05 && !(await session.write("pre", { float: db }, [to]))) return false;
      return true;
    });
    if (!ok) return false;
  }
  links.set(input, true);
  return true;
}

// ---------------------------------------------------------------- sources

/** `in.source`: USB, S/PDIF, I2S, ADAT, S/PDIF 2..4. */
export const SOURCE_USB = 0;
export const SOURCE_SPDIF = 1;
export const SOURCE_I2S = 2;
export const SOURCE_ADAT = 3;
export const SOURCE_SPDIF2 = 4;
/** No ADAT pin chosen. */
const NO_PIN = 0xff;

/** Longest name the device keeps: 31 bytes in its 32-byte field. */
export const NAME_MAX = 31;

/**
 * The sources this device offers now, in the source menu's order (the
 * Console's): USB, S/PDIF and the further S/PDIF receivers that are on, I2S,
 * and ADAT once it is on with a pin (the firmware ignores it otherwise).
 */
export function sourceOptions(s: DspState): number[] {
  const out = [SOURCE_USB, SOURCE_SPDIF];
  const spdif = typeof s.spdifInputsEnabled === "number" ? s.spdifInputsEnabled : 1;
  for (let i = 1; i < 4; i++) if (spdif & (1 << i)) out.push(SOURCE_SPDIF2 + i - 1);
  if ((s.i2sInputChannels ?? 0) > 0) out.push(SOURCE_I2S);
  if (s.adatInputEnabled === true && typeof s.adatInputPin === "number" && s.adatInputPin !== NO_PIN) out.push(SOURCE_ADAT);
  return out;
}

/** A source's own title ("S/PDIF" numbered only while another receiver is on), as the Console's source menu. */
export function builtInTitle(s: DspState, source: number): string {
  const spdif = typeof s.spdifInputsEnabled === "number" ? s.spdifInputsEnabled : 1;
  const others = (spdif & 0x0e) !== 0;
  switch (source) {
    case SOURCE_USB:
      return "USB";
    case SOURCE_SPDIF:
      return others ? "S/PDIF 1" : "S/PDIF";
    case SOURCE_I2S:
      return "I2S";
    case SOURCE_ADAT:
      return "ADAT";
    case 4:
    case 5:
    case 6:
      return `S/PDIF ${source - 2}`;
    case 7:
      return "Audio link";
    default:
      return `Source ${source}`;
  }
}

/** A source as the pages show it: its user name, else its own title. */
export function sourceTitle(s: DspState, source: number, names: string[]): string {
  const n = (names[source] ?? "").trim();
  return n || builtInTitle(s, source);
}

/**
 * A typed name as the device keeps it and its front panel draws it:
 * printable ASCII, at most 31 characters, ends trimmed (Qt's `sanitizeName`,
 * Android's `DeviceNames.clean`). "" clears the name.
 */
export function cleanName(text: string): string {
  let out = "";
  for (const ch of text) {
    const c = ch.codePointAt(0) ?? 0;
    if (c >= 0x20 && c < 0x7f) out += ch;
    if (out.length >= NAME_MAX) break;
  }
  return out.trim();
}
