// WP P1WB1: snapshots, A/B compare, safe apply with rollback, import and export on the web.
//
// One desk per session (as the System feed is), so the scene bar's A/B toggle, the countdown
// bar and the screens share it. Everything that decides is the core's (P1SN, P1AB, P1DF, P1SA,
// P1OF): the desk only holds the documents the page has, calls the core and keeps the answers.
//
// The web has no file system (OD-P1b): a snapshot is a `.dspipreset` document the page
// downloads, and one the user opens from a file is held for this visit. A recall over DSPi
// Link to a hub with the guard is a guarded apply (OD-P1g, p1cfix): `applyGuardDefault` says
// so, and then the plain recall goes through `applyGuardedSnapshot`; on USB the rollback is the
// user's choice. While a guard waits, the A/B calls are refused by the core: the toggle is off
// then, and a guard that ends other than kept ends the compare (the core's; the desk reads
// the compare again). The countdown runs on the page's poll (`applyPoll`): no timer in the core.

import {
  CoreCallError,
  IMPORT_BY_BLOCK,
  type AbCompareRecord,
  type ConfigDiff,
  type GuardedApplyRecord,
  type ImportReport,
  type NoticeRecord, // -- WP REVERTW --
} from "../../core/protocol";
import { noticeRow, revertNotice } from "../../ui/texts"; // -- WP REVERTW --
import type { HubSnapshotRecallRecord } from "../../core/hub-settings-protocol"; // -- WP P2WB3 --
import type { Session } from "../../ui/session";
// -- WP WB6K -- the document's input pair-link mask, both ways
import { inputLinks } from "../inputs/model";

/** A document the page holds for this visit: captured here, or opened from a file. */
export interface HeldDoc {
  id: number;
  name: string;
  note: string;
  /** `meta.kind`: `snapshot`, `preset`, `template`. */
  kind: string;
  /** `meta.savedUtc`, "" when it has none. */
  savedUtc: string;
  fileName: string;
  text: string;
  origin: "captured" | "file";
}

/** What the last action did, shown until the next (never a modal: a success is the change itself). */
export interface Outcome {
  title: string;
  lines: string[];
  notes: string[];
  tone: "info" | "warn" | "error";
}

export interface DeskState {
  docs: HeldDoc[];
  ab: AbCompareRecord;
  /** The last guarded apply this page knows of; `waiting` shows the countdown. */
  guard: GuardedApplyRecord | null;
  /** Imports and recalls are guarded by default (DSPi Link to a hub with the guard). */
  guardDefault: boolean;
  /** What is running, "" when nothing. One action at a time. */
  busy: string;
  outcome: Outcome | null;
  /** The A/B pair's differences, once asked for. */
  abDiff: ConfigDiff | null;
  /** How long a guarded apply waits for Keep before it reverts, s. */
  timeoutS: number;
  // -- WP P1DCFIX --
  /** The core's countdown sentence (notice `guard.waiting`): `{s}` is the seconds left. */
  waitingText: string;
  /** -- WP P1WB2 -- beside the countdown when the hub runs the guard too: the core's row `guard.hub_backstop`. */
  backstopText: string;
}

/** The rollback's default, seconds (OD-P1g). */
export const DEFAULT_TIMEOUT_S = 30;
/** How often the page drives a waiting guard, ms. */
export const GUARD_POLL_MS = 250;

// -- WP P1DCFIX --
/** The core's notice key for the countdown (P1NT's table, F7). */
export const GUARD_WAITING_KEY = "guard.waiting";
/** The same words, until the core has answered (the core's row is the one shown). */
// -- WP P1WB2 -- the hub's backstop beside the countdown (the core's row, P1NT)
export const GUARD_BACKSTOP_KEY = "guard.hub_backstop";
const BACKSTOP_FALLBACK = "The hub puts them back too if this console goes away.";
const WAITING_FALLBACK = "Keep these settings? They go back in {s} s unless you press Keep.";

