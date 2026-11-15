// WP WB6l: one session's control-surface editor state, over the core's calls
// (crates/dspi-wasm/src/surfaces.rs). Qt's CsController (WP QP12) in the web's
// shape: the snapshot and the records of model.ts, the core's rows for each
// draft, and the operations. Every write is one `csApply`, which the core
// refuses on a read-only connection, in the client read-only mode and under the
// installer's lock before anything is sent; the page shows that refusal on the
// card it came from. A learn, a running macro and a shown display are followed
// by polls of the core, as on Qt.

import type { CsDescription, CsRecord, CsReply } from "../../core/protocol";
import type { Session } from "../../ui/session";
import {
  LEARN_ARMED,
  LEARN_DONE,
  LEARN_TIMEOUT,
  MACRO_NONE,
  STATUS_NO_IR,
  TARGET_OUTPUTS,
  TY,
  activePages,
  applyOps,
  cleanName,
  displayConfig,
  displaySlot,
  failureText,
  freePage,
  freeSlot,
  inUse,
  keyOf,
  macroRunning,
  maxSteps,
  nameOf,
  num,
  protocolName,
  removeOps,
  supported,
  syncRecords,
  type CsRec,
} from "./model";

/** The learn window is read every 100 ms: the device's 10 s and a margin. */
const LEARN_MS = 100;
const LEARN_POLLS = 115;
/** A running macro is followed every 250 ms, at most this often. */
const MACRO_MS = 250;
const MACRO_POLLS = 120;
/** The display panel's state while its card shows. */
const DISPLAY_MS = 1000;

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export class CsDesk {
  snap: CsRecord | null = null;
  recs = new Map<string, CsRec>();
  loaded = false;
  loadError = "";
  /** The display settings' and pages' rows. */
  display: CsDescription | null = null;
  pages = new Map<number, CsDescription | null>();
  displayMessage = "";
  /** The remote button being learned, -1 for none, and what to tell meanwhile. */
  learning = -1;
  learnMessage = "";
  /** Operations running now. */
  busy = 0;
  texts: Record<string, string> = {};
  /** Counts every change, for the view. */
  rev = 0;

  private readonly listeners = new Set<() => void>();
  private readonly queues = new Map<string, Promise<void>>();
  private timers = new Set<ReturnType<typeof setTimeout>>();
  private learnTicks = 0;
  private macroTicks = 0;
  private macroPolling = false;
  private displayWatch = false;
  private displayPolling = false;
  private alive = true;

  constructor(readonly session: Session) {}

  /** Writes are possible at all (the session's view; the core is the one that refuses). */
  get writable(): boolean {
    return !this.session.readOnlyReason && this.session.state.status === "open";
  }

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private bump(): void {
    this.rev++;
    for (const fn of this.listeners) fn();
  }

  private later(ms: number, fn: () => void): void {
    if (!this.alive) return;
    const t = setTimeout(() => {
      this.timers.delete(t);
      if (this.alive) fn();
    }, ms);
    this.timers.add(t);
  }

  dispose(): void {
    if (this.learning >= 0) void this.cancelLearn();
    this.alive = false;
    for (const t of this.timers) clearTimeout(t);
    this.timers.clear();
    this.listeners.clear();
  }

  rec(key: string): CsRec | undefined {
    return this.recs.get(key);
  }

  private put(r: CsRec): void {
    this.recs.set(r.key, r);
  }

  private patch(key: string, p: Partial<CsRec>): CsRec | undefined {
    const r = this.recs.get(key);
    if (!r) return undefined;
    const next = { ...r, ...p };
    this.recs.set(key, next);
    return next;
  }

  // ---- the device ---------------------------------------------------------------

  /** Read everything from the device (reads only, so also while read only). */
  async fetch(): Promise<void> {
    this.loadError = "";
    this.bump();
    try {
      if (Object.keys(this.texts).length === 0) this.texts = await this.session.core.call("csStatusTexts", undefined);
      const snap = await this.session.core.call("csFetch", undefined);
      await this.applySnapshot(snap);
      this.loaded = true;
    } catch (e) {
      this.loadError = message(e);
    }
    this.bump();
  }

  private async readSnapshot(): Promise<void> {
    try {
      await this.applySnapshot(await this.session.core.call("csSnapshot", undefined));
    } catch (e) {
      this.loadError = message(e);
    }
  }

  private async applySnapshot(snap: CsRecord): Promise<void> {
    this.snap = snap;
    this.recs = supported(snap) ? syncRecords(snap, this.recs) : new Map();
    await this.describeAll();
    this.bump();
  }

  private describe(kind: string, slot: number, draft: CsRecord): Promise<CsDescription | null> {
    return this.session.core.call("csDescribe", { kind, slot, draft }).then(
      (d) => (d.ok ? d : null),
      () => null,
    );
  }

  /** The rows of every record a page could show. */
  private async describeAll(): Promise<void> {
    const snap = this.snap;
    if (!snap || !supported(snap)) return;
    for (const r of [...this.recs.values()]) {
      if (!inUse(r)) continue;
      if (r.kind === "macro") {
        const steps = (Array.isArray(r.draft.steps) ? r.draft.steps : []) as CsRecord[];
        const ds = await Promise.all(steps.map((s) => this.describe("step", 0, s)));
        this.patch(r.key, { steps: ds });
      } else {
        this.patch(r.key, { description: await this.describe(r.kind, r.slot, r.draft) });
      }
    }
    this.pages = new Map();
    if (displaySlot(snap, this.recs) >= 0) {
      this.display = await this.describe("display", 0, displayConfig(snap));
      for (const p of activePages(snap)) this.pages.set(p.index, await this.describe("page", p.index, p.page));
    } else this.display = null;
  }

  private async describeOne(key: string): Promise<void> {
    const r = this.recs.get(key);
    if (!r) return;
    if (r.kind === "macro") {
      const steps = (Array.isArray(r.draft.steps) ? r.draft.steps : []) as CsRecord[];
      this.patch(key, { steps: await Promise.all(steps.map((s) => this.describe("step", 0, s))) });
    } else this.patch(key, { description: await this.describe(r.kind, r.slot, r.draft) });
    this.bump();
  }

  /**
   * Run `ops` in order, stopping at the first refusal, then read the snapshot
   * again. Answers the refusal's words, or "" when every one went through.
   */
  private async run(ops: CsRecord[]): Promise<{ failure: string; reply: CsReply | null }> {
    this.busy++;
    this.bump();
    let failure = "";
    let reply: CsReply | null = null;
    try {
      for (const op of ops) {
        reply = await this.session.core.call("csApply", { op });
        if (!reply.ok) {
          failure = failureText(reply, this.texts);
          break;
        }
      }
    } catch (e) {
      failure = message(e);
    }
    await this.readSnapshot();
    // The change history and anything else the session shows follow the write.
    void this.session.refresh();
    this.busy--;
    this.bump();
    return { failure, reply };
  }

  /** One record's edits one after the other, each on the draft the last one left. */
  private queue(key: string, job: () => Promise<void>): Promise<void> {
    const next = (this.queues.get(key) ?? Promise.resolve()).then(job, job);
    this.queues.set(key, next);
    return next;
  }

  // ---- edits --------------------------------------------------------------------

  /** A row of `key`'s draft changed: the core's next draft (CG20), described again. Nothing is sent. */
  edit(key: string, row: string, value: number | boolean): Promise<void> {
    return this.queue(key, async () => {
      const r = this.recs.get(key);
      if (!r) return;
      const e = await this.session.core.call("csEdit", { kind: r.kind, slot: r.slot, draft: r.draft, change: { id: row, value } }).catch((x) => ({ ok: false as const, error: message(x) }));
      if (e.ok) {
        this.patch(key, { draft: e.draft, message: "", isError: false });
        await this.describeOne(key);
      } else {
        this.patch(key, { message: e.error, isError: true });
        this.bump();
      }
    });
  }

  setName(key: string, name: string): void {
    const r = this.recs.get(key);
    if (!r || !this.snap) return;
    const clean = cleanName(name);
    if (r.kind === "binding") this.patch(key, { nameEdit: clean === nameOf(this.snap, r.slot) ? null : clean });
    else this.patch(key, { draft: { ...r.draft, name: clean } });
    this.bump();
  }

  /** Drop `key`'s edits (a receiver's: its remote buttons' too). */
  revertRecord(key: string): void {
    const r = this.recs.get(key);
    if (!r) return;
    this.patch(key, { draft: r.live, nameEdit: null, message: "", isError: false });
    if (r.kind === "binding" && num(r.draft, "type") === TY.Ir)
      for (const ir of this.recs.values()) if (ir.kind === "ir") this.patch(ir.key, { draft: ir.live, message: "" });
    void this.describeAll().then(() => this.bump());
  }

  async apply(key: string): Promise<void> {
    const r = this.recs.get(key);
    if (!r || r.applying) return;
    const { ops, irKeys } = applyOps(r, this.recs);
    if (ops.length === 0) return;
    this.patch(key, { applying: true, message: "" });
    for (const k of irKeys) this.patch(k, { applying: true });
    const { failure } = await this.run(ops);
    for (const k of irKeys) {
      const ir = this.recs.get(k);
      if (ir) this.patch(k, failure ? { applying: false } : { applying: false, draft: ir.live, message: "" });
    }
    const now = this.recs.get(key);
    if (!now) return;
    // The refused draft stays, so what was wrong can be fixed and applied again.
    if (failure) this.patch(key, { applying: false, message: failure, isError: true });
    else this.patch(key, { applying: false, draft: now.live, nameEdit: null, message: "", isError: false });
    await this.describeAll();
    this.bump();
  }

  async remove(key: string): Promise<void> {
    const r = this.recs.get(key);
    if (!r || r.applying || !this.snap) return;
    const ops = removeOps(r, this.snap);
    if (ops.length > 0 && !this.writable) return;
    if (ops.length === 0) {
      // Never applied: dropping the draft is enough.
      this.patch(key, { draft: r.live, nameEdit: null, message: "" });
      await this.describeAll();
      this.bump();
      return;
    }
    this.patch(key, { applying: true, nameEdit: null, message: "" });
    const { failure } = await this.run(ops);
    const now = this.recs.get(key);
    if (now) this.patch(key, { applying: false, draft: now.live, message: failure, isError: !!failure });
    await this.describeAll();
    this.bump();
  }

  /** A new control of `type` in the first free slot, on free pins, applied at once (it works the moment it is added). */
  async addControl(type: number): Promise<string | null> {
    if (!this.writable || !this.snap) return null;
    const slot = freeSlot("binding", this.snap, this.recs);
    if (slot === null) return null;
    const key = keyOf("binding", slot);
    const made = await this.session.core.call("csNewBinding", { slot, component: type }).catch(() => null);
    if (!made?.ok) return null;
    this.patch(key, { draft: made.draft, message: "" });
    await this.describeOne(key);
    await this.apply(key);
    return key;
  }

  /** A new group, drafted: one with no members is not one the device stores, so Apply waits for them. */
  addGroup(): string | null {
    if (!this.snap) return null;
    const g = freeSlot("group", this.snap, this.recs);
    if (g === null) return null;
    const key = keyOf("group", g);
    this.patch(key, { draft: { targetKind: TARGET_OUTPUTS, memberMask: 0, name: `Group ${g + 1}` }, message: "" });
    void this.describeOne(key);
    this.bump();
    return key;
  }

  addMacro(): string | null {
    if (!this.snap) return null;
    const m = freeSlot("macro", this.snap, this.recs);
    if (m === null) return null;
    const key = keyOf("macro", m);
    this.patch(key, { draft: { name: `Macro ${m + 1}`, steps: [] }, steps: [], message: "" });
    this.bump();
    return key;
  }

  /** A new remote button under the receiver; it needs its code from a learn. */
  async addRemoteButton(): Promise<string | null> {
    if (!this.snap) return null;
    const s = freeSlot("ir", this.snap, this.recs);
    if (s === null) return null;
    const made = await this.session.core.call("csNewRecord", { kind: "ir" }).catch(() => null);
    if (!made?.ok) return null;
    const key = keyOf("ir", s);
    this.patch(key, { draft: made.draft, message: "" });
    await this.describeOne(key);
    return key;
  }

  // ---- macros -------------------------------------------------------------------

  private stepsOf(key: string): CsRecord[] {
    const r = this.recs.get(key);
    return r && Array.isArray(r.draft.steps) ? [...(r.draft.steps as CsRecord[])] : [];
  }

  addStep(key: string): Promise<void> {
    return this.queue(key, async () => {
      if (!this.snap || this.stepsOf(key).length >= maxSteps(this.snap)) return;
      const made = await this.session.core.call("csNewRecord", { kind: "step" }).catch(() => null);
      const r = this.recs.get(key);
      if (!r || !made?.ok) return;
      this.patch(key, { draft: { ...r.draft, steps: [...this.stepsOf(key), made.draft] } });
      await this.describeOne(key);
    });
  }

  moveStep(key: string, step: number, delta: number): void {
    const steps = this.stepsOf(key);
    const dest = step + delta;
    const r = this.recs.get(key);
    if (!r || step < 0 || step >= steps.length || dest < 0 || dest >= steps.length) return;
    [steps[step], steps[dest]] = [steps[dest]!, steps[step]!];
    const ds = [...r.steps];
    [ds[step], ds[dest]] = [ds[dest] ?? null, ds[step] ?? null];
    this.patch(key, { draft: { ...r.draft, steps }, steps: ds });
    this.bump();
  }

  /** The sequencer runs the steps in order, so a removed one closes the gap. */
  removeStep(key: string, step: number): void {
    const steps = this.stepsOf(key);
    const r = this.recs.get(key);
    if (!r || step < 0 || step >= steps.length) return;
    steps.splice(step, 1);
    const ds = [...r.steps];
    ds.splice(step, 1);
    this.patch(key, { draft: { ...r.draft, steps }, steps: ds });
    this.bump();
  }

  editStep(key: string, step: number, row: string, value: number | boolean): Promise<void> {
    return this.queue(key, async () => {
      const steps = this.stepsOf(key);
      const s = steps[step];
      if (!s) return;
      const e = await this.session.core.call("csEdit", { kind: "step", slot: 0, draft: s, change: { id: row, value } }).catch((x) => ({ ok: false as const, error: message(x) }));
      const r = this.recs.get(key);
      if (!r) return;
      if (e.ok) {
        steps[step] = e.draft;
        const ds = [...r.steps];
        ds[step] = await this.describe("step", 0, e.draft);
        this.patch(key, { draft: { ...r.draft, steps }, steps: ds, message: "", isError: false });
      } else this.patch(key, { message: e.error, isError: true });
      this.bump();
    });
  }

  /** Run a macro, or stop the one running. */
  async fire(key: string): Promise<void> {
    const r = this.recs.get(key);
    if (!r || r.kind !== "macro" || !this.snap) return;
    const running = macroRunning(this.snap) === r.slot;
    const { failure } = await this.run([running ? { op: "macroCancel" } : { op: "macroFire", index: r.slot }]);
    if (failure) {
      this.patch(key, { message: failure, isError: true });
      this.bump();
      return;
    }
    this.macroTicks = 0;
    if (!this.macroPolling) this.pollMacro();
  }

  private pollMacro(): void {
    this.macroPolling = true;
    this.later(MACRO_MS, async () => {
      await this.session.core.call("csApply", { op: { op: "refreshStatus" } }).catch(() => null);
      await this.readSnapshot();
      this.bump();
      if (!this.snap || macroRunning(this.snap) === MACRO_NONE || ++this.macroTicks >= MACRO_POLLS) this.macroPolling = false;
      else this.pollMacro();
    });
  }

  // ---- IR learn -----------------------------------------------------------------

  async startLearn(sub: number): Promise<void> {
    const key = keyOf("ir", sub);
    if (!this.recs.get(key) || this.learning >= 0) return;
    this.learning = sub;
    this.learnTicks = 0;
    this.learnMessage = "Point the remote at the receiver and press the button to learn.";
    this.patch(key, { message: "" });
    this.bump();
    const { failure, reply } = await this.run([{ op: "learnArm" }]);
    if (this.learning !== sub) return;
    if (!failure) {
      this.pollLearn(sub);
      return;
    }
    this.learning = -1;
    this.learnMessage = "";
    this.patch(key, { message: reply?.status === STATUS_NO_IR ? "No IR receiver is active - apply the receiver first." : failure, isError: true });
    this.bump();
  }

  private pollLearn(sub: number): void {
    this.later(LEARN_MS, async () => {
      if (this.learning !== sub) return;
      const r = await this.session.core.call("csApply", { op: { op: "learnRead" } }).catch((e) => ({ ok: false, error: message(e) }) as CsReply);
      if (this.learning !== sub) return;
      this.learnTicks++;
      if (r.ok && r.state !== LEARN_ARMED) await this.finishLearn(r, false);
      else if (!r.ok || this.learnTicks >= LEARN_POLLS) await this.finishLearn(r, true);
      else this.pollLearn(sub);
    });
  }

  private async finishLearn(result: CsReply, timedOut: boolean): Promise<void> {
    const sub = this.learning;
    this.learning = -1;
    this.learnMessage = "";
    const key = keyOf("ir", sub);
    const r = this.recs.get(key);
    if (!r) return this.bump();
    const code = result.code ?? 0;
    if (timedOut || result.state !== LEARN_DONE || code === 0) {
      if (!result.ok && result.error) this.patch(key, { message: result.error, isError: true });
      else {
        const device = result.state === LEARN_TIMEOUT;
        this.patch(key, { message: device ? "No remote button was detected - try again." : "Learn stopped.", isError: device });
      }
      // A learn the page gave up on is ended on the device too.
      if (timedOut) void this.session.core.call("csApply", { op: { op: "learnCancel" } }).catch(() => null);
      this.bump();
      return;
    }
    this.patch(key, {
      draft: { ...r.draft, protocol: result.protocol ?? 0, code },
      message: `Learned a ${protocolName(result.protocol ?? 0)} code. Apply to keep it.`,
      isError: false,
    });
    await this.describeOne(key);
  }

  async cancelLearn(): Promise<void> {
    const sub = this.learning;
    if (sub < 0) return;
    this.learning = -1;
    this.learnMessage = "";
    this.patch(keyOf("ir", sub), { message: "Learn cancelled.", isError: false });
    this.bump();
    await this.session.core.call("csApply", { op: { op: "learnCancel" } }).catch(() => null);
  }

  // ---- aux outputs --------------------------------------------------------------

  private async auxOp(slot: number, op: CsRecord): Promise<void> {
    const { failure } = await this.run([op]);
    this.patch(keyOf("binding", slot), { message: failure, isError: !!failure });
    this.bump();
  }

  /** Switch an aux output now: instant, never written to flash. */
  setAuxState(slot: number, on: boolean): Promise<void> {
    return this.auxOp(slot, { op: "auxState", index: slot, on: on ? 1 : 0 });
  }

  setAuxLevel(slot: number, percent: number): Promise<void> {
    return this.auxOp(slot, { op: "auxLevel", index: slot, percent: Math.min(100, Math.max(0, percent)) });
  }

  // ---- the display --------------------------------------------------------------

  /** A display setting changed: the core's next settings, sent at once (as the Console). */
  async editDisplay(row: string, value: number | boolean): Promise<void> {
    if (!this.snap) return;
    const e = await this.session.core.call("csEdit", { kind: "display", slot: 0, draft: displayConfig(this.snap), change: { id: row, value } }).catch((x) => ({ ok: false as const, error: message(x) }));
    if (!e.ok) {
      this.displayMessage = e.error;
      return this.bump();
    }
    const { failure } = await this.run([{ op: "displayConfig", config: e.draft }]);
    this.displayMessage = failure;
    this.bump();
  }

  async editPage(index: number, row: string, value: number | boolean): Promise<void> {
    if (!this.snap) return;
    const page = activePages(this.snap).find((p) => p.index === index)?.page;
    if (!page) return;
    const e = await this.session.core.call("csEdit", { kind: "page", slot: index, draft: page, change: { id: row, value } }).catch((x) => ({ ok: false as const, error: message(x) }));
    if (!e.ok) {
      this.displayMessage = e.error;
      return this.bump();
    }
    const { failure } = await this.run([{ op: "displayPage", index, page: e.draft }]);
    this.displayMessage = failure;
    this.bump();
  }

  async addPage(): Promise<void> {
    if (!this.snap) return;
    const index = freePage(this.snap);
    if (index === null) return;
    const made = await this.session.core.call("csNewRecord", { kind: "page" }).catch(() => null);
    if (!made?.ok) return;
    const { failure } = await this.run([{ op: "displayPage", index, page: made.draft }]);
    this.displayMessage = failure;
    this.bump();
  }

  async removePage(index: number): Promise<void> {
    const { failure } = await this.run([{ op: "displayPage", index, page: {} }]);
    this.displayMessage = failure;
    this.bump();
  }

  /** Follow the panel's state while its card shows. */
  watchDisplay(on: boolean): void {
    this.displayWatch = on;
    if (on && !this.displayPolling) this.pollDisplay();
  }

  private pollDisplay(): void {
    this.displayPolling = true;
    this.later(DISPLAY_MS, async () => {
      if (!this.displayWatch || !this.snap || displaySlot(this.snap, this.recs) < 0) {
        this.displayPolling = false;
        return;
      }
      await this.session.core.call("csApply", { op: { op: "displayStatus" } }).catch(() => null);
      await this.readSnapshot();
      this.pollDisplay();
    });
  }

  // ---- the save bar -------------------------------------------------------------

  /** Write the configuration to flash, so it survives a reboot. Answers the refusal's words, or "". */
  async save(): Promise<string> {
    const { failure } = await this.run([{ op: "save" }]);
    return failure;
  }

  /** The device puts back what is stored; every draft then follows it. */
  async revert(): Promise<string> {
    if (this.learning >= 0) await this.cancelLearn();
    const { failure } = await this.run([{ op: "revert" }]);
    for (const r of [...this.recs.values()]) this.patch(r.key, { draft: r.live, nameEdit: null, message: "" });
    await this.describeAll();
    this.bump();
    return failure;
  }
}
