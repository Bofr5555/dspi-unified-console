// WP INSTW: the Installation tab's form model (MPLAN 1.4). The logic is the core's
// (`installation*` calls, WP INST0): this keeps the page's draft and the edits made on it,
// asks the core to validate it and say what saving changes, and saves it. No DOM here:
// InstallationTab.tsx draws it.
//
// On `conflict` (someone saved the installation meanwhile) the draft is rebuilt from a fresh
// view: the hub has no 3-way merge, so copying the new revision into the old draft would
// silently undo the other save. The edits made are kept aside, and may be made again on the
// fresh draft, where the core judges each again.

import { CoreCallError } from "../../../core/protocol";
import type { SceneAction as SceneActionDef, SystemModel, Unit } from "../model";
import type {
  InstallationDraft,
  InstallationEdit,
  InstallationIssue,
  InstallationMethods,
  InstallationProgrammeKind,
  InstallationRole,
  InstallationView,
} from "./protocol";

export type InstallationCall = <M extends keyof InstallationMethods>(
  method: M,
  params: InstallationMethods[M]["params"],
) => Promise<InstallationMethods[M]["result"]>;

// -- WP WEBM3 -- (RB C-2, as Qt's INSTQ) an S/PDIF source is a role on the form: the core's
// S/PDIF follower marked as a source (`setSpdifSource`), fed by nothing the model holds.
/** A unit's role on the form: the core's, or an S/PDIF source. */
export type UnitRole = InstallationRole | "spdif_source";

export const ROLES: { id: UnitRole; label: string }[] = [
  { id: "receiver", label: "Receiver" },
  { id: "repeater", label: "Repeater" },
  { id: "spdif_follower", label: "S/PDIF follower" },
  { id: "spdif_source", label: "S/PDIF source" }, // -- WP WEBM3 --
  { id: "usb_direct", label: "USB-direct" },
];

/** A unit's role as the model holds it. */
export function roleOf(u: Unit): UnitRole {
  if (u.transport === "usb_direct") return "usb_direct";
  if (u.transport === "spdif") return u.spdifSource ? "spdif_source" : "spdif_follower"; // -- WP WEBM3 --
  return u.repeater ? "repeater" : "receiver";
}

export function roleWord(r: UnitRole): string {
  return ROLES.find((x) => x.id === r)?.label ?? r;
}

/** Whether a unit of role `r` is fed by an upstream element. */
export function needsUpstream(r: UnitRole): boolean {
  return r !== "usb_direct" && r !== "spdif_source"; // -- WP WEBM3 --
}

// -- WP WEBM3 --
/** A new S/PDIF source's time-base slot until the person chooses (outputs 7-8, as Qt's). */
export const NEW_SOURCE_MASK = 0b1000;

/** The core's role for a form role. */
export function coreRole(r: UnitRole): InstallationRole {
  return r === "spdif_source" ? "spdif_follower" : r;
}

/**
 * The edits that give unit `u` the role `to`, as one step: a source by its transport stops being
 * one first; a new source keeps the slots it had, else the default. None when it has that role.
 */
export function roleEdits(u: Unit, to: UnitRole): InstallationEdit[] {
  const was = roleOf(u);
  if (to === was) return [];
  const edits: InstallationEdit[] = [];
  if (was === "spdif_source") edits.push({ op: "setSpdifSource", id: u.id, txMask: 0 });
  edits.push({ op: "setUnitRole", id: u.id, role: coreRole(to) });
  if (to === "spdif_source") edits.push({ op: "setSpdifSource", id: u.id, txMask: u.spdifTxMask || NEW_SOURCE_MASK });
  return edits;
}

/** The S/PDIF output slots (0 to 3) and their outputs, "1-2" to "7-8". */
export const SPDIF_SLOTS = [0, 1, 2, 3].map((slot) => ({ slot, outputs: `${2 * slot + 1}-${2 * slot + 2}` }));