/** The countdown sentence with the seconds in, split around them (the page marks the number). */
export function waitingWords(text: string, seconds: number): { before: string; seconds: number; after: string } {
  const at = text.indexOf("{s}");
  if (at < 0) return { before: `${text} `, seconds, after: " s" };
  return { before: text.slice(0, at), seconds, after: text.slice(at + 3) };
}

/** A guard the bar offers: one waiting for Keep, or one lost with the device open again (Revert stays offered). */
export function guardOffered(g: GuardedApplyRecord | null): boolean {
  // -- WP REVERTW -- a lost guard whose end the hub no longer knows has no Revert: its outcome says so
  return !!g && (g.phase === "waiting" || (g.phase === "lost" && !g.revertNotice));
}

// -- WP REVERTW --
/** How a guard's end reads: a revert that was not fully successful leads with its notice (owner 2026-11-19). */
export function guardOutcome(g: GuardedApplyRecord, row: (key: string) => NoticeRecord | null = noticeRow): Outcome {
  const n = revertNotice(g.revertNotice, row);
  if (n) {
    return { title: n.title, lines: [g.message || n.reason, n.action].filter(Boolean), notes: g.notes, tone: "error" };
  }
  const tone = g.phase === "kept" ? "info" : g.phase === "reverted" ? "warn" : "error";
  return { title: g.label || "Safe apply", lines: [g.message].filter(Boolean), notes: g.notes, tone };
}

const NO_COMPARE: AbCompareRecord = { active: false, side: null, aName: "", bName: "", recall: null };

/** What the core's `meta` says of a document, for the list only (the core reads it again when used). */
export function docMeta(text: string, fileName: string): { name: string; note: string; kind: string; savedUtc: string } {
  let meta: Record<string, unknown> = {};
  let snap: Record<string, unknown> = {};
  try {
    const doc = JSON.parse(text) as { meta?: unknown; snapshot?: unknown };
    if (doc && typeof doc.meta === "object" && doc.meta) meta = doc.meta as Record<string, unknown>;
    // A snapshot's own block (P1SN) holds its note.
    if (doc && typeof doc.snapshot === "object" && doc.snapshot) snap = doc.snapshot as Record<string, unknown>;
  } catch {
    // The core says why when it is used; listed under its file's name meanwhile.
  }
  const str = (o: Record<string, unknown>, k: string) => (typeof o[k] === "string" ? (o[k] as string) : "");
  const stem = fileName.replace(/^.*[\\/]/, "").replace(/\.[^.]+$/, "");
  return {
    name: str(meta, "name") || stem || "Untitled",
    note: str(snap, "note") || str(meta, "note"),
    kind: str(meta, "kind") || "preset",
    savedUtc: str(meta, "savedUtc"),
  };
}

