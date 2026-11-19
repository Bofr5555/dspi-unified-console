// WP ATWEB: the "Several units" step's state (PLAN-AT 1.4 to 1.6, OD-ATh, OD-ATl). The session
// runs on this screen's own core (its DSPi Link session reaches our hub's `system.measure.*`);
// every unit of the group is judged by its **own** core: a Worker of its own on the same hub,
// opened when the unit is chosen, whose `measureSystemUnitCheck` is asked again right before each
// call that plays (the preflight, the start, the summed check, the verify; review AT-C F2). The
// microphone is the browser's, its blocks straight from the worklet to this core's Worker.
//
// Safety is the core's and the hub's (S1 to S13 per unit, S-AT1 to S-AT7); this desk adds the
// coordinator's placement rule (nothing plays before the person chose the listening position),
// never offers Apply or Revert below Admin, never presses Keep by itself, and stops what it
// started when the tab is left.

import { CoreClient } from "../../../core/client";
import { browserCaptureEnv, startCapture, type Capture, type CaptureEnv } from "../../../core/capture";
import { isMeasureSystemEvent } from "../../../core/measure-system";
import type {
  CoreEvent,
  CoreMethods,
  DeviceSnapshot,
  LinkDeviceRecord,
  MeasureSystemAppliedRecord,
  MeasureSystemPreflightRecord,
  MeasureSystemProposalRecord,
  MeasureSystemResultRecord,
  MeasureSystemSetupRecord,
  MeasureSystemSimInfoRecord,
  MeasureSystemStateRecord,
  MeasureSystemUnitCheckRecord,
  Method,
} from "../../../core/protocol";
import { LocalHubTokens, type HubTokens } from "../../../fleet/hub-tokens";
import { safeLocalStorage } from "../../../fleet/local";
import { linkContext, routeLink, type LinkContext } from "../../../fleet/security";
import type { Source } from "../../../fleet/types";
import { outputRows } from "../../main/model";
import { CONFIRM_MS, KEYS, draftProblem, initialDraft, placementProblem, refusalWords, setupOf, type SystemDraft } from "./model";

/** What the desk needs of a core (a session's `CoreClient` is one). */
export interface SystemCore {
  call<M extends Method>(method: M, params: CoreMethods[M]["params"]): Promise<CoreMethods[M]["result"]>;
  subscribe(fn: (e: CoreEvent) => void): () => void;
}

/** A unit's own core, opened by the desk. */
export interface UnitCore extends SystemCore {
  dispose(): Promise<void>;
}

/** What the desk needs of the screen's session (a `Session` is one). */
export interface SystemSession {
  readonly core: SystemCore;
  readonly source: Source;
  readonly deviceId: string;
  readonly linkContext: LinkContext | null;
  readonly state: { snapshot: DeviceSnapshot | null };
  readonly openDevice?: ((serial: string) => void) | null;
}

/** A unit of the hub as the step lists it. */
export interface UnitInfo {
  serial: string;
  name: string;
  online: boolean;
  /** This screen's own DSPi. */
  own: boolean;
  /** Its outputs (index, name), once its core has read it. */
  outputs: { index: number; name: string }[];
  /** Its core could not be opened, in words. */
  error: string;
  opening: boolean;
}

/** The second Keep's question after a failed verify. */
export interface KeepQuestion {
  since: number;
  text: string;
}

export interface SystemDeskOptions {
  /** A new core Worker (default: the page's worker.js). */
  spawn?: () => UnitCore;
  tokens?: HubTokens;
  env?: () => CaptureEnv;
  now?: () => number;
}

function defaultSpawn(): UnitCore {
  return CoreClient.spawn(new URL("./worker.js", import.meta.url)) as unknown as UnitCore;
}