/** A source's slot `slot` turned on or off: the edit, or why not (a source sends on one slot at least). */
export function slotEdit(u: Unit, slot: number, on: boolean): InstallationEdit | string {
  if (!u.spdifSource || slot < 0 || slot > 3) return "Only an S/PDIF source sends a time base.";
  const was = u.spdifTxMask ?? 0;
  const mask = on ? was | (1 << slot) : was & ~(1 << slot);
  if (!mask) return "An S/PDIF source sends its time base on one pair of outputs at least.";
  return { op: "setSpdifSource", id: u.id, txMask: mask };
}

/** `list` with `id` in it (`on`) or not, in its order. */
export function withMember(list: readonly string[], id: string, on: boolean): string[] {
  return on ? (list.includes(id) ? [...list] : [...list, id]) : list.filter((x) => x !== id);
}

/**
 * Text from the hub (names, the core's sentences that hold them) as the page shows it: without
 * control and format characters (U+202E and the other bidi overrides, zero-width ones), so a
 * name cannot reorder or hide the words of a confirmation around it.
 */
export function cleanText(s: string): string {
  return s.replace(/[\p{Cc}\p{Cf}]/gu, "");
}

/** What may feed a unit of role `r` in `model` (never the unit itself): id and name. */
export function upstreamChoices(model: SystemModel, r: UnitRole, self?: string): { id: string; name: string }[] {
  if (!needsUpstream(r)) return []; // -- WP WEBM3 -- an S/PDIF source too
  if (r === "spdif_follower") return model.units.filter((u) => u.spdifSource && u.id !== self).map((u) => ({ id: u.id, name: u.name || u.id }));
  return [
    ...model.senders.map((s) => ({ id: s.id, name: s.name || s.id })),
    ...model.fanOuts.map((f) => ({ id: f.id, name: f.name || f.id })),
    ...model.units.filter((u) => u.id !== self && roleOf(u) === "repeater").map((u) => ({ id: u.id, name: u.name || u.id })),
  ];
}

/** Where on the form an issue is shown: the element's field, the element, or the top. */
export interface IssuePlace {
  subject: string;
  field: "upstream" | "room" | "rooms" | "";
}

export function issuePlace(i: InstallationIssue): IssuePlace {
  if (!i.subject) return { subject: "", field: "" };
  const rest = i.message.slice(i.message.indexOf(":") + 1).trim();
  if (/^upstream\b/.test(rest)) return { subject: i.subject, field: "upstream" };
  if (/^no room\b/.test(rest)) return { subject: i.subject, field: i.message.startsWith("zone ") ? "rooms" : "room" };
  return { subject: i.subject, field: "" };
}

/** The issues for element `id`'s `field` ("" the element's own). */
export function issuesAt(issues: InstallationIssue[], id: string, field: IssuePlace["field"]): InstallationIssue[] {
  return issues.filter((i) => {
    const p = issuePlace(i);
    return p.subject === id && p.field === field;
  });
}

/** The issues no element of `model` shows: the installation's own, or about an element it no longer holds. */
export function generalIssues(issues: InstallationIssue[], model: SystemModel): InstallationIssue[] {
  const ids = new Set<string>([...model.units, ...model.rooms, ...model.zones, ...model.groups /* -- WP WEBM3 -- shown at the group */].map((x) => x.id));
  return issues.filter((i) => !i.subject || !ids.has(i.subject));
}

/** A unit seen but not held, joining: the `addUnit` edit. */
export function joinEdit(serial: string, f: { name: string; room: string; role: UnitRole; upstream: string }): InstallationEdit {
  const name = f.name.trim();
  return {
    op: "addUnit",
    serial,
    ...(name ? { name } : {}),
    room: f.room || null,
    role: coreRole(f.role), // -- WP WEBM3 -- a source joins as a follower, then is marked
    upstream: needsUpstream(f.role) ? f.upstream || null : null,
  };
}