/** A file name for a document called `name`: no path or reserved characters, with the extension. */
export function fileNameFor(name: string, fallback = "snapshot"): string {
  const base = name.replace(/[\\/:*?"<>|\u0000-\u001f]+/g, "_").replace(/^[\s.]+|[\s.]+$/g, "").slice(0, 120);
  return `${base || fallback}.dspipreset`;
}

/** Seconds left of a waiting guard, rounded up, never below 0. */
export function secondsLeft(g: GuardedApplyRecord | null): number {
  return g && g.phase === "waiting" ? Math.max(0, Math.ceil(g.remainingMs / 1000)) : 0;
}

/** Whether the A/B toggle may be used now: a compare runs, no guard waits, nothing else runs, writes allowed. */
export function mayToggle(st: DeskState, readOnly: boolean): boolean {
  return st.ab.active && st.guard?.phase !== "waiting" && !st.busy && !readOnly;
}

/** The import options for the chosen blocks (`IMPORT_*` bits), always by block. */
export function importOptions(blocks: number): number {
  return IMPORT_BY_BLOCK | (blocks & 0x1ff);
}

function message(e: unknown): string {
  const m = e instanceof Error ? e.message : String(e);
  return m ? m[0]!.toUpperCase() + m.slice(1) : m;
}

export class SnapshotDesk {
  state: DeskState = {
    docs: [],
    ab: NO_COMPARE,
    guard: null,
    guardDefault: false,
    busy: "",
    outcome: null,
    abDiff: null,
    timeoutS: DEFAULT_TIMEOUT_S,
    waitingText: WAITING_FALLBACK,
    backstopText: BACKSTOP_FALLBACK, // -- WP P1WB2 --
  };
  private readonly listeners = new Set<() => void>();
  private next = 1;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private alive = true;
  // -- WP P1DCFIX -- the session's status, to ask the core again when it opens again
  private unwatch: (() => void) | undefined;
  private wasOpen = false;
  // -- WP P5FOLLOW -- the core's events: a guard the auto-tune started
  private unsubscribe: (() => void) | undefined;

  constructor(
    readonly session: Session,
    private readonly pollMs = GUARD_POLL_MS,
  ) {}

  /** Whether this session may write (a viewer or read-only firmware may not). */
  get readOnly(): string {
    return this.session.readOnlyReason;
  }

  /** What the core says now: the guard's default, a compare, a guard (a reload of the view). */
  async start(): Promise<void> {
    const core = this.session.core;
    // -- WP P1DCFIX -- the same device open again: a lost guard's Revert is offered again (F1)
    if (!this.unwatch && typeof this.session.watch === "function") {
      this.wasOpen = this.session.state?.status === "open";
      this.unwatch = this.session.watch(() => {
        const open = this.session.state.status === "open";
        if (open && !this.wasOpen) void this.reread();
        this.wasOpen = open;
      });
    }
    // -- WP P5FOLLOW -- (review P5-E) a tune's guard is the bar's too: adopted from the Worker's
    // tune events, its countdown shown; Keep stays the core's gated applyConfirm.
    const sub = (core as { subscribe?: (fn: (e: unknown) => void) => () => void }).subscribe;
    if (!this.unsubscribe && typeof sub === "function") this.unsubscribe = sub.call(core, (e) => this.onCoreEvent(e));
    const [guardDefault, ab, guard, waiting, backstop] = await Promise.all([
      core.call("applyGuardDefault", undefined).catch(() => false),
      core.call("abState", undefined).catch(() => NO_COMPARE),
      core.call("applyStatus", undefined).catch(() => null),
      core.call("noticeFor", { key: GUARD_WAITING_KEY }).catch(() => null),
      core.call("noticeFor", { key: GUARD_BACKSTOP_KEY }).catch(() => null), // -- WP P1WB2 --
    ]);
    if (!this.alive) return;
    const waitingText = waiting && waiting.key === GUARD_WAITING_KEY ? `${waiting.title} ${waiting.reason}` : this.state.waitingText;
    const backstopText = backstop && backstop.key === GUARD_BACKSTOP_KEY ? backstop.reason : this.state.backstopText;
    this.set({ guardDefault, ab, guard: guard && guard.phase !== "idle" ? guard : null, waitingText, backstopText });
    if (guard?.phase === "waiting") this.schedule();
  }

  /** -- WP P1DCFIX -- what the core says of the guard and the compare now (a session that opened again). */
  private async reread(): Promise<void> {
    const core = this.session.core;
    const [guardDefault, ab, guard] = await Promise.all([
      core.call("applyGuardDefault", undefined).catch(() => this.state.guardDefault),
      core.call("abState", undefined).catch(() => this.state.ab),
      core.call("applyStatus", undefined).catch(() => null),
    ]);
    if (!this.alive) return;
    this.set({ guardDefault, ab, guard: guard && guard.phase !== "idle" ? guard : this.state.guard });
    if (guard?.phase === "waiting") this.schedule();
  }

  stop(): void {
    this.alive = false;
    clearTimeout(this.timer);
    this.listeners.clear();
    this.unwatch?.();
    this.unwatch = undefined;
    this.unsubscribe?.(); // -- WP P5FOLLOW --
    this.unsubscribe = undefined;
  }

  // -- WP P5FOLLOW --
  /** A core event: a waiting guard of the auto-tune that this bar does not drive yet is adopted. */
  private onCoreEvent(e: unknown): void {
    const x = e as { event?: unknown; state?: { guard?: GuardedApplyRecord | null } | null } | null;
    if (!this.alive || !x || x.event !== "measureTune") return;
    const g = x.state?.guard;
    if (g) this.adoptGuard(g);
  }

  /**
   * Show a guard another part of the page started (the auto-tune's apply): while it waits, with its
   * countdown, driven as the bar's own (`applyPoll`). Nothing for a guard that does not wait, or the
   * one the bar already drives (its countdown is the bar's own poll's).
   */
  adoptGuard(g: GuardedApplyRecord): void {
    if (!this.alive || g.phase !== "waiting") return;
    const now = this.state.guard;
    if (now && now.phase === "waiting" && now.id === g.id) return;
    this.set({ guard: g });
    this.schedule();
  }

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private set(patch: Partial<DeskState>): void {
    this.state = { ...this.state, ...patch };
    for (const l of this.listeners) l();
  }

  /** One action at a time; its failure is the outcome, in the core's words. */
  private async run<T>(what: string, f: () => Promise<T>): Promise<T | null> {
    if (this.state.busy) return null;
    this.set({ busy: what });
    try {
      return await f();
    } catch (e) {
      this.set({ outcome: { title: `${what}: not done`, lines: [await this.refusal(e)], notes: [], tone: "error" } });
      return null;
    } finally {
      this.set({ busy: "" });
    }
  }

  // -- WP MERGEP1D --
  /**
   * A failure in the core's words; a refusal by the installer's lock in its
   * notice row (`installer_lock.hub` when the hub holds the lock, not the "on
   * this console" words). The one lock prompt (`heldBy`) comes with P1-E.
   */
  private async refusal(e: unknown): Promise<string> {
    if (!(e instanceof CoreCallError) || !e.noticeKey?.startsWith("installer_lock")) return message(e);
    const n = await this.session.core.call("noticeFor", { key: e.noticeKey }).catch(() => null);
    return n && n.key === e.noticeKey ? `${n.title}: ${n.reason}` : message(e);
  }

  private writable(what: string): boolean {
    if (!this.readOnly) return true;
    this.set({ outcome: { title: `${what}: not done`, lines: [this.readOnly], notes: [], tone: "error" } });
    return false;
  }

  // ---------------------------------------------------------------- documents

  private hold(text: string, fileName: string, origin: HeldDoc["origin"]): HeldDoc {
    const m = docMeta(text, fileName);
    const doc: HeldDoc = { id: this.next++, ...m, fileName: fileName || fileNameFor(m.name), text, origin };
    this.set({ docs: [...this.state.docs, doc] });
    return doc;
  }

  /** The device as a snapshot document: held here, and the text for the download. */
  capture(name: string, note: string): Promise<HeldDoc | null> {
    return this.run("Capture", async () => {
      const { text } = await this.session.core.call("snapshotCapture", { name, note });
      const doc = this.hold(text, "", "captured");
      this.set({ outcome: null });
      return doc;
    });
  }

  /** A document from a file: the core reads it first (its words when it is not one). */
  addFile(text: string, fileName: string): Promise<HeldDoc | null> {
    return this.run("Open Snapshot", async () => {
      await this.session.core.call("configDiff", { before: text, after: text });
      this.set({ outcome: null });
      return this.hold(text, fileName, "file");
    });
  }

  remove(id: number): void {
    this.set({ docs: this.state.docs.filter((d) => d.id !== id) });
  }

  doc(id: number): HeldDoc | undefined {
    return this.state.docs.find((d) => d.id === id);
  }

  // ---------------------------------------------------------------- recall

  /**
   * Recall a held snapshot. Guarded when the core says so here (Link to a hub with the
   * guard), or when the user asks (`guarded`); otherwise a plain recall.
   */
  recall(id: number, guarded = false, timeoutS = this.state.timeoutS): Promise<boolean> {
    const doc = this.doc(id);
    if (!doc) return Promise.resolve(false);
    return this.recallText(doc.text, guarded, timeoutS);
  }

  // -- WP P2WB2 -- a document the desk does not hold (the hub's copy) recalls as a held one does
  /** Recall a snapshot document's text, guarded as `recall` decides. */
  recallText(text: string, guarded = false, timeoutS = this.state.timeoutS): Promise<boolean> {
    if (!this.writable("Recall")) return Promise.resolve(false);
    return this.run("Recall", async () => {
      if (guarded || (await this.guardDefaultNow())) {
        const g = await this.session.core.call("applyGuardedSnapshot", { text, timeoutS });
        await this.guardStarted(g);
        return true;
      }
      const r = await this.session.core.call("snapshotRecall", { text });
      await this.after();
      this.set({ outcome: { title: `Recalled “${r.name}”`, lines: r.changes, notes: r.notes, tone: r.notes.length ? "warn" : "info" } });
      return true;
    }).then((v) => v ?? false);
  }

  // -- WP P2WB3 --
  /**
   * Recall a snapshot the hub keeps, on the hub (`recall`, the hub's `snapshot.recall`). Where
   * recalls are guarded here, the hub guards it itself and the core adopts that guard: the
   * countdown bar's Keep and Revert end it as any other (Revert and the timeout have the hub put
   * back what it captured). The way back where the hub cannot recall it guarded (`unsupported`):
   * the hub's document (`text`) recalled here, guarded here, as before.
   */
  recallOnHub(recall: (guard: boolean, timeoutMs: number) => Promise<HubSnapshotRecallRecord>, text: () => Promise<string>): Promise<boolean> {
    if (!this.writable("Recall")) return Promise.resolve(false);
    return this.run("Recall", async () => {
      const guarded = await this.guardDefaultNow();
      const timeoutS = this.state.timeoutS;
      let r: HubSnapshotRecallRecord;
      try {
        r = await recall(guarded, timeoutS * 1000);
      } catch (e) {
        if (!(guarded && e instanceof CoreCallError && e.code === "unsupported")) throw e;
        const g = await this.session.core.call("applyGuardedSnapshot", { text: await text(), timeoutS });
        await this.guardStarted(g);
        return true;
      }
      if (guarded) {
        const g = await this.session.core.call("applyStatus", undefined).catch(() => null);
        if (g?.phase === "waiting") {
          await this.guardStarted(g);
          return true;
        }
      }
      await this.after();
      this.set({ outcome: { title: `Recalled “${r.name}” on the hub`, lines: r.lines, notes: r.notes, tone: r.notes.length ? "warn" : "info" } });
      return true;
    }).then((v) => v ?? false);
  }

  /** Asked again before each recall and import: the session may have moved (a reconnect). */
  private async guardDefaultNow(): Promise<boolean> {
    const on = await this.session.core.call("applyGuardDefault", undefined).catch(() => this.state.guardDefault);
    if (on !== this.state.guardDefault) this.set({ guardDefault: on });
    return on;
  }

  /** After a write the core made: the device read again, and the compare as the core has it. */
  private async after(): Promise<void> {
    await this.session.refresh();
    const ab = await this.session.core.call("abState", undefined).catch(() => this.state.ab);
    this.set({ ab, abDiff: ab.active ? this.state.abDiff : null });
  }

  // ---------------------------------------------------------------- A/B

  /** Compare the held `b` with `a` (null: the device now, captured as A). */
  abStart(b: number, a: number | null): Promise<boolean> {
    const bd = this.doc(b);
    const ad = a == null ? null : this.doc(a);
    if (!bd || (a != null && !ad) || !this.writable("Compare")) return Promise.resolve(false);
    return this.run("Compare", async () => {
      const ab = await this.session.core.call("abStart", ad ? { a: ad.text, b: bd.text } : { b: bd.text });
      this.set({ ab, abDiff: null, outcome: null });
      await this.after();
      return true;
    }).then((v) => v ?? false);
  }

  /** The other side; refused (and off) while a guard waits. */
  abToggle(): Promise<boolean> {
    if (!mayToggle(this.state, !!this.readOnly)) return Promise.resolve(false);
    return this.run("A/B", async () => {
      const ab = await this.session.core.call("abToggle", undefined);
      this.set({ ab });
      await this.after();
      if (ab.recall?.notes.length) this.set({ outcome: { title: `Now ${ab.side === "a" ? "A" : "B"}`, lines: [], notes: ab.recall.notes, tone: "warn" } });
      return true;
    }).then((v) => v ?? false);
  }

  abKeep(side: "a" | "b"): Promise<boolean> {
    if (!this.state.ab.active || !this.writable("Keep")) return Promise.resolve(false);
    return this.run("Keep", async () => {
      const ab = await this.session.core.call("abKeep", { side });
      this.set({ ab, abDiff: null });
      await this.after();
      return true;
    }).then((v) => v ?? false);
  }

  abDiff(): Promise<ConfigDiff | null> {
    return this.run("Differences", async () => {
      const d = await this.session.core.call("abDiff", undefined);
      this.set({ abDiff: d });
      return d;
    });
  }

  // ---------------------------------------------------------------- safe apply

  private async guardStarted(g: GuardedApplyRecord): Promise<void> {
    this.set({ guard: g, outcome: null });
    await this.after();
    if (g.phase === "waiting") this.schedule();
    else this.guardEnded(g);
  }

  private schedule(): void {
    clearTimeout(this.timer);
    if (this.alive) this.timer = setTimeout(() => void this.pollGuard(), this.pollMs);
  }

  /** One turn of the guard: the countdown, and the revert when it runs out (the core's, on this poll). */
  async pollGuard(): Promise<void> {
    if (!this.alive || this.state.guard?.phase !== "waiting") return;
    try {
      const g = await this.session.core.call("applyPoll", undefined);
      if (!this.alive) return;
      if (g.phase === "waiting") {
        this.set({ guard: g });
        this.schedule();
        return;
      }
      this.set({ guard: g });
      await this.after();
      this.guardEnded(g);
    } catch (e) {
      // The session ended: nothing is replayed (the lost-Link rule). Said once.
      this.set({ outcome: { title: "Safe apply", lines: [message(e)], notes: [], tone: "error" } });
      if (e instanceof CoreCallError && (e.code === "not_open" || e.code === "dead")) {
        // -- WP P1DCFIX -- where the core says it stands (no I/O): a lost guard keeps its Revert
        const now = await this.session.core.call("applyStatus", undefined).catch(() => null);
        if (this.alive && now && now.phase !== "waiting") this.set({ guard: now.phase === "idle" ? null : now });
        return;
      }
      this.schedule();
    }
  }

  private guardEnded(g: GuardedApplyRecord): void {
    // -- WP P1DCFIX -- a lost guard is the bar's (with Revert), not a result line
    // -- WP REVERTW -- unless its end is not known: then it is the outcome
    if (g.phase === "lost" && !g.revertNotice) return;
    this.set({ outcome: guardOutcome(g) });
  }

  /** Keep the change. */
  keep(): Promise<boolean> {
    return this.endGuard("applyConfirm");
  }

  /** Put the settings from before back: while it waits, or (on the user's word only) a lost one. */
  revert(): Promise<boolean> {
    return this.endGuard("applyRevert");
  }

  private async endGuard(method: "applyConfirm" | "applyRevert"): Promise<boolean> {
    const phase = this.state.guard?.phase;
    // -- WP P1DCFIX -- a lost guard can still be reverted (F1); Keep is for a waiting one
    if (phase !== "waiting" && !(phase === "lost" && method === "applyRevert")) return false;
    clearTimeout(this.timer);
    const g = await this.session.core.call(method, undefined).catch((e: unknown) => {
      this.set({ outcome: { title: "Safe apply", lines: [message(e)], notes: [], tone: "error" } });
      return null;
    });
    if (!g) {
      // Ask where it stands: it may have ended meanwhile.
      const now = await this.session.core.call("applyStatus", undefined).catch(() => null);
      if (now) this.set({ guard: now });
      if (now?.phase === "waiting") this.schedule();
      return false;
    }
    this.set({ guard: g });
    await this.after();
    this.guardEnded(g);
    return true;
  }

  // ---------------------------------------------------------------- import and export

  /** What importing `text` with `blocks` (and the EQ of `eqChannels`) would change; nothing written. */
  preview(text: string, blocks: number, eqChannels = 0xffff_ffff): Promise<ConfigDiff> {
    return this.session.core.call("configPreview", { text, options: importOptions(blocks), eqChannels });
  }

  /**
   * Import the chosen blocks. With rollback (`guarded`, or by default here) it is a guarded
   * apply and the countdown runs; otherwise the core's report is the outcome.
   */
  importDoc(text: string, name: string, blocks: number, eqChannels: number, guarded: boolean, timeoutS = this.state.timeoutS): Promise<boolean> {
    if (!this.writable("Import")) return Promise.resolve(false);
    return this.run("Import", async () => {
      const options = importOptions(blocks);
      if (guarded || (await this.guardDefaultNow())) {
        const g = await this.session.core.call("applyGuardedImport", { text, options, eqChannels, timeoutS });
        await this.guardStarted(g);
        return true;
      }
      // -- WP P1WB2 -- the import was confirmed in its dialog: its token
      const token = await this.session.token("config_import");
      const r: ImportReport = await this.session.core.call("configImport", { text, options, eqChannels, ...(token ? { token } : {}) });
      restoreLinks(this.session, r.links); // -- WP WB6K --
      await this.after();
      this.set({ outcome: { title: `Imported “${name}”`, lines: r.lines, notes: [], tone: r.clean ? "info" : "warn" } });
      return true;
    }).then((v) => v ?? false);
  }

  /** The device (or the open document) as a configuration file's text. */
  exportDoc(name: string): Promise<string | null> {
    return this.run("Export", async () => (await this.session.core.call("configExport", { name, appVersion: "web", links: inputLinks(this.session).mask })).text);
  }

  /** The device or document as a template's text. */
  saveTemplate(name: string): Promise<string | null> {
    return this.run("Save as Template", async () => (await this.session.core.call("templateSave", { name, appVersion: "web", links: inputLinks(this.session).mask })).text);
  }

  /** The open document as it now stands (offline); nothing is unsaved afterwards. */
  saveDocument(): Promise<string | null> {
    return this.run("Save File", async () => {
      const { text } = await this.session.core.call("documentSave", { appVersion: "web", links: inputLinks(this.session).mask });
      await this.session.refresh();
      return text;
    });
  }

  /** The rollback time for the next guarded apply, 5..300 s. */
  setTimeoutS(s: number): void {
    const v = Number.isFinite(s) ? Math.min(300, Math.max(5, Math.round(s))) : DEFAULT_TIMEOUT_S;
    if (v !== this.state.timeoutS) this.set({ timeoutS: v });
  }

  dismiss(): void {
    if (this.state.outcome) this.set({ outcome: null });
  }

  // -- WP WB6K --
  /** One of the Files screen's actions, as the desk runs its own: one at a time, a failure as the outcome. */
  act<T>(what: string, f: () => Promise<T>, writes = true): Promise<T | null> {
    if (writes && !this.writable(what)) return Promise.resolve(null);
    return this.run(what, f);
  }

  /** Show `outcome` (the Files screen's actions). */
  report(outcome: Outcome): void {
    this.set({ outcome });
  }
}

/** Hand `text` to the browser as a download called `fileName`. */
export function download(text: string, fileName: string): void {
  const url = URL.createObjectURL(new Blob([text], { type: "application/json" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = fileName;
  a.style.display = "none";
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

// -- WP WB6K --
/** An import's pair-link mask (null: the document has none) onto the session's links, pair by pair. */
export function restoreLinks(session: Session, links: number | null | undefined): void {
  if (typeof links !== "number") return;
  const l = inputLinks(session);
  for (let pair = 0; pair < 4; pair++) l.set(pair * 2, (links & (1 << pair)) !== 0);
}