export class SystemDesk {
  draft: SystemDraft = initialDraft();
  units: UnitInfo[] = [];
  /** The hub serves `system_measure`. */
  hubHas = false;
  hubRole = "";
  /** Why this step cannot run here at all ("" it can): no hub, the hub without the family. */
  unavailable = "";
  preflight: MeasureSystemPreflightRecord | null = null;
  state: MeasureSystemStateRecord | null = null;
  result: MeasureSystemResultRecord | null = null;
  proposal: MeasureSystemProposalRecord | null = null;
  applied: MeasureSystemAppliedRecord | null = null;
  question: KeepQuestion | null = null;
  error = "";
  errorKey = "";
  said = "";
  busy = "";
  /** The setup the last measurement ran with (the checks of its summed runs use it). */
  measured: MeasureSystemSetupRecord | null = null;
  capture: Capture | null = null;
  /** The browser microphone to use ("" its default) and the calibration document. */
  micDeviceId = "";
  micDocument = "";
  // -- WP ATDEMO --
  /** The demo hub's simulated microphone (only `uconsole --demo hub ... --with-measure-units`). */
  sim: MeasureSystemSimInfoRecord | null = null;
  /** Measure with it (the default where it is offered, as P5WMS's demo). */
  useSim = false;
  private readonly cores = new Map<string, UnitCore>();
  private readonly listeners = new Set<() => void>();
  private unsubscribe: (() => void) | null = null;
  private alive = false;
  /** Bumped when the tab is left: a start still on its way then knows. */
  private epoch = 0;
  private readonly spawn: () => UnitCore;
  private readonly tokens: HubTokens;
  private readonly env: () => CaptureEnv;
  private readonly now: () => number;

  constructor(
    readonly session: SystemSession,
    opts: SystemDeskOptions = {},
  ) {
    this.spawn = opts.spawn ?? defaultSpawn;
    this.tokens = opts.tokens ?? new LocalHubTokens(safeLocalStorage());
    this.env = opts.env ?? browserCaptureEnv;
    this.now = opts.now ?? (() => Date.now());
  }

  private get core(): SystemCore {
    return this.session.core;
  }

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private changed(): void {
    for (const l of [...this.listeners]) l();
  }

  /** A run, a summed check or a verify holds the hub's session. */
  get running(): boolean {
    return !!this.state?.active;
  }

  /** The second Keep may be pressed now. */
  confirmReady(): boolean {
    return !!this.question && this.now() - this.question.since >= CONFIRM_MS;
  }

  /** Why nothing may start now, or "" (the placement first, then the form). */
  get blocker(): string {
    return this.unavailable || draftProblem(this.draft);
  }

  // ---- the tab ----

  /** The tab is shown: the hub's family and role, its units, what the core still holds. */
  async enter(): Promise<void> {
    this.alive = true;
    this.unsubscribe?.();
    this.unsubscribe = this.core.subscribe((ev) => {
      // The Worker's events beside the console's own (`CoreEvent` does not list them).
      const e: unknown = ev;
      if (isMeasureSystemEvent(e)) void this.onState(e.state);
    });
    const [caps, role] = await Promise.all([
      this.core.call("hubCaps", undefined).catch(() => null),
      this.core.call("hubRole", undefined).catch(() => ({ role: "" })),
    ]);
    this.hubRole = role.role;
    this.hubHas = !!caps?.onHub && (caps.caps ?? []).includes("system_measure");
    if (!this.hubHas) {
      this.unavailable = "Measuring several units together needs the DSPi hub: it starts every unit's generator on one shared sample counter.";
      this.errorKey = KEYS.needsHub;
      this.changed();
      return;
    }
    this.unavailable = "";
    this.errorKey = "";
    // -- WP ATDEMO -- the demo hub's simulated microphone, where the hub has one
    this.sim = await this.core.call("measureSystemSimInfo", undefined).catch(() => null);
    this.useSim = !!this.sim?.available;
    const devices: LinkDeviceRecord[] = await this.core.call("linkDevices", undefined).catch(() => []);
    if (!this.alive) return;
    const own = this.session.deviceId;
    this.units = devices.map((d) => ({
      serial: d.serial,
      name: d.name || d.serial,
      online: d.state === "online",
      own: d.serial === own,
      outputs: d.serial === own ? this.ownOutputs() : [],
      error: "",
      opening: false,
    }));
    // What the core still holds from earlier on this console: its session, its result.
    this.state = await this.core.call("measureSystemState", undefined).catch(() => null);
    this.applied = await this.core.call("measureSystemApplied", undefined).catch(() => null);
    await this.loadResult(true);
    await this.preparePort();
    this.changed();
  }

  /** The tab is gone: whatever it started is stopped, the units' cores closed. */
  async leave(): Promise<void> {
    this.alive = false;
    this.epoch++;
    this.unsubscribe?.();
    this.unsubscribe = null;
    if (this.state?.active) await this.core.call("measureSystemStop", undefined).catch(() => null);
    await this.closeMic();
    if (this.port) {
      this.port = null;
      await this.core.call("measureSystemCaptureClose", undefined).catch(() => null);
    }
    const cores = [...this.cores.values()];
    this.cores.clear();
    await Promise.all(cores.map((c) => c.dispose().catch(() => undefined)));
  }

