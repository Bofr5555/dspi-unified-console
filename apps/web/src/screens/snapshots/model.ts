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
} from "../../core/protocol";
import type { Session } from "../../ui/session";

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
}

/** The rollback's default, seconds (OD-P1g). */
export const DEFAULT_TIMEOUT_S = 30;
/** How often the page drives a waiting guard, ms. */
export const GUARD_POLL_MS = 250;

// -- WP P1DCFIX --
/** The core's notice key for the countdown (P1NT's table, F7). */
export const GUARD_WAITING_KEY = "guard.waiting";
/** The same words, until the core has answered (the core's row is the one shown). */
const WAITING_FALLBACK = "Keep these settings? They go back in {s} s unless you press Keep.";

/** The countdown sentence with the seconds in, split around them (the page marks the number). */
export function waitingWords(text: string, seconds: number): { before: string; seconds: number; after: string } {
  const at = text.indexOf("{s}");
  if (at < 0) return { before: `${text} `, seconds, after: " s" };
  return { before: text.slice(0, at), seconds, after: text.slice(at + 3) };
}

/** A guard the bar offers: one waiting for Keep, or one lost with the device open again (Revert stays offered). */
export function guardOffered(g: GuardedApplyRecord | null): boolean {
  return !!g && (g.phase === "waiting" || g.phase === "lost");
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
  };
  private readonly listeners = new Set<() => void>();
  private next = 1;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private alive = true;
  // -- WP P1DCFIX -- the session's status, to ask the core again when it opens again
  private unwatch: (() => void) | undefined;
  private wasOpen = false;

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
    const [guardDefault, ab, guard, waiting] = await Promise.all([
      core.call("applyGuardDefault", undefined).catch(() => false),
      core.call("abState", undefined).catch(() => NO_COMPARE),
      core.call("applyStatus", undefined).catch(() => null),
      core.call("noticeFor", { key: GUARD_WAITING_KEY }).catch(() => null),
    ]);
    if (!this.alive) return;
    const waitingText = waiting && waiting.key === GUARD_WAITING_KEY ? `${waiting.title} ${waiting.reason}` : this.state.waitingText;
    this.set({ guardDefault, ab, guard: guard && guard.phase !== "idle" ? guard : null, waitingText });
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
    if (!doc || !this.writable("Recall")) return Promise.resolve(false);
    return this.run("Recall", async () => {
      if (guarded || (await this.guardDefaultNow())) {
        const g = await this.session.core.call("applyGuardedSnapshot", { text: doc.text, timeoutS });
        await this.guardStarted(g);
        return true;
      }
      const r = await this.session.core.call("snapshotRecall", { text: doc.text });
      await this.after();
      this.set({ outcome: { title: `Recalled “${r.name}”`, lines: r.changes, notes: r.notes, tone: r.notes.length ? "warn" : "info" } });
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
    if (g.phase === "lost") return;
    const tone = g.phase === "kept" ? "info" : g.phase === "reverted" ? "warn" : "error";
    this.set({ outcome: { title: g.label || "Safe apply", lines: [g.message].filter(Boolean), notes: g.notes, tone } });
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
      const r: ImportReport = await this.session.core.call("configImport", { text, options, eqChannels });
      await this.after();
      this.set({ outcome: { title: `Imported “${name}”`, lines: r.lines, notes: [], tone: r.clean ? "info" : "warn" } });
      return true;
    }).then((v) => v ?? false);
  }

  /** The device (or the open document) as a configuration file's text. */
  exportDoc(name: string): Promise<string | null> {
    return this.run("Export", async () => (await this.session.core.call("configExport", { name, appVersion: "web" })).text);
  }

  /** The device or document as a template's text. */
  saveTemplate(name: string): Promise<string | null> {
    return this.run("Save as Template", async () => (await this.session.core.call("templateSave", { name, appVersion: "web" })).text);
  }

  /** The open document as it now stands (offline); nothing is unsaved afterwards. */
  saveDocument(): Promise<string | null> {
    return this.run("Save File", async () => {
      const { text } = await this.session.core.call("documentSave", { appVersion: "web" });
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