export interface FormState {
  /** Reading, or nothing read yet. */
  loading: boolean;
  view: InstallationView | null;
  /** The view's draft with `edits` made on it. */
  draft: InstallationDraft | null;
  /** The edits made since the view was read, in order. */
  edits: InstallationEdit[];
  issues: InstallationIssue[];
  /** What saving would change, cleaned for display. */
  changes: string[];
  /** The last refusal or failure, "" when none. */
  error: string;
  /** The installation changed on the hub: the draft was rebuilt, these edits were not saved. */
  lost: InstallationEdit[] | null;
  saving: boolean;
  /** The last save: its hub warnings. */
  saved: { revision: number; warnings: string[] } | null;
}

const EMPTY: FormState = { loading: true, view: null, draft: null, edits: [], issues: [], changes: [], error: "", lost: null, saving: false, saved: null };

export class InstallationForm {
  state: FormState = EMPTY;
  private readonly listeners = new Set<() => void>();

  constructor(private readonly call: InstallationCall) {}

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Whether the form may change anything: the role and the session allow it. */
  editable(readOnly = false): boolean {
    return !!this.state.view?.mayEdit && !readOnly;
  }

  get dirty(): boolean {
    return this.state.edits.length > 0;
  }

  /** Read the hub's installation again; the draft starts over from it. */
  async load(): Promise<void> {
    this.set({ loading: true });
    try {
      const view = await this.call("installationView", undefined);
      this.set({ loading: false, view, draft: view.draft, edits: [], error: "" });
      await this.judge(view.draft);
    } catch (e) {
      this.set({ loading: false, error: message(e) });
    }
  }

  /** Make `edits` on the draft, all or none. False (with the core's reason) when refused. */
  async edit(...edits: InstallationEdit[]): Promise<boolean> {
    const draft = this.state.draft;
    if (!draft || !edits.length) return false;
    try {
      const next = await this.call("installationDraftEdit", { draft, edits });
      this.set({ draft: next, edits: [...this.state.edits, ...edits], error: "", saved: null });
      await this.judge(next);
      return true;
    } catch (e) {
      this.set({ error: message(e) });
      return false;
    }
  }

  // -- WP WEBM3 --
  /**
   * `first` and the edits `rest` makes of the draft it gives, as one step (all or none): for an
   * edit after one that makes an id the core chooses (a new S/PDIF source's mark). The core makes
   * the same id from the same draft, so `first` is tried once alone to learn it.
   */
  async editThen(first: InstallationEdit, rest: (next: InstallationDraft) => InstallationEdit[]): Promise<boolean> {
    const draft = this.state.draft;
    if (!draft) return false;
    let more: InstallationEdit[];
    try {
      more = rest(await this.call("installationDraftEdit", { draft, edits: [first] }));
    } catch (e) {
      this.set({ error: message(e) });
      return false;
    }
    return this.edit(first, ...more);
  }

  /** Say why an edit was not made (one the page refuses before the core is asked). */
  refuse(why: string): void {
    this.set({ error: why });
  }

  /** Forget the edits: the draft is the view's again. */
  async revert(): Promise<void> {
    const view = this.state.view;
    if (!view) return;
    this.set({ draft: view.draft, edits: [], error: "", lost: null });
    await this.judge(view.draft);
  }

  /** Whether Save is offered: edits the hub would take. */
  get savable(): boolean {
    return this.dirty && !this.state.issues.length && !this.state.saving && !this.state.loading;
  }

  /**
   * Save the draft (one `system.set` at its revision). `ok`: read again. `conflict`: someone
   * saved meanwhile; the draft is rebuilt from a fresh view and the edits kept in `lost`.
   */
  async save(): Promise<"ok" | "conflict" | "failed"> {
    const draft = this.state.draft;
    if (!draft || !this.savable) return "failed";
    this.set({ saving: true, error: "" });
    try {
      const saved = await this.call("installationSave", { draft });
      await this.load();
      this.set({ saving: false, saved: { revision: saved.revision, warnings: saved.warnings.map(cleanText) }, lost: null });
      return "ok";
    } catch (e) {
      if (e instanceof CoreCallError && e.code === "conflict") {
        const lost = this.state.edits;
        await this.load();
        this.set({ saving: false, lost });
        return "conflict";
      }
      this.set({ saving: false, error: message(e) });
      return "failed";
    }
  }

