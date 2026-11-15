// WP WB6l: the control-surface editor's records, as Qt's CsController keeps
// them (WP QP12) and the Console's ControlSurfacesSettingsTab shows them.
//
// Every slot the device has is a record: what the device holds (`live`, from the
// core's snapshot) and what the page edits (`draft`). What a record shows, which
// values a row offers and what an edit makes of the record are the core's
// (`csDescribe`, `csEdit`: CG20); this file only decides which records a page
// lists, whether one is changed, and which operations an Apply or a Remove sends
// (`csApply`). No I/O, so the unit tests reach it.

import type { CsDescription, CsRecord, CsRow } from "../../core/protocol";

/** The component types (`CS_TYPE_*`, control_surfaces.h). */
export const TY = { None: 0, Button: 1, Switch: 2, Pot: 3, Encoder: 4, Led: 5, LedPwm: 6, Ir: 7, Display: 8, AuxOut: 9, AuxPwm: 10 } as const;

/** `CS_TARGET_OUTPUT_CH`: a new group's members are outputs. */
export const TARGET_OUTPUTS = 2;
/** `CS_STATUS_NO_IR`: a learn needs a running receiver. */
export const STATUS_NO_IR = 0x1e;
/** `CS_IR_LEARN_STATE_*`. */
export const LEARN_ARMED = 1;
export const LEARN_DONE = 2;
export const LEARN_TIMEOUT = 3;
/** `CS_MACRO_NONE`: no macro running. */
export const MACRO_NONE = 0xff;
/** `CS_DPAGE_ACTIVE`: a display page in use. */
export const PAGE_ACTIVE = 0x01;

export type RecordKind = "binding" | "ir" | "group" | "macro";
/** The editor's pages: wired controls, channel groups, macros, auxiliary outputs. */
export type CsPage = "controls" | "groups" | "macros" | "aux";

export interface CsRec {
  key: string;
  kind: RecordKind;
  slot: number;
  live: CsRecord;
  draft: CsRecord;
  /** A binding's name being typed, null when it is the device's. */
  nameEdit: string | null;
  /** What the last apply, edit or learn had to say. */
  message: string;
  isError: boolean;
  applying: boolean;
  /** The core's rows for the draft; null until described. */
  description: CsDescription | null;
  /** A macro's steps' rows. */
  steps: (CsDescription | null)[];
}

export function num(o: unknown, key: string): number {
  const v = o && typeof o === "object" ? (o as Record<string, unknown>)[key] : undefined;
  return typeof v === "number" && Number.isFinite(v) ? v : 0;
}

function arr(o: unknown, key: string): unknown[] {
  const v = o && typeof o === "object" ? (o as Record<string, unknown>)[key] : undefined;
  return Array.isArray(v) ? v : [];
}

function obj(v: unknown): CsRecord {
  return v && typeof v === "object" && !Array.isArray(v) ? (v as CsRecord) : {};
}

function str(o: unknown, key: string): string {
  const v = o && typeof o === "object" ? (o as Record<string, unknown>)[key] : undefined;
  return typeof v === "string" ? v : "";
}