  private ownOutputs(): { index: number; name: string }[] {
    const snap = this.session.state.snapshot;
    return snap ? outputRows(snap.state, snap.outputLabels).map((r) => ({ index: r.index, name: `${r.descriptor} ${r.name}` })) : [];
  }

  // ---- the form ----

  // -- WP ATDEMO --
  /** Measure with the demo hub's simulated microphone (`on`) or the browser's. */
  setUseSim(on: boolean): void {
    this.useSim = on && !!this.sim?.available;
    this.changed();
  }

  /** The simulated microphone measures now. */
  get simOn(): boolean {
    return this.useSim && !!this.sim?.available;
  }

  setDraft(d: SystemDraft): void {
    const before = new Set(this.draft.units.map((u) => u.serial));
    const after = new Set(d.units.map((u) => u.serial));
    this.draft = d;
    this.preflight = null;
    for (const s of after) if (!before.has(s)) void this.openUnit(s);
    for (const s of before) if (!after.has(s)) void this.closeUnit(s);
    this.changed();
  }

  private unit(serial: string): UnitInfo | undefined {
    return this.units.find((u) => u.serial === serial);
  }

  /** The unit's own core: this screen's for its own DSPi, else a Worker of its own on the hub. */
  private async openUnit(serial: string): Promise<void> {
    const u = this.unit(serial);
    if (!u || u.own || this.cores.has(serial)) return;
    u.opening = true;
    u.error = "";
    this.changed();
    let core: UnitCore | null = null;
    try {
      const src = this.session.source;
      if (src.kind !== "hub") throw new Error("this console is not connected through a hub");
      const ctx = this.session.linkContext ?? (await linkContext().catch((): LinkContext => ({ loc: null, proxy: false })));
      const url = routeLink(src.url, ctx);
      const saved = await this.tokens.get(src.url);
      core = this.spawn();
      const hello = await core.call("linkConnect", { url, token: saved?.token ?? "", hubId: saved?.hubId ?? "" });
      if (hello.needsPairing) throw new Error("the hub asks this console to pair again: open the unit in the console once, then choose it here");
      const snap = await core.call("open", { kind: "hub", url, deviceId: serial });
      if (!this.alive || !this.draft.units.some((x) => x.serial === serial)) {
        await core.dispose().catch(() => undefined);
        return;
      }
      this.cores.set(serial, core);
      u.outputs = outputRows(snap.state, snap.outputLabels).map((r) => ({ index: r.index, name: `${r.descriptor} ${r.name}` }));
    } catch (e) {
      await core?.dispose().catch(() => undefined);
      u.error = `${serial}: ${refusalWords(e).text}`;
    } finally {
      u.opening = false;
      this.changed();
    }
  }

  private async closeUnit(serial: string): Promise<void> {
    const c = this.cores.get(serial);
    this.cores.delete(serial);
    const u = this.unit(serial);
    if (u && !u.own) u.outputs = [];
    await c?.dispose().catch(() => undefined);
  }

  /** A unit's own core, or why there is none. */
  private coreOf(serial: string): SystemCore {
    if (serial === this.session.deviceId) return this.core;
    const c = this.cores.get(serial);
    if (!c) throw new Error(`${serial}: its own connection is not open yet (choose it again)`);
    return c;
  }

  /**
   * Every unit's fresh check from its own core (review AT-C F2): asked right before each call
   * that takes `checks`. A unit whose check fails names itself.
   */
  async checksFor(setup: MeasureSystemSetupRecord): Promise<MeasureSystemUnitCheckRecord[]> {
    const out: MeasureSystemUnitCheckRecord[] = [];
    for (const unit of setup.units) {
      try {
        out.push(await this.coreOf(unit.serial).call("measureSystemUnitCheck", { setup, unit }));
      } catch (e) {
        throw new Error(`${unit.serial}: ${refusalWords(e).text}`);
      }
    }
    return out;
  }

  // ---- what plays ----

  private clear(): void {
    this.error = "";
    this.errorKey = "";
    this.said = "";
  }

  fail(e: unknown): void {
    const r = refusalWords(e);
    this.error = r.text;
    this.errorKey = r.key;
    this.changed();
  }