  /** After a conflict: the edits that were not saved, made again on the fresh draft. */
  async remake(): Promise<boolean> {
    const lost = this.state.lost;
    if (!lost?.length) return false;
    const done = await this.edit(...lost);
    if (done) this.set({ lost: null });
    return done;
  }

  dismissLost(): void {
    this.set({ lost: null });
  }

  private async judge(draft: InstallationDraft): Promise<void> {
    try {
      const [issues, changes] = await Promise.all([this.call("installationValidate", { draft }), this.call("installationDiff", { draft })]);
      if (this.state.draft !== draft) return;
      this.set({ issues: issues.map((i) => ({ subject: i.subject, message: cleanText(i.message) })), changes: changes.map(cleanText) });
    } catch (e) {
      this.set({ error: message(e) });
    }
  }

  private set(patch: Partial<FormState>): void {
    this.state = { ...this.state, ...patch };
    for (const l of this.listeners) l();
  }
}

function message(e: unknown): string {
  // The core's refusal of an edit names it ("invalid argument: edit 2: ..."): the edit is what is said.
  const m = cleanText(e instanceof Error ? e.message : String(e)).replace(/^invalid argument: /i, "");
  return m ? m[0]!.toUpperCase() + m.slice(1) : m;
}

// -- WP FEATUI -- FEAT1's sources, slots, programme sources and scenes on the form (as Qt's
// InstallationController). Channels and inputs are numbered from 0, as the model and the core's
// sentences ("slots 0 1 - - …") number them.

/** A unit's matrix inputs that slots fill (`UNIT_INPUTS`); 255 is a silent slot. */
export const UNIT_INPUTS = 8;
export const SILENT = 255;

/** "8 9", "8-11", "0, 2" as numbers below `limit`; null for anything else or nothing. */
export function parseNumbers(text: string, limit: number): number[] | null {
  const out: number[] = [];
  for (const part of text.split(/[\s,;]+/).filter((p) => p.length > 0)) {
    const ends = part.split("-");
    if (ends.length > 2 || ends.some((e) => !/^\d+$/.test(e))) return null;
    const from = Number(ends[0]);
    const to = ends.length === 2 ? Number(ends[1]) : from;
    if (to < from || to >= limit) return null;
    for (let n = from; n <= to; n++) out.push(n);
  }
  return out.length ? out : null;
}

export function numbersText(list: readonly number[] | null | undefined): string {
  return (list ?? []).join(" ");
}

/** A unit's slots as typed ("8 9 - -"): 8 entries, 255 for "-"; [] for nothing (its own); null when not slots. */
export function parseSlots(text: string): number[] | null {
  const parts = text.trim().split(/[\s,;]+/).filter((p) => p.length > 0);
  if (!parts.length) return [];
  if (parts.length > UNIT_INPUTS) return null;
  const out: number[] = [];
  for (const p of parts) {
    if (p === "-") out.push(SILENT);
    else if (/^\d+$/.test(p) && Number(p) < 64) out.push(Number(p));
    else return null;
  }
  while (out.length < UNIT_INPUTS) out.push(SILENT);
  return out;
}

export function slotsText(slots: readonly number[] | null | undefined): string {
  return (slots ?? []).map((n) => (n === SILENT ? "-" : String(n))).join(" ");
}

/** A unit on the audio link (a zone's source routes only those). */
export function onAlink(u: Unit): boolean {
  return !u.transport || u.transport === "alink";
}

/** The zone whose source owns unit `u`'s slots, and that source; null when they are its own. */
export function slotsOwner(model: SystemModel, u: Unit): { zone: string; source: string } | null {
  if (!onAlink(u) || !u.room) return null;
  const z = model.zones.find((z) => z.source && z.rooms.includes(u.room ?? ""));
  if (!z || !z.source) return null;
  const s = model.sources.find((x) => x.id === z.source);
  return { zone: cleanText(z.name || z.id), source: cleanText(s?.name || z.source) };
}

