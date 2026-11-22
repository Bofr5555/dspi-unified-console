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
import type { SystemModel, Unit } from "../model";
import type { InstallationDraft, InstallationEdit, InstallationIssue, InstallationMethods, InstallationRole, InstallationView } from "./protocol";

export type InstallationCall = <M extends keyof InstallationMethods>(
  method: M,
  params: InstallationMethods[M]["params"],
) => Promise<InstallationMethods[M]["result"]>;

export const ROLES: { id: InstallationRole; label: string }[] = [
  { id: "receiver", label: "Receiver" },
  { id: "repeater", label: "Repeater" },
  { id: "spdif_follower", label: "S/PDIF follower" },
  { id: "usb_direct", label: "USB-direct" },
];

/** A unit's role as the model holds it. */
export function roleOf(u: Unit): InstallationRole {
  if (u.transport === "usb_direct") return "usb_direct";
  if (u.transport === "spdif") return "spdif_follower";
  return u.repeater ? "repeater" : "receiver";
}

export function roleWord(r: InstallationRole): string {
  return ROLES.find((x) => x.id === r)?.label ?? r;
}

/** Whether a unit of role `r` is fed by an upstream element. */
export function needsUpstream(r: InstallationRole): boolean {
  return r !== "usb_direct";
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
export function upstreamChoices(model: SystemModel, r: InstallationRole, self?: string): { id: string; name: string }[] {
  if (r === "usb_direct") return [];
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
  const ids = new Set<string>([...model.units, ...model.rooms, ...model.zones].map((x) => x.id));
  return issues.filter((i) => !i.subject || !ids.has(i.subject));
}

/** A unit seen but not held, joining: the `addUnit` edit. */
export function joinEdit(serial: string, f: { name: string; room: string; role: InstallationRole; upstream: string }): InstallationEdit {
  const name = f.name.trim();
  return {
    op: "addUnit",
    serial,
    ...(name ? { name } : {}),
    room: f.room || null,
    role: f.role,
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