  /** The units' checks and the core's preflight; nothing plays. */
  async check(): Promise<MeasureSystemPreflightRecord | null> {
    this.clear();
    const why = this.blocker;
    if (why) {
      this.error = why;
      this.changed();
      return null;
    }
    this.busy = "Checking every unit";
    this.changed();
    try {
      const setup = setupOf(this.draft, 48_000, this.micDocument);
      const checks = await this.checksFor(setup);
      this.preflight = await this.core.call("measureSystemPreflight", { setup, checks });
      return this.preflight;
    } catch (e) {
      this.preflight = null;
      this.fail(e);
      return null;
    } finally {
      this.busy = "";
      this.changed();
    }
  }

  /** The microphone, every unit's fresh check, then the core's start. False when nothing started. */
  async start(): Promise<boolean> {
    this.clear();
    const why = this.blocker;
    if (why || this.running) {
      this.error = why || "A measurement across units is already running.";
      this.changed();
      return false;
    }
    const epoch = this.epoch;
    this.busy = "Starting";
    this.changed();
    try {
      // Firefox starts an audio context only close to the click: the microphone comes first.
      const rate = await this.micForRun(); // -- WP ATDEMO --
      const setup = setupOf(this.draft, rate, this.micDocument);
      const checks = await this.checksFor(setup);
      this.preflight = await this.core.call("measureSystemPreflight", { setup, checks });
      if (!this.preflight.clear) {
        await this.closeMic();
        return false;
      }
      this.result = null;
      this.proposal = null;
      this.applied = null;
      this.question = null;
      const fresh = await this.checksFor(setup);
      this.state = await this.core.call("measureSystemStart", { setup, checks: fresh });
      this.measured = setup;
      if (!this.alive || epoch !== this.epoch) {
        await this.core.call("measureSystemStop", undefined).catch(() => null);
        await this.closeMic();
        return false;
      }
      return true;
    } catch (e) {
      await this.closeMic();
      this.fail(e);
      return false;
    } finally {
      this.busy = "";
      this.changed();
    }
  }

  /** Stop: `system.measure.abort` (every unit stopped, every START cancelled), the microphone. */
  async stop(): Promise<void> {
    try {
      this.state = await this.core.call("measureSystemStop", undefined);
    } catch (e) {
      this.fail(e);
    }
    await this.closeMic();
    this.changed();
  }

  /** The summed check (RC5f) or the verify after an apply: fresh checks, the microphone, the runs. */
  async summed(kind: "sum" | "verify"): Promise<boolean> {
    this.clear();
    const setup = this.measured;
    if (!setup) {
      this.error = "Measure the group first.";
      this.changed();
      return false;
    }
    if (placementProblem(this.draft.placement)) {
      this.error = placementProblem(this.draft.placement);
      this.changed();
      return false;
    }
    const epoch = this.epoch;
    this.busy = kind === "sum" ? "Starting the summed check" : "Starting the verify";
    this.changed();
    try {
      await this.micForRun(); // -- WP ATDEMO --
      const checks = await this.checksFor(setup);
      this.state = await this.core.call(kind === "sum" ? "measureSystemSumCheck" : "measureSystemVerify", { checks });
      if (!this.alive || epoch !== this.epoch) {
        await this.core.call("measureSystemStop", undefined).catch(() => null);
        await this.closeMic();
        return false;
      }
      return true;
    } catch (e) {
      await this.closeMic();
      this.fail(e);
      return false;
    } finally {
      this.busy = "";
      this.changed();
    }
  }

  // ---- the proposal ----

  /** Apply (Admin): never offered below it; the core refuses it too. */
  async apply(): Promise<boolean> {
    this.clear();
    const p = this.proposal;
    if (!p) return false;
    if (this.hubRole !== "admin") {
      this.error = "Applying an alignment across units needs the Admin role.";
      this.changed();
      return false;
    }
    this.busy = "Applying";
    this.changed();
    try {
      this.applied = await this.core.call("measureSystemApply", { proposalId: p.id });
      this.said = this.applied.message || "Applied: measure the summed runs again to verify it, then Keep or Revert.";
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    } finally {
      this.busy = "";
      this.changed();
    }
  }

  /**
   * Keep, the first press. After a verify that did not pass the core refuses it once: its words
   * (`applied.message`) open the second question; only the person's own second press, at least
   * `CONFIRM_MS` later, keeps (`confirmKeep`). Nothing presses it by itself.
   */
  async keep(): Promise<boolean> {
    return this.sendKeep(false);
  }

  async confirmKeep(): Promise<boolean> {
    if (!this.question || !this.confirmReady()) return false;
    return this.sendKeep(true);
  }

  cancelKeep(): void {
    this.question = null;
    this.changed();
  }

