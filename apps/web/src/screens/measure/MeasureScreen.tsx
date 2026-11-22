// WP P5WMS: the Measure screen (PLAN-P5 T1, T5, T7 to T9, T11; S1 to S13 are the core's): the
// setup and the run, the results, the live view with its direct adjustments, and a tab slot for
// P5WAT's auto-tune and documents. Shown in Advanced and Installer (modes.json's `measure`).
//
// One `MeasureDesk` per screen holds what the tabs share: the microphone (the browser's, through
// the core Worker's capture port, or the demo's simulated one), the run's and the live view's
// states as the Worker pushes them, the result document, the calibration and the target. Every
// rule is the core's: the preflight, the ramp, the aborts, the documents. This screen plays
// nothing; the DSPi's generator is the only stimulus, and Stop is shown whenever anything runs.
// Leaving the screen stops whatever it started.

import { PageParts, usePart } from "../../ui/shell/Parts"; // -- WP WEBM3 --
import type { FunctionComponent } from "preact";
import { useEffect, useMemo, useState } from "preact/hooks";
import { CoreCallError } from "../../core/protocol";
import { browserCaptureEnv, listMicrophones, startCapture, type Capture, type CaptureEnv, type MicrophoneInfo } from "../../core/capture";
import {
  isMeasureEvent,
  measureCall,
  type CaptureOpenRecord,
  type CaptureTarget,
  type MeasureRtaAveraging,
  type MeasureRtaPassiveFrameRecord,
  type MeasureLiveSetupRecord,
  type MeasureLiveStateRecord,
  type MeasurePreflightRecord,
  type MeasureSetupRecord,
  type MeasureSimInfoRecord,
  type MeasureStateRecord,
} from "../../core/measure-protocol";
import { useSessionState } from "../../ui/components";
import type { Ask, ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { noticeOf, noticeRow } from "../../ui/texts";
import { outputRows } from "../main/model";
import { LiveTab } from "./live";
import { PassiveTab } from "./passive";
import { playBlocker, type MeasureDraft } from "./model";
import { ResultsTab } from "./results";
import { RunPanel } from "./run";
import { SetupPanel } from "./setup";
import { WAT_TABS } from "./tune"; // -- WP P5WAT --
import { SYSTEM_TABS } from "./system/SystemTab"; // -- WP ATWEB --

/** The microphone choice: the demo's simulated one, or a browser input ("" its default). */
export type MicChoice = { kind: "sim" } | { kind: "browser"; deviceId: string };

/** What the measurement tabs share. */
export class MeasureDesk {
  state: MeasureStateRecord | null = null;
  live: MeasureLiveStateRecord | null = null;
  sim: MeasureSimInfoRecord | null = null;
  mics: MicrophoneInfo[] = [];
  mic: MicChoice = { kind: "browser", deviceId: "" };
  capture: Capture | null = null;
  /** The last failure in words (a refused call, the microphone). */
  error = "";
  /** The result document's text (a run's, a file's), and where it came from. */
  result: string | null = null;
  resultFrom = "";
  /** The `mic` and `target` documents in use ("" none). */
  micDocument = "";
  targetDocument = "";
  targetName = "";
  hubMeasure = false;
  hubRole = "";
  /** The setup form, kept across the tabs. */
  draft: MeasureDraft | null = null;
  /** The last preflight's answer. */
  preflight: MeasurePreflightRecord | null = null;
  private readonly listeners = new Set<() => void>();
  private stopEvents: (() => void) | null = null;
  private alive = false;

  constructor(
    readonly session: Session,
    private readonly env: () => CaptureEnv = browserCaptureEnv,
  ) {}

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private changed(): void {
    for (const l of [...this.listeners]) l();
  }

  get core() {
    return this.session.core;
  }

  /** The screen is shown: events, the simulated microphone's availability, the hub's documents. */
  async start(): Promise<void> {
    this.alive = true;
    this.stopEvents = this.core.subscribe((ev) => {
      // The Worker's events beside the console's own (`CoreEvent` does not list them).
      const e: unknown = ev;
      if (!isMeasureEvent(e)) return;
      // The passive RTA never runs beside the generator: it would show the stimulus.
      if (e.state.active && this.passive) void this.stopPassive();
      if (e.event === "measure") void this.onState(e.state);
      else {
        const was = this.live?.active;
        this.live = e.state;
        // The live view ended by itself (an abort): the microphone with it.
        if (was && !e.state.active) void this.closeMic();
      }
      this.changed();
    });
    const [sim, caps, role, st, live] = await Promise.all([
      measureCall(this.core, "measureSimInfo", undefined).catch(() => null),
      this.core.call("hubCaps", undefined).catch(() => null),
      this.core.call("hubRole", undefined).catch(() => ({ role: "" })),
      measureCall(this.core, "measureState", undefined).catch(() => null),
      measureCall(this.core, "measureLiveState", undefined).catch(() => null),
    ]);
    if (!this.alive) return;
    this.sim = sim;
    this.hubMeasure = !!(caps as { hubMeasure?: boolean } | null)?.hubMeasure;
    this.hubRole = role.role;
    // The simulated microphone answers the generator only: a passive view needs a real one.
    if (sim?.available && !this.passiveOnly) this.mic = { kind: "sim" };
    this.state = st;
    this.live = live;
    this.changed();
    await this.refreshMics();
    await this.preparePort();
  }

  /** This connection may not play the generator: the passive RTA and the results only. */
  get passiveOnly(): boolean {
    return !!playBlocker(this.session.readOnlyReason, this.hubRole);
  }

  /** The Worker's capture port, asked for ahead of a run (see `openMic`), and where it sends. */
  private port: CaptureOpenRecord | null = null;
  private portTo: CaptureTarget = "measure";
  private opening = false;

  private async preparePort(): Promise<void> {
    const to: CaptureTarget = this.passiveOnly ? "passive" : "measure";
    // A new port closes the Worker's earlier one: never while a capture is open or opening.
    if (this.opening || this.capture || (this.port && this.portTo === to) || (this.mic.kind === "sim" && to === "measure")) return;
    try {
      this.port = await measureCall(this.core, "measureCaptureOpen", { to });
      this.portTo = to;
    } catch {
      this.port = null;
    }
  }

  /** The screen is gone: whatever it started is stopped. */
  async stop(): Promise<void> {
    this.alive = false;
    this.stopEvents?.();
    this.stopEvents = null;
    if (this.live?.active) await measureCall(this.core, "measureLiveStop", undefined).catch(() => null);
    if (this.state?.active) await measureCall(this.core, "measureStop", undefined).catch(() => null);
    if (this.passive) await measureCall(this.core, "measureRtaPassiveStop", undefined).catch(() => null);
    this.passive = null;
    await this.closeMic();
  }

  /** The browser's inputs (labels once the permission was given); quietly none without the API. */
  async refreshMics(): Promise<void> {
    try {
      this.mics = await listMicrophones(this.env());
    } catch {
      this.mics = [];
    }
    this.changed();
  }

  setMic(m: MicChoice): void {
    this.mic = m;
    void this.preparePort();
    this.changed();
  }

  setDocs(patch: Partial<Pick<MeasureDesk, "micDocument" | "targetDocument" | "targetName">>): void {
    Object.assign(this, patch);
    this.changed();
  }

  setDraft(d: MeasureDraft): void {
    this.draft = d;
    this.preflight = null;
    this.changed();
  }

  /** The core's preflight of `setup`, kept for the form. */
  async runPreflight(setup: MeasureSetupRecord): Promise<MeasurePreflightRecord | null> {
    try {
      this.preflight = await measureCall(this.core, "measurePreflight", { setup });
    } catch (e) {
      this.preflight = null;
      this.fail(e);
    }
    this.changed();
    return this.preflight;
  }

  setResult(text: string | null, from: string): void {
    this.result = text;
    this.resultFrom = from;
    this.changed();
  }

  fail(e: unknown): void {
    const n = noticeOf(e);
    this.error = n ? `${n.title}: ${n.detail || n.reason}` : e instanceof Error ? e.message : String(e);
    this.changed();
  }

  clearError(): void {
    this.error = "";
    this.changed();
  }

  /** Something runs: the Stop bar shows. */
  get running(): boolean {
    return !!(this.state?.active || this.live?.active || this.passive || (this.capture && !this.capture.stopped));
  }

  /**
   * The microphone for a run: the simulated one (on, the core hears its room), or the browser's
   * (the simulated one off, then the permission, the capture straight to the Worker). The rate
   * the session should expect.
   */
  async openMic(to: CaptureTarget = "measure"): Promise<number> {
    if (this.mic.kind === "sim" && to === "measure") {
      await this.closeMic();
      this.sim = await measureCall(this.core, "measureSimSet", { on: true, seed: 1 });
      return 48_000;
    }
    // Firefox starts an audio context only close to the user's click: nothing slow comes first
    // (the Worker's port is asked for ahead, the simulated microphone switched off after).
    if (this.capture) await this.closeMic();
    this.opening = true;
    try {
      return await this.openBrowserMic(to);
    } finally {
      this.opening = false;
    }
  }

  private async openBrowserMic(to: CaptureTarget): Promise<number> {
    const open = this.port && this.portTo === to ? this.port : await measureCall(this.core, "measureCaptureOpen", { to });
    this.port = null;
    const env = this.env();
    // The worklet posts on the Worker's port: the blocks never pass through the page.
    const routed: CaptureEnv = {
      ...env,
      MessageChannel: class {
        readonly port1 = open.port;
        readonly port2 = new env.MessageChannel().port2;
      } as unknown as CaptureEnv["MessageChannel"],
    };
    const deviceId = this.mic.kind === "browser" ? this.mic.deviceId : "";
    this.capture = await startCapture({ deviceId: deviceId || undefined, workletUrl: open.workletUrl, env: routed });
    this.capture.onEvent((e) => {
      if (e.kind === "lost") this.error = noticeRow(e.noticeKey)?.title ?? "The microphone went away";
      this.changed();
    });
    if (this.sim?.on) this.sim = await measureCall(this.core, "measureSimSet", { on: false });
    void this.refreshMics();
    this.changed();
    return this.capture.report.rateHz;
  }

  async closeMic(): Promise<void> {
    const c = this.capture;
    this.capture = null;
    if (c && !c.stopped) await c.stop().catch(() => undefined);
    if (this.alive) void this.preparePort();
    this.changed();
  }

  // ---- the passive microphone RTA (P5CURV): programme audio, nothing played ----

  /** The passive RTA's last frame while it runs, else null. */
  passive: MeasureRtaPassiveFrameRecord | null = null;
  private passiveTimer: ReturnType<typeof setTimeout> | undefined;

  /** Why the passive RTA may not start now, or "": never beside a run or the live view. */
  get passiveBlocker(): string {
    if (this.state?.active || this.live?.active) return "A measurement or the live view plays the generator now: the passive RTA would show the test signal.";
    return "";
  }

  /** The microphone to the passive RTA (the Worker pushes its blocks), then its frames, twice a second. */
  async startPassive(averaging: MeasureRtaAveraging): Promise<boolean> {
    this.clearError();
    if (this.passiveBlocker) {
      this.error = this.passiveBlocker;
      this.changed();
      return false;
    }
    try {
      const rate = await this.openMic("passive");
      const st = await measureCall(this.core, "measureRtaPassiveStart", { setup: { micDocument: this.draft?.micDocument ?? this.micDocument, captureRateHz: rate, averaging } });
      this.passive = { state: st, rta: null };
      // Review P5-D: as `startRun` (the microphone is not left open behind a screen that is gone).
      if (!this.alive) {
        await this.stopGone();
        return false;
      }
      this.changed();
      this.pollPassive();
      return true;
    } catch (e) {
      await this.closeMic();
      this.fail(e);
      return false;
    }
  }

  private pollPassive(): void {
    clearTimeout(this.passiveTimer);
    this.passiveTimer = setTimeout(async () => {
      if (!this.passive) return;
      try {
        const f = await measureCall(this.core, "measureRtaPassiveFrame", undefined);
        if (!this.passive) return;
        this.passive = f;
        this.changed();
      } catch {
        // The next turn tries again.
      }
      if (this.passive && this.alive) this.pollPassive();
    }, 500);
  }

  async stopPassive(): Promise<void> {
    clearTimeout(this.passiveTimer);
    const was = this.passive;
    this.passive = null;
    if (was) await measureCall(this.core, "measureRtaPassiveStop", undefined).catch(() => null);
    await this.closeMic();
  }

  async passiveHold(on: boolean): Promise<void> {
    try {
      await measureCall(this.core, "measureRtaPassiveHold", { on });
    } catch (e) {
      this.fail(e);
    }
  }

  async passiveAveraging(averaging: MeasureRtaAveraging): Promise<void> {
    if (!this.passive) return;
    try {
      await measureCall(this.core, "measureRtaPassiveAveraging", { averaging });
    } catch (e) {
      this.fail(e);
    }
  }

  /** Preflight, the microphone, then the run (the Worker turns it). False when it did not start. */
  async startRun(setup: MeasureSetupRecord): Promise<boolean> {
    this.clearError();
    try {
      if (this.live?.active) this.live = await measureCall(this.core, "measureLiveStop", undefined);
      const rate = await this.openMic();
      const run = { ...setup, captureRateHz: rate };
      this.preflight = await measureCall(this.core, "measurePreflight", { setup: run });
      if (!this.preflight.clear) {
        await this.closeMic();
        this.changed();
        return false;
      }
      this.result = null;
      this.state = await measureCall(this.core, "measureStart", { setup: run });
      // Review P5-D: the screen was left while this started (the microphone's permission question
      // can take long): nothing keeps playing without its Stop.
      if (!this.alive) {
        await this.stopGone();
        return false;
      }
      this.changed();
      return true;
    } catch (e) {
      await this.closeMic();
      this.fail(e);
      return false;
    }
  }

  async nextPosition(): Promise<void> {
    try {
      this.state = await measureCall(this.core, "measureNextPosition", undefined);
      this.changed();
    } catch (e) {
      this.fail(e);
    }
  }

  /** Stop: the run and the live view (one STOP_NOW each in the core), the microphone. */
  async stopAll(): Promise<void> {
    if (this.passive) await this.stopPassive();
    // Review P5-D: each stop on its own, so a live view that does not answer its stop never keeps
    // the run's stop from being sent.
    try {
      if (this.live?.active) this.live = await measureCall(this.core, "measureLiveStop", undefined);
    } catch (e) {
      this.fail(e);
    }
    try {
      // A passive-only connection has no run to stop (and sends nothing).
      if (!this.passiveOnly || this.state?.active) this.state = await measureCall(this.core, "measureStop", undefined);
    } catch (e) {
      this.fail(e);
    }
    await this.closeMic();
    this.changed();
  }

  /** Review P5-D: something started after the screen was left: stopped as `stop` would have. */
  private async stopGone(): Promise<void> {
    if (this.live?.active) await measureCall(this.core, "measureLiveStop", undefined).catch(() => null);
    if (this.state?.active) await measureCall(this.core, "measureStop", undefined).catch(() => null);
    if (this.passive) await measureCall(this.core, "measureRtaPassiveStop", undefined).catch(() => null);
    this.passive = null;
    await this.closeMic();
  }

  async startLive(setup: MeasureLiveSetupRecord): Promise<boolean> {
    this.clearError();
    try {
      const rate = await this.openMic();
      // The live view forgets a finished run: its result is taken first.
      if (this.state?.resultReady && !this.result) await this.takeResult();
      this.live = await measureCall(this.core, "measureLiveStart", { setup: { ...setup, captureRateHz: rate } });
      // Review P5-D: as `startRun`.
      if (!this.alive) {
        await this.stopGone();
        return false;
      }
      this.changed();
      return true;
    } catch (e) {
      await this.closeMic();
      this.fail(e);
      return false;
    }
  }

  async stopLive(): Promise<void> {
    try {
      this.live = await measureCall(this.core, "measureLiveStop", undefined);
    } catch (e) {
      this.fail(e);
    }
    await this.closeMic();
  }

  private async onState(st: MeasureStateRecord): Promise<void> {
    const was = this.state;
    this.state = st;
    // A live view's runs are the live view's: the result and the microphone are its own.
    if (this.live?.active || st.active || !was?.active) return;
    if (st.resultReady) await this.takeResult();
    await this.closeMic();
  }

  private async takeResult(): Promise<void> {
    try {
      this.result = await measureCall(this.core, "measureResult", undefined);
      this.resultFrom = "run";
    } catch (e) {
      if (!(e instanceof CoreCallError)) throw e;
    }
    this.changed();
  }
}

/** Re-render on the desk's changes. */
export function useDesk(d: MeasureDesk): MeasureDesk {
  const [, bump] = useState(0);
  useEffect(() => d.watch(() => bump((n) => n + 1)), [d]);
  return d;
}

/** What each tab of the screen gets. */
export interface MeasureTabProps {
  session: Session;
  desk: MeasureDesk;
  ask: Ask;
  /** Why this connection may not play the generator, or "". */
  blocked: string;
  outputName: (o: number) => string;
  outputs: number[];
  /** Show another tab. */
  show: (id: string) => void;
}

export interface MeasureTab {
  id: string;
  label: string;
  View: FunctionComponent<MeasureTabProps>;
  /** Shown to a connection that may not play the generator. */
  passive?: boolean;
}

function MeasureTabView(props: MeasureTabProps) {
  return (
    <>
      <SetupPanel {...props} />
      <RunPanel {...props} />
    </>
  );
}

/** The screen's tabs. P5WAT adds its auto-tune and documents tabs in the slot below. */
export const MEASURE_TABS: MeasureTab[] = [
  { id: "measure", label: "Measure", View: MeasureTabView },
  { id: "results", label: "Results", View: ResultsTab, passive: true },
  { id: "live", label: "Live", View: LiveTab },
  ...SYSTEM_TABS, // -- WP ATWEB -- several units on one time base
  // -- tab slot: WP P5WAT -- (auto-tune and the documents)
  ...WAT_TABS, // -- WP P5WAT --
];

const PASSIVE_TAB: MeasureTab = { id: "passive", label: "Passive RTA", View: PassiveTab, passive: true };

export function MeasureScreen(props: ScreenProps) {
  const { session } = props;
  const st = useSessionState(session);
  const desk = useMemo(() => new MeasureDesk(session), [session]);
  useDesk(desk);
  useEffect(() => {
    void desk.start();
    return () => void desk.stop();
  }, [desk]);
  const snap = st.snapshot;
  const rows = useMemo(() => (snap ? outputRows(snap.state, snap.outputLabels) : []), [snap]);
  const outputName = (o: number) => {
    const r = rows.find((x) => x.index === o);
    return r ? `${r.descriptor} ${r.name}` : `Output ${o + 1}`;
  };
  const blocked = playBlocker(session.readOnlyReason, desk.hubRole);
  const tabs = blocked ? [PASSIVE_TAB, ...MEASURE_TABS.filter((t) => t.passive)] : MEASURE_TABS;
  // -- WP WEBM3 -- the tabs are the page's parts, in its head; the one on show kept per session
  const [chosen, setChosen] = usePart(session, "measure", tabs.map((t) => t.id));
  const tab = tabs.find((t) => t.id === chosen) ?? tabs[0]!;
  if (!snap) return null;
  const p: MeasureTabProps = { session, desk, ask: props.ask, blocked, outputName, outputs: rows.map((r) => r.index), show: setChosen };
  return (
    <div class="measure-screen" role="region" aria-label="Measure" style={{ display: "flex", flexDirection: "column", gap: "10px", minWidth: 0 }}>
      {desk.running ? (
        <div class="banner" role="status" aria-label="Measuring" style={{ position: "sticky", top: 0, zIndex: 2, display: "flex", flexWrap: "wrap", gap: "8px", alignItems: "center" }}>
          <span style={{ flex: "1 1 12em" }}>
            {desk.passive ? "The microphone is open for the passive RTA; nothing is played." : desk.live?.active ? "The live view is playing the DSPi's generator." : desk.state?.active ? "Measuring with the DSPi's generator." : "The microphone is open."}
          </span>
          <button type="button" class="danger" onClick={() => void desk.stopAll()}>
            Stop
          </button>
        </div>
      ) : null}
      {desk.error ? (
        <p class="error" role="alert">
          {desk.error}
        </p>
      ) : null}
      <PageParts label="Measurement" parts={tabs} current={tab.id} onPart={setChosen} />
      <div role="tabpanel" aria-labelledby={`${tab.id}-part-tab`} style={{ display: "contents" }}>
        <tab.View {...p} />
      </div>
    </div>
  );
}
