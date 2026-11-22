// WP SW3: one session's view of the system: the hub's audio-link model and
// status (`hubSystem`, the hub's `system.*`), or, for a device the console
// holds itself or a hub without an installation, that one device as a unit.
// The System view, the scene bar and the status rail share one feed per
// session; it polls `system.status` every few seconds while the page is
// visible (the hub answers it at most once a second per session).

import { CoreCallError } from "../../core/protocol";
import type { Session } from "../../ui/session";
import {
  sceneTaken,
  singleUnit,
  type AlignReport,
  type Role,
  type SceneReport,
  type SystemModel,
  type SystemStatus,
  type WriteResult,
} from "./model";

/** The last scene recalled from this session, with what each unit said. */
export interface SceneRun {
  scene: string;
  name: string;
  report: SceneReport | null;
  /** Why it did not run (a refused op is named here, in the hub's words). */
  error: string;
  running: boolean;
  /** -- fixup sw3x -- why the hub took none of its writes ("" when taken). */
  refusal: string;
  /**
   * -- WP HWLK1F -- what a taken recall did, in the core's sentence (`sceneOutcome`),
   * the same every client shows: at once on units without the audio link, at the
   * frame on the others. "" while running or when not taken.
   */
  outcome: string;
}

/** `system.get`'s answer. -- fixup sw3x -- with the session's role now. */
interface Got {
  model: SystemModel;
  revision: number;
  role?: string;
}

export interface FeedState {
  /** `hub`: an installation on the hub; `single`: this device alone. */
  kind: "loading" | "hub" | "single";
  model: SystemModel | null;
  status: SystemStatus | null;
  role: Role;
  /** The last failure to read or write, "" when none. */
  error: string;
  scene: SceneRun | null;
  /** -- fixup sw3x -- the last scene that was recalled (taken), not merely asked for. */
  recalled: { scene: string; name: string } | null;
  /** -- fixup sw3x -- the model's revision as last read (`system.get`). */
  revision: number;
  /** -- WP HWLK1F -- `message`: a written alignment in the core's sentence (`alignOutcome`). */
  align: { report: AlignReport | null; error: string; running: boolean; message: string } | null;
  /** -- WP HWLK1F -- the hub's warnings for the model last saved from this page (units without the audio link). */
  warnings: string[];
  /** The system mute's last writes. */
  mute: { writes: WriteResult[]; error: string } | null;
}

/** How often `system.status` is asked while the page is visible. */
export const POLL_MS = 3000;
/** The hub's limit, with a margin. */
const STATUS_GAP_MS = 1100;

export class SystemFeed {
  state: FeedState = { kind: "loading", model: null, status: null, role: "", error: "", scene: null, recalled: null, revision: 0, align: null, mute: null, warnings: [] };
  private readonly listeners = new Set<() => void>();
  private timer: ReturnType<typeof setTimeout> | undefined;
  private alive = true;
  private lastStatus = 0;
  private reading = false;
  /** -- fixup sw3x -- the frame of the last superframe scene this page had taken. */
  private heldFrame: number | null = null;
  /** System views showing this feed: a device on its own is read only while one is. */
  private viewers = 0;

  constructor(
    private readonly session: Session,
    private readonly pollMs = POLL_MS,
  ) {}

  /** Whether this session is on a hub (a hub's installation may be shown). */
  get onHub(): boolean {
    return this.session.source.kind === "hub";
  }

  /** Whether the session may write at all (a viewer's page or read-only firmware may not). */
  get readOnly(): boolean {
    return !!this.session.readOnlyReason;
  }

  start(): void {
    void this.tick();
  }

  stop(): void {
    this.alive = false;
    if (this.timer) clearTimeout(this.timer);
    this.listeners.clear();
  }

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /**
   * A System view shows this feed while the returned function is not called.
   * A device on its own is read only then: its rail needs nothing, and an idle
   * session makes no calls but its meters. A hub is read always, for the
   * rail's alarms and the system mute.
   */
  show(): () => void {
    this.viewers++;
    if (!this.onHub) void this.refresh();
    return () => {
      this.viewers--;
    };
  }

  private async tick(): Promise<void> {
    if (!this.alive) return;
    const visible = typeof document === "undefined" || document.visibilityState === "visible";
    if (visible && this.session.state.status === "open" && (this.onHub || this.viewers > 0)) await this.refresh();
    if (this.alive) this.timer = setTimeout(() => void this.tick(), this.pollMs);
  }