  private async sendKeep(second: boolean): Promise<boolean> {
    this.clear();
    this.busy = "Keeping";
    this.changed();
    try {
      this.applied = await this.core.call("measureSystemKeep", undefined);
      this.question = null;
      this.said = this.applied.message || "Kept.";
      return this.applied.stage === "kept";
    } catch (e) {
      const a = await this.core.call("measureSystemApplied", undefined).catch(() => null);
      if (a) this.applied = a;
      if (a?.stage === "verified" && a.verified === false) {
        // The core's refusal is the question; a second press too early keeps it open as it was.
        if (!second || !this.question) this.question = { since: this.now(), text: a.message || refusalWords(e).text };
      } else this.fail(e);
      return false;
    } finally {
      this.busy = "";
      this.changed();
    }
  }

  /** Revert (Admin): the filed model and every unit's delays put back. */
  async revert(): Promise<boolean> {
    this.clear();
    this.question = null;
    if (this.hubRole !== "admin") {
      this.error = "Reverting an alignment across units needs the Admin role.";
      this.changed();
      return false;
    }
    this.busy = "Reverting";
    this.changed();
    try {
      this.applied = await this.core.call("measureSystemRevert", undefined);
      this.said = this.applied.message || "Reverted: the installation and every unit are as they were filed.";
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    } finally {
      this.busy = "";
      this.changed();
    }
  }

  // ---- the session's states ----

  private async onState(st: MeasureSystemStateRecord): Promise<void> {
    const was = this.state;
    this.state = st;
    this.changed();
    if (st.active || !was?.active) return;
    await this.closeMic();
    if (st.phase !== "done") return;
    if (st.purpose === "measure") await this.loadResult(false);
    else {
      this.proposal = await this.core.call("measureSystemProposal", undefined).catch(() => this.proposal);
      this.applied = await this.core.call("measureSystemApplied", undefined).catch(() => this.applied);
      if (st.purpose === "verify" && this.applied?.verified === false) this.said = this.applied.message;
    }
    this.changed();
  }

  /** The result and its proposal as the core has them (quietly none). */
  private async loadResult(quiet: boolean): Promise<void> {
    try {
      this.result = await this.core.call("measureSystemResult", undefined);
      this.proposal = await this.core.call("measureSystemProposal", undefined);
    } catch (e) {
      if (!quiet) this.fail(e);
    }
  }

  // ---- the microphone ----

  /** The Worker's capture port, asked for ahead of a run (Firefox starts audio only close to the click). */
  private port: { port: MessagePort; workletUrl: string } | null = null;

  private async preparePort(): Promise<void> {
    if (this.port || this.capture || !this.alive) return;
    this.port = await this.core.call("measureSystemCaptureOpen", undefined).catch(() => null);
  }

  // -- WP ATDEMO --
  /**
   * The microphone for a run: the demo hub's simulated one (the core hears the hub's room; the
   * page opens none), or the browser's (first, close to the click; the simulated one then off).
   * The rate, Hz.
   */
  private async micForRun(): Promise<number> {
    if (this.simOn) {
      await this.closeMic();
      this.sim = await this.core.call("measureSystemSimSet", { on: true });
      return 48_000;
    }
    const rate = await this.openMic();
    if (this.sim?.on) this.sim = await this.core.call("measureSystemSimSet", { on: false });
    return rate;
  }

  /** The browser's microphone, its blocks straight to this core's Worker. The rate, Hz. */
  async openMic(): Promise<number> {
    if (this.capture && !this.capture.stopped) return this.capture.report.rateHz;
    const open = this.port ?? (await this.core.call("measureSystemCaptureOpen", undefined));
    this.port = null;
    const env = this.env();
    const routed: CaptureEnv = {
      ...env,
      MessageChannel: class {
        readonly port1 = open.port;
        readonly port2 = new env.MessageChannel().port2;
      } as unknown as CaptureEnv["MessageChannel"],
    };
    this.capture = await startCapture({ deviceId: this.micDeviceId || undefined, workletUrl: open.workletUrl, env: routed });
    this.capture.onEvent((e) => {
      if (e.kind === "lost") this.error = "The microphone went away.";
      this.changed();
    });
    this.changed();
    return this.capture.report.rateHz;
  }

  async closeMic(): Promise<void> {
    const c = this.capture;
    this.capture = null;
    if (c) {
      if (!c.stopped) await c.stop().catch(() => undefined);
      // The port goes with the capture; the next one is asked for ahead again.
      await this.core.call("measureSystemCaptureClose", undefined).catch(() => null);
    }
    await this.preparePort();
  }
}