/** The names of the audio link units in a zone's rooms: what its source routes. */
export function zoneUnits(model: SystemModel, zone: string): string[] {
  const z = model.zones.find((x) => x.id === zone);
  if (!z) return [];
  return model.units.filter((u) => onAlink(u) && z.rooms.includes(u.room ?? "")).map((u) => cleanText(u.name || u.id));
}

/** A unit whose slots, as the hub keeps them, a zone's source replaces. */
export interface Rerouted {
  id: string;
  name: string;
  zone: string;
  source: string;
  /** It had slots of its own (not left to the unit). */
  handMade: boolean;
}

/** The units whose slots the draft's zone sources change: the notice before Save. */
export function rerouted(draft: InstallationDraft): Rerouted[] {
  const out: Rerouted[] = [];
  for (const u of draft.model.units) {
    const was = draft.base.units.find((b) => b.id === u.id);
    if (!was || JSON.stringify(was.slots ?? null) === JSON.stringify(u.slots ?? null)) continue;
    const owner = slotsOwner(draft.model, u);
    if (!owner) continue;
    out.push({ id: u.id, name: cleanText(u.name || u.id), zone: owner.zone, source: owner.source, handMade: Array.isArray(was.slots) });
  }
  return out;
}

export const PROGRAMME_KINDS: { id: InstallationProgrammeKind; label: string }[] = [
  { id: "computer_usb", label: "Computer over USB" },
  { id: "spdif", label: "S/PDIF input" },
  { id: "analogue", label: "Analogue input" },
  { id: "alink_sender", label: "Into the audio link sender" },
];

/** What a programme source of `kind` may feed: units, or for the sender kind the senders. */
export function programmeFeeds(model: SystemModel, kind: InstallationProgrammeKind): { id: string; name: string }[] {
  return kind === "alink_sender"
    ? model.senders.map((s) => ({ id: s.id, name: cleanText(s.name || s.id) }))
    : model.units.map((u) => ({ id: u.id, name: cleanText(u.name || u.id) }));
}

/** The highest input + 1 a programme source of `kind` may name. */
export function programmeLimit(kind: InstallationProgrammeKind): number {
  return kind === "alink_sender" ? 64 : UNIT_INPUTS;
}

/** What a scene's action targets, in words. */
export function targetWord(model: SystemModel, target: string): string {
  const at = target.indexOf(":");
  if (at < 0) return "Everywhere";
  const id = target.slice(at + 1);
  const all = [...model.rooms, ...model.zones, ...model.groups, ...model.units];
  return cleanText(all.find((e) => e.id === id)?.name || id);
}

/** A scene's action in words. */
export function actionText(model: SystemModel, a: SceneActionDef): string {
  const where = targetWord(model, a.target);
  switch (a.op) {
    case "mute":
      return `Mute ${where}`;
    case "unmute":
      return `Unmute ${where}`;
    case "master_volume":
      return `${where}: master volume ${((a.value ?? 0) / 100).toFixed(1)} dB`;
    case "source": {
      const s = model.sources.find((x) => x.id === a.source);
      return `Switch ${where} to source ${cleanText(s?.name || a.source || "")}`;
    }
    default:
      return `${where}: ${cleanText(a.op)}`;
  }
}

/** The scene targets the form offers: everywhere, the zones, rooms and groups. */
export function sceneTargets(model: SystemModel): { id: string; name: string }[] {
  return [
    { id: "all", name: "Everywhere" },
    ...model.zones.map((z) => ({ id: `zone:${z.id}`, name: cleanText(z.name || z.id) })),
    ...model.rooms.map((r) => ({ id: `room:${r.id}`, name: cleanText(r.name || r.id) })),
    ...model.groups.map((g) => ({ id: `group:${g.id}`, name: cleanText(g.name || g.id) })),
  ];
}