  /** Read everything again: the model when it changed, the status, the role. */
  async refresh(): Promise<void> {
    if (this.reading) return;
    this.reading = true;
    try {
      if (this.onHub) await this.readHub();
      else await this.readSingle();
    } finally {
      this.reading = false;
    }
  }

  private async readHub(): Promise<void> {
    const core = this.session.core;
    try {
      if (!this.state.role) {
        const { role } = await core.call("hubRole", undefined);
        this.set({ role: role as Role });
      }
      if (!this.state.model) {
        const got = (await core.call("hubSystem", { t: "system.get" })) as unknown as Got;
        if (got.role) this.set({ role: got.role as Role });
        if (!got.model.units.length && !got.model.senders.length) {
          // A hub without an installation: this device alone, as for USB.
          await this.readSingle();
          return;
        }
        this.set({ model: got.model, revision: got.revision, kind: "hub" });
      }
      if (this.state.kind !== "hub") return this.readSingle();
      const wait = this.lastStatus + STATUS_GAP_MS - Date.now();
      if (wait > 0) await new Promise((r) => setTimeout(r, wait));
      this.lastStatus = Date.now();
      const status = (await core.call("hubSystem", { t: "system.status" })) as unknown as SystemStatus;
      if (status.revision !== this.state.revision) {
        const got = (await core.call("hubSystem", { t: "system.get" })) as unknown as Got;
        this.set({ model: got.model, revision: got.revision });
      }
      // -- fixup sw3x -- the role as the hub judges it now: a live change shows at the next poll.
      this.set(status.role ? { status, role: status.role as Role, error: "" } : { status, error: "" });
    } catch (e) {
      // `rate_limited`: another read came first; the next poll asks again.
      if (!(e instanceof CoreCallError && e.code === "rate_limited")) this.set({ error: message(e) });
    }
  }

  private async readSingle(): Promise<void> {
    const snap = this.session.state.snapshot;
    if (!snap) return;
    const core = this.session.core;
    let link: Parameters<typeof singleUnit>[1] = null;
    try {
      const info = await core.call("alinkInfo", undefined);
      if (info) {
        const [status, slots] = await Promise.all([core.call("alinkStatus", undefined), core.call("alinkSlots", undefined)]);
        link = { role: info.role, state: status.state, slots, dLocalMin: info.dLocalMin, dLocalMax: info.dLocalMax };
      }
    } catch {
      // A unit whose alink cannot be read is shown without it.
    }
    const one = singleUnit({ name: snap.device.name, serial: snap.device.serial ?? "" }, link, this.session.state.status === "open");
    this.set({ kind: "single", model: one.model, status: one.status, error: "" });
  }

  /** `system.align`: compute (`apply` false) or write the alignment. Admin. */
  async align(apply: boolean): Promise<void> {
    this.set({ align: { report: this.state.align?.report ?? null, error: "", running: true, message: "" } });
    try {
      const report = (await this.session.core.call("hubSystem", { t: "system.align", apply })) as unknown as AlignReport;
      // -- WP HWLK1F -- what was written and what was skipped, in the core's words
      const said = apply ? ((await this.session.core.call("alignOutcome", { report, model: this.state.model })) as { message: string }) : null;
      this.set({ align: { report, error: "", running: false, message: said?.message ?? "" } });
      void this.refresh();
    } catch (e) {
      this.set({ align: { report: null, error: message(e), running: false, message: "" } });
    }
  }

  /**
   * `system.set`: the whole model, as the hub keeps it. Admin. -- fixup sw3x --
   * `base` is the revision the edits started from: the hub refuses the model
   * (`conflict`) when it has a newer one, so nobody else's change is lost.
   */
  async setModel(model: SystemModel, base: number): Promise<"ok" | "conflict" | "failed"> {
    try {
      const r = (await this.session.core.call("hubSystem", { t: "system.set", model, revision: base })) as { warnings?: unknown };
      await this.reloadModel();
      // -- WP HWLK1F -- kept, with the hub's warnings (a unit without the audio link)
      const warnings = Array.isArray(r?.warnings) ? r.warnings.filter((w): w is string => typeof w === "string") : [];
      this.set({ error: "", warnings });
      void this.refresh();
      return "ok";
    } catch (e) {
      if (e instanceof CoreCallError && e.code === "conflict") {
        await this.reloadModel().catch(() => undefined);
        return "conflict";
      }
      this.set({ error: message(e) });
      return "failed";
    }
  }