/** Two JSON values alike, whatever the order of their keys. */
export function same(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((x, i) => same(x, b[i]));
  }
  if (a && b && typeof a === "object" && typeof b === "object") {
    const ka = Object.keys(a as object);
    const kb = Object.keys(b as object);
    if (ka.length !== kb.length) return false;
    return ka.every((k) => same((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
  }
  return false;
}

export function keyOf(kind: string, slot: number): string {
  return `${kind}/${slot}`;
}

/** The Console's name for a component type. */
export function typeName(t: number): string {
  switch (t) {
    case TY.None:
      return "None";
    case TY.Button:
      return "Push Button";
    case TY.Switch:
      return "Toggle Switch";
    case TY.Pot:
      return "Potentiometer / Fader";
    case TY.Encoder:
      return "Rotary Encoder";
    case TY.Led:
      return "Indicator LED";
    case TY.LedPwm:
      return "Dimmable LED";
    case TY.Ir:
      return "IR Remote";
    case TY.Display:
      return "Display";
    case TY.AuxOut:
      return "On/Off Output";
    case TY.AuxPwm:
      return "Dimmable Output";
    default:
      return `Type ${t}`;
  }
}

export function isAux(t: number): boolean {
  return t === TY.AuxOut || t === TY.AuxPwm;
}

export function protocolName(p: number): string {
  return p === 1 ? "NEC" : p === 2 ? "RC5" : p === 3 ? "RC6" : p === 4 ? "Generic" : "None";
}

/** Seconds in the most readable unit (the Console's fmtDelaySeconds). */
export function delayText(s: number): string {
  if (s < 60) return s < 10 ? `${Number(s.toPrecision(2))} s` : `${Math.round(s)} s`;
  const mins = s / 60;
  return `${mins === Math.round(mins) ? mins.toFixed(0) : mins.toFixed(1)} min`;
}

// ---- the snapshot ---------------------------------------------------------------

export function supported(snap: CsRecord | null): boolean {
  return snap?.supported === true;
}
export function slotCount(snap: CsRecord): number {
  return Math.min(num(snap, "maxBindings"), arr(snap, "bindings").length);
}
export function irCount(snap: CsRecord): number {
  return Math.min(num(snap, "maxIr"), arr(snap, "ir").length);
}
export function groupCount(snap: CsRecord): number {
  return Math.min(num(snap, "maxGroups"), arr(snap, "groups").length);
}
export function macroCount(snap: CsRecord): number {
  return Math.min(num(snap, "maxMacros"), arr(snap, "macros").length);
}
export function maxSteps(snap: CsRecord): number {
  return num(snap, "maxSteps");
}
export function typeCount(snap: CsRecord): number {
  return arr(snap, "types").length;
}
export function nameOf(snap: CsRecord, slot: number): string {
  const n = arr(snap, "names")[slot];
  return typeof n === "string" ? n : "";
}
export function status(snap: CsRecord): CsRecord {
  return obj(snap.status);
}
/** The device holds changes not saved to flash (the core's `dirty`: a real difference from the last clean read). */
export function deviceDirty(snap: CsRecord | null): boolean {
  return !!snap && supported(snap) && status(snap).dirty === true;
}
export function slotActive(snap: CsRecord, slot: number): boolean {
  return slot >= 0 && slot < 16 && (num(status(snap), "activeMask") & (1 << slot)) !== 0;
}
export function macroRunning(snap: CsRecord): number {
  const e = obj(snap.ext);
  return typeof e.macroRunning === "number" ? e.macroRunning : MACRO_NONE;
}

/** The device's record of `kind` in `slot`. A macro is its name and steps. */
export function liveFor(snap: CsRecord, kind: RecordKind, slot: number): CsRecord {
  if (kind === "macro") {
    const m = obj(arr(snap, "macros")[slot]);
    return { name: str(m, "name"), steps: arr(m, "steps") };
  }
  const table = kind === "binding" ? "bindings" : kind === "ir" ? "ir" : "groups";
  return obj(arr(snap, table)[slot]);
}

/**
 * Every slot of the device a record, kept from `prev`: a draft with no edits
 * follows the device (the Console re-seeds it), one being edited or applied stays.
 */
export function syncRecords(snap: CsRecord, prev: Map<string, CsRec>): Map<string, CsRec> {
  const out = new Map<string, CsRec>();
  const sync = (kind: RecordKind, count: number) => {
    for (let i = 0; i < count; i++) {
      const key = keyOf(kind, i);
      const live = liveFor(snap, kind, i);
      const was = prev.get(key);
      if (!was) {
        out.set(key, { key, kind, slot: i, live, draft: live, nameEdit: null, message: "", isError: false, applying: false, description: null, steps: [] });
        continue;
      }
      const follow = !was.applying && same(was.draft, was.live);
      const nameEdit = was.nameEdit !== null && was.nameEdit === nameOf(snap, i) ? null : was.nameEdit;
      out.set(key, { ...was, live, draft: follow ? live : was.draft, nameEdit });
    }
  };
  sync("binding", slotCount(snap));
  sync("ir", irCount(snap));
  sync("group", groupCount(snap));
  sync("macro", macroCount(snap));
  return out;
}

/** A remote button changed: one not learned yet has nothing to apply. */
export function irDirty(r: CsRec | undefined): boolean {
  if (!r || same(r.draft, r.live)) return false;
  return num(r.draft, "protocol") !== 0 || num(r.live, "protocol") !== 0;
}

export function anyIrDirty(recs: Map<string, CsRec>): boolean {
  for (const r of recs.values()) if (r.kind === "ir" && irDirty(r)) return true;
  return false;
}

/** Whether Apply has something to send for `r` (a receiver's: its remote buttons' too). */
export function isDirty(r: CsRec, recs: Map<string, CsRec>): boolean {
  if (r.kind === "binding") {
    if (!same(r.draft, r.live) || r.nameEdit !== null) return true;
    return num(r.draft, "type") === TY.Ir && anyIrDirty(recs);
  }
  if (r.kind === "ir") return irDirty(r);
  return !same(r.draft, r.live);
}

function macroOccupied(m: CsRecord): boolean {
  return str(m, "name") !== "" || arr(m, "steps").length > 0;
}

/** Whether a record is shown: configured on the device or being drafted. */
export function inUse(r: CsRec): boolean {
  if (r.kind === "binding") return num(r.draft, "type") !== TY.None || num(r.live, "type") !== TY.None;
  if (r.kind === "ir") return Object.values(r.draft).some((v) => typeof v === "number" && v !== 0) || num(r.live, "protocol") !== 0;
  if (r.kind === "group")
    return num(r.draft, "targetKind") !== 0 || str(r.draft, "name") !== "" || (num(r.live, "targetKind") !== 0 && num(r.live, "memberMask") !== 0);
  return macroOccupied(r.draft) || macroOccupied(r.live);
}

/** What the device holds of `r`, so Remove has something to clear. */
export function liveConfigured(r: CsRec): boolean {
  if (r.kind === "binding") return num(r.live, "type") !== TY.None;
  if (r.kind === "ir") return num(r.live, "protocol") !== 0;
  if (r.kind === "group") return num(r.live, "targetKind") !== 0 && num(r.live, "memberMask") !== 0;
  return macroOccupied(r.live);
}

function bindingRecs(snap: CsRecord, recs: Map<string, CsRec>): CsRec[] {
  const out: CsRec[] = [];
  for (let s = 0; s < slotCount(snap); s++) {
    const r = recs.get(keyOf("binding", s));
    if (r) out.push(r);
  }
  return out;
}

/** The binding slot of the IR receiver, drafted or live; -1 for none. */
export function receiverSlot(snap: CsRecord, recs: Map<string, CsRec>): number {
  const r = bindingRecs(snap, recs).find((b) => num(b.draft, "type") === TY.Ir || num(b.live, "type") === TY.Ir);
  return r ? r.slot : -1;
}

/** The binding slot of a running display; -1 for none. */
export function displaySlot(snap: CsRecord, recs: Map<string, CsRec>): number {
  const r = bindingRecs(snap, recs).find((b) => num(b.live, "type") === TY.Display);
  return r ? r.slot : -1;
}

/** The records page `page` lists, in slot order. */
export function keysFor(page: CsPage | "remotes", snap: CsRecord, recs: Map<string, CsRec>): string[] {
  const list = (kind: RecordKind, count: number, keep: (r: CsRec) => boolean) => {
    const out: string[] = [];
    for (let i = 0; i < count; i++) {
      const r = recs.get(keyOf(kind, i));
      if (r && inUse(r) && keep(r)) out.push(r.key);
    }
    return out;
  };
  if (page === "controls" || page === "aux") {
    const aux = page === "aux";
    return list("binding", slotCount(snap), (r) => (isAux(num(r.draft, "type")) || isAux(num(r.live, "type"))) === aux);
  }
  if (page === "groups") return list("group", groupCount(snap), () => true);
  if (page === "macros") return list("macro", macroCount(snap), () => true);
  return list("ir", irCount(snap), () => true);
}

/** Which pages the device has (Qt's updatePages): groups, macros and aux outputs only where the firmware has them. */
export function pagesOf(snap: CsRecord | null): CsPage[] {
  if (!snap || !supported(snap)) return ["controls"];
  const out: CsPage[] = ["controls"];
  if (groupCount(snap) > 0) out.push("groups");
  if (macroCount(snap) > 0) out.push("macros");
  if (num(snap, "capsVersion") >= 18 && typeCount(snap) > TY.AuxPwm) out.push("aux");
  return out;
}

export const PAGE_LABELS: Record<CsPage, string> = { controls: "Controls", groups: "Groups", macros: "Macros", aux: "Auxiliary Outputs" };

/** The types Add offers on `page`: one IR receiver, one display, and aux types only on the aux page. */
export function addableTypes(page: CsPage, snap: CsRecord, recs: Map<string, CsRec>): { type: number; name: string }[] {
  const aux = page === "aux";
  const displayDrafted = bindingRecs(snap, recs).some((b) => num(b.draft, "type") === TY.Display);
  const out: { type: number; name: string }[] = [];
  for (let t = 1; t < typeCount(snap); t++) {
    if (isAux(t) !== aux) continue;
    if (t === TY.Ir && (receiverSlot(snap, recs) >= 0 || irCount(snap) === 0)) continue;
    if (t === TY.Display && displayDrafted) continue;
    out.push({ type: t, name: typeName(t) });
  }
  return out;
}

/** The first free record of `kind`, or null when every slot is in use. */
export function freeSlot(kind: RecordKind, snap: CsRecord, recs: Map<string, CsRec>): number | null {
  const count = kind === "binding" ? slotCount(snap) : kind === "ir" ? irCount(snap) : kind === "group" ? groupCount(snap) : macroCount(snap);
  for (let i = 0; i < count; i++) {
    const r = recs.get(keyOf(kind, i));
    if (r && !inUse(r)) return i;
  }
  return null;
}

// ---- operations -------------------------------------------------------------------

/** What Apply sends for `r`: a binding and its name, a receiver's remote buttons with it, or the record. */
export function applyOps(r: CsRec, recs: Map<string, CsRec>): { ops: CsRecord[]; irKeys: string[] } {
  const ops: CsRecord[] = [];
  const irKeys: string[] = [];
  if (r.kind === "binding") {
    if (!same(r.draft, r.live)) ops.push({ op: "binding", index: r.slot, binding: r.draft });
    if (r.nameEdit !== null) ops.push({ op: "name", index: r.slot, name: r.nameEdit });
    if (num(r.draft, "type") === TY.Ir)
      for (const ir of recs.values())
        if (ir.kind === "ir" && irDirty(ir)) {
          irKeys.push(ir.key);
          ops.push({ op: "ir", index: ir.slot, command: ir.draft });
        }
  } else if (r.kind === "group") ops.push({ op: "group", index: r.slot, group: r.draft });
  else if (r.kind === "macro") ops.push({ op: "macro", index: r.slot, macro: r.draft });
  else ops.push({ op: "ir", index: r.slot, command: r.draft });
  return { ops, irKeys };
}

/** What Remove sends: an empty record (and no name) where the device holds one; nothing for a draft never applied. */
export function removeOps(r: CsRec, snap: CsRecord): CsRecord[] {
  if (r.kind === "binding") {
    const ops: CsRecord[] = [];
    if (num(r.live, "type") !== TY.None) ops.push({ op: "binding", index: r.slot, binding: {} });
    if (nameOf(snap, r.slot) !== "") ops.push({ op: "name", index: r.slot, name: "" });
    return ops;
  }
  if (!liveConfigured(r)) return [];
  const field = r.kind === "ir" ? "command" : r.kind === "group" ? "group" : "macro";
  return [{ op: r.kind, index: r.slot, [field]: {} }];
}

/** A name as the device keeps it: printable ASCII, at most 31 characters. */
export function cleanName(s: string): string {
  return s.replace(/[^\x20-\x7e]/g, "").slice(0, 31);
}

// ---- what a card shows ------------------------------------------------------------

export function rowsOf(d: CsDescription | null, dropType = false): CsRow[] {
  if (!d || !d.ok) return [];
  return d.rows.filter((r) => !(dropType && r.id === "type"));
}

export function summaryOf(d: CsDescription | null): string {
  return d && d.ok ? d.summary : "";
}

export function rowValue(d: CsDescription | null, id: string): number {
  const r = rowsOf(d).find((x) => x.id === id);
  return typeof r?.value === "number" ? r.value : 0;
}

/** A remote button's learned code as the Console shows it. */
export function codeText(draft: CsRecord): string {
  const p = num(draft, "protocol");
  if (p === 0) return "Not learned";
  return `${protocolName(p)} 0x${(num(draft, "code") >>> 0).toString(16).toUpperCase().padStart(8, "0")}`;
}

/** A macro's line: running, its steps and their delays. */
export function macroSummary(r: CsRec, snap: CsRecord): string {
  const steps = arr(r.draft, "steps");
  if (macroRunning(snap) === r.slot) return `Running - step ${num(obj(snap.ext), "macroStep") + 1} of ${arr(r.live, "steps").length}.`;
  if (steps.length === 0) return "No steps yet.";
  const total = r.steps.reduce((t, d) => t + rowValue(d, "preDelay"), 0);
  const count = steps.length === 1 ? "1 step" : `${steps.length} steps`;
  return total > 0 ? `${count}, ${delayText(total)} total` : count;
}

/** A binding's pill: Pending while changed, else Active or Inactive. */
export function pillOf(r: CsRec, snap: CsRecord, recs: Map<string, CsRec>): { text: string; good: boolean } {
  if (isDirty(r, recs)) return { text: "Pending", good: false };
  return slotActive(snap, r.slot) ? { text: "Active", good: true } : { text: "Inactive", good: false };
}

/** The display panel's state, as the Console words it. */
export function panelState(snap: CsRecord): { text: string; good: boolean; error: boolean; detail: string } {
  const st = obj(obj(snap.display).status);
  const init = num(st, "init");
  const text = init === 2 ? "Running" : init === 1 ? "Starting up" : init === 3 ? "Not responding" : "Not started";
  const naks = num(st, "naks");
  const detail =
    naks === 1
      ? "1 I2C error so far. Check wiring, pull-up resistors, and the address."
      : naks > 1
        ? `${naks} I2C errors so far. Check wiring, pull-up resistors, and the address.`
        : "Reported by the device.";
  return { text, good: init === 2, error: init === 3, detail };
}

/** The display pages in use, by index. */
export function activePages(snap: CsRecord): { index: number; page: CsRecord; shown: boolean }[] {
  const d = obj(snap.display);
  const pages = arr(d, "pages");
  const max = Math.min(num(d, "maxPages"), pages.length);
  const current = num(obj(d.status), "page");
  const out: { index: number; page: CsRecord; shown: boolean }[] = [];
  for (let i = 0; i < max; i++) {
    const p = obj(pages[i]);
    if (num(p, "flags") & PAGE_ACTIVE) out.push({ index: i, page: p, shown: current === i });
  }
  return out;
}

export function freePage(snap: CsRecord): number | null {
  const d = obj(snap.display);
  const pages = arr(d, "pages");
  const max = Math.min(num(d, "maxPages"), pages.length);
  for (let i = 0; i < max; i++) if (!(num(obj(pages[i]), "flags") & PAGE_ACTIVE)) return i;
  return null;
}

export function displayConfig(snap: CsRecord): CsRecord {
  return obj(obj(snap.display).config);
}

/** An aux output's live switch and level (percent). */
export function auxOf(snap: CsRecord, slot: number): { on: boolean; level: number } {
  const st = arr(snap, "auxState")[slot];
  const lv = arr(snap, "auxLevel")[slot];
  return { on: st === true || (typeof st === "number" && st !== 0), level: typeof lv === "number" ? lv / 256 : 0 };
}

/** The words for a refused operation: the core's error, else its status code's sentence. */
export function failureText(reply: { error?: string; status?: number }, texts: Record<string, string>): string {
  if (reply.error) return reply.error;
  const t = texts[String(reply.status ?? 0)];
  return t || "Failed to apply the binding";
}