  /** The hub's model and revision now. */
  async reloadModel(): Promise<void> {
    const got = (await this.session.core.call("hubSystem", { t: "system.get" })) as unknown as Got;
    this.set(got.role ? { model: got.model, revision: got.revision, role: got.role as Role } : { model: got.model, revision: got.revision });
  }

  // -- WP INSTW --
  /**
   * Read the installation again after the Installation tab saved it: a hub that had none
   * (this device alone) may have one now, and one that had one may have none.
   */
  async reread(): Promise<void> {
    if (!this.onHub) return;
    this.set({ model: null, kind: "loading" });
    await this.refresh();
  }

  /** `system.scene.apply`: recall a scene at one alink frame on every unit. */
  async recall(sceneId: string): Promise<void> {
    const name = this.state.model?.scenes.find((s) => s.id === sceneId)?.name || sceneId;
    this.set({ scene: { scene: sceneId, name, report: null, error: "", running: true, refusal: "", outcome: "" } });
    try {
      const report = (await this.session.core.call("hubSystem", { t: "system.scene.apply", scene: sceneId })) as unknown as SceneReport;
      if (sceneTaken(report)) {
        if (report.plan.path === "superframe") this.heldFrame = report.plan.frame;
        // -- WP HWLK1F -- the core's sentence: "on time on every unit" only when it is true
        const said = await this.session.core.call("sceneOutcome", {
          scene: sceneId,
          report,
          model: this.state.model,
          rateHz: this.state.status?.alignment.rateHz || undefined,
        });
        this.set({ scene: { scene: sceneId, name, report, error: "", running: false, refusal: "", outcome: said.message }, recalled: { scene: sceneId, name } });
      } else {
        // -- fixup sw3x -- nothing was taken: not recalled. Through the sender, say
        // which scene's frame it still holds (the status's, else the one this page set).
        const sender = report.writes[0]?.serial;
        await this.refresh();
        const seen = sender ? this.state.status?.observed[sender]?.pendingApplyFrame : null;
        const after = report.frameAfterWrites ?? report.plan.frame - report.plan.leadFrames;
        const pending = seen ?? (this.heldFrame !== null && this.heldFrame > after ? this.heldFrame : null);
        // -- WP WB8 -- the core's words for it, the same every client shows (by the sender's answer)
        const outcome = await this.session.core.call("sceneOutcome", {
          scene: sceneId,
          report,
          model: this.state.model,
          heldFrame: pending,
          rateHz: this.state.status?.alignment.rateHz || undefined,
        });
        this.set({ scene: { scene: sceneId, name, report, error: "", running: false, refusal: outcome.message, outcome: "" } });
      }
    } catch (e) {
      this.set({ scene: { scene: sceneId, name, report: null, error: message(e), running: false, refusal: "", outcome: "" } });
    }
    void this.refresh();
  }

  /** `system.mute`: set or clear the system mute on every reachable sender. */
  async setMute(muted: boolean): Promise<void> {
    try {
      const r = (await this.session.core.call("hubSystem", { t: "system.mute", muted })) as { writes: WriteResult[] };
      this.set({ mute: { writes: r.writes, error: "" } });
    } catch (e) {
      this.set({ mute: { writes: [], error: message(e) } });
    }
    void this.refresh();
  }

  /** `system.alarm.ack`: someone has seen it. */
  async acknowledge(id: number): Promise<void> {
    try {
      await this.session.core.call("hubSystem", { t: "system.alarm.ack", alarm: id });
      const r = (await this.session.core.call("hubSystem", { t: "system.alarms" })) as { alarms: SystemStatus["alarms"] };
      if (this.state.status) this.set({ status: { ...this.state.status, alarms: r.alarms }, error: "" });
    } catch (e) {
      this.set({ error: message(e) });
    }
  }

  private set(patch: Partial<FeedState>): void {
    if (!this.alive) return;
    this.state = { ...this.state, ...patch };
    for (const l of this.listeners) l();
  }
}

function message(e: unknown): string {
  const m = e instanceof Error ? e.message : String(e);
  return m ? m[0]!.toUpperCase() + m.slice(1) : m;
}
