// WP P2WB1: one session's view of its hub's own families: the inventory, the
// health records, the alarms and (for an Admin) the statistics, through the
// core's typed calls (`hubInventory`, `hubHealth`, `hubAlarms`, `hubStats`).
// Read when started and every `periodMs` while the page is visible, and kept
// current between reads by the hub's `inventory.changed` and `alarm.changed`
// (the session's `watchHub`). One read at a time, by the caller's timer.
//
// It is also the health panel's `HubAlarmSource`: the device alarms about the
// session's device as conditions, acknowledged at the hub.
//
// On a session that is not on a hub, or a hub without these families (an
// upstream hub), `caps` says so and nothing is read: the screen does not show.

import {
  CoreCallError,
  type HubAlarmRecord,
  type HubCapsRecord,
  type HubDeviceRecord,
  type HubHealthRecord,
  type HubServerEventRecord,
  type HubStatsRecord,
  type IdentifyInfoRecord,
  type NoticeRecord,
} from "../../core/protocol";
import type { HubAlarmRuleRecord } from "../../core/hub-settings-protocol"; // -- WP P3WHB --
import type { Session } from "../../ui/session";
import type { Condition, HubAlarmSource } from "../../ui/status-rail/health";
import { noticeOf } from "../../ui/texts";
import { alarmConditions, alarmIdOf, clockTime, sortAlarms, withAlarm, withDevice } from "./model";

/** How often the hub is read while the page is visible, ms. */
export const HUB_POLL_MS = 5000;

// -- WP P3WHB --
/** How often the alarm rules (the health board's thresholds) are read again, ms. */
export const RULES_POLL_MS = 30_000;

/** The seconds an Identify blinks for. */
export const IDENTIFY_SECONDS = 10;

export interface HubView {
  /** Null before the first read of the caps. */
  caps: HubCapsRecord | null;
  /** The role the hub gave this session: `viewer`, `control`, `admin`; "" off a hub. */
  role: string;
  devices: HubDeviceRecord[];
  health: HubHealthRecord[];
  alarms: HubAlarmRecord[];
  /** Admin with `hub_stats`: the hub's statistics; null otherwise. */
  stats: HubStatsRecord | null;
  /** The last read's failure, "" when it went. */
  error: string;
  /** The alarms' notices, by alarm id (the core's words). */
  notices: Record<string, NoticeRecord>;
  // -- WP P3WHB --
  /** The hub's alarm rules (`hubAlarmRulesGet`), the health board's thresholds; [] when not read. */
  rules: HubAlarmRuleRecord[];
}

/** What one action said: a line for the user, and the notice it came with. */
export interface Said {
  text: string;
  bad: boolean;
  notice: NoticeRecord | null;
}

export interface HubFeedOptions {
  periodMs?: number;
  /** Whether the page is visible (tests). */
  visible?: () => boolean;
}

/** Whether the hub has any of the families this screen shows. */
export function hubScreenShown(caps: HubCapsRecord | null): boolean {
  return !!caps?.onHub && (caps.inventory || caps.health || caps.alarms);
}

export class HubFeed {
  view: HubView = { caps: null, role: "", devices: [], health: [], alarms: [], stats: null, error: "", notices: {}, rules: [] /* -- WP P3WHB -- */ };
  private readonly listeners = new Set<() => void>();
  private readonly alarmListeners = new Set<(on: Condition[]) => void>();
  private timer: ReturnType<typeof setTimeout> | null = null;
  private running = false;
  private inflight: Promise<void> | null = null;
  private unwatch: (() => void) | null = null;
  private unhub: (() => void) | null = null;
  private readonly periodMs: number;
  private readonly visible: () => boolean;
  private phase: string | null = null;
  private rulesAt = 0; // -- WP P3WHB -- when the rules were read last (Date.now()), 0 never

  constructor(
    private readonly session: Session,
    opts: HubFeedOptions = {},
  ) {
    this.periodMs = opts.periodMs ?? HUB_POLL_MS;
    this.visible = opts.visible ?? (() => typeof document === "undefined" || document.visibilityState === "visible");
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    this.unhub = this.session.watchHub((events) => this.onEvents(events));
    // A new device bound, or the DSPi Link back: the caps and the role again, at once.
    this.phase = this.session.state.link?.phase ?? null;
    this.unwatch = this.session.watch(() => {
      const p = this.session.state.link?.phase ?? null;
      if (p === this.phase) return;
      this.phase = p;
      if (p === "bound") void this.tick();
    });
    void this.tick();
  }

  stop(): void {
    this.running = false;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.unwatch?.();
    this.unwatch = null;
    this.unhub?.();
    this.unhub = null;
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** One read now (the timer calls it; tests too); during a read, that read. */
  tick(): Promise<void> {
    if (!this.running) return Promise.resolve();
    if (this.inflight) return this.inflight;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.inflight = (async () => {
      try {
        if (this.visible()) await this.read();
      } finally {
        this.inflight = null;
      }
      if (this.running) this.timer = setTimeout(() => void this.tick(), this.periodMs);
    })();
    return this.inflight;
  }

  /** The serial of the session's own device, "" before it is open. */
  openSerial(): string {
    return this.session.state.snapshot?.device.serial || this.session.deviceId || "";
  }

  /** A device's name as the inventory has it, else its serial. */
  deviceName(serial: string): string {
    return this.view.devices.find((d) => d.serial === serial)?.name || serial;
  }

  // -- the actions --

  // -- WP P2WB3 -- Identify for any device on the hub (`hubIdentifySerial`), not only the open one
  /** Identify device `serial` on the hub ("" the open one): its LED blinks. */
  async identify(serial: string, seconds = IDENTIFY_SECONDS): Promise<Said> {
    try {
      const info: IdentifyInfoRecord | null = await this.session.core.call("hubIdentifySerial", { serial, seconds });
      const left = info?.remainingS ?? 0;
      const who = serial ? `${this.deviceName(serial)}: ` : "";
      return { text: `${who}${left > 0 ? `blinking for ${left} s.` : seconds > 0 ? "Identify sent." : "Identify stopped."}`, bad: false, notice: null };
    } catch (e) {
      return failed(e);
    }
  }

  // -- WP P2WB3 -- an Admin's inventory edits, forget and the hub's name
  /** Set a device's location, note and tags (Admin); the table takes the hub's record. */
  async setDetails(serial: string, d: { location: string; note: string; tags: string[] }): Promise<Said> {
    try {
      const rec = await this.session.core.call("hubInventorySet", { serial, location: d.location, note: d.note, tags: d.tags });
      if (rec && typeof rec.serial === "string") this.set({ devices: withDevice(this.view.devices, rec.serial, rec) });
      return { text: `Saved the details of ${rec?.name || serial}.`, bad: false, notice: null };
    } catch (e) {
      return failed(e);
    }
  }

  /** Forget an offline device (Admin); `withData` its snapshots on the hub too. */
  async forget(serial: string, withData: boolean): Promise<Said> {
    const name = this.deviceName(serial);
    try {
      await this.session.core.call("hubInventoryForget", { serial, withData });
      this.set({ devices: withDevice(this.view.devices, serial, null) });
      return { text: withData ? `Forgot ${name} and its snapshots on the hub.` : `Forgot ${name}.`, bad: false, notice: null };
    } catch (e) {
      return failed(e);
    }
  }

  /** Rename the hub (Admin): the session shows the hub's answer. */
  async rename(name: string): Promise<Said> {
    try {
      const now = await this.session.core.call("hubRename", { name });
      this.session.hubRenamed(now);
      return { text: `The hub is now called ${now}.`, bad: false, notice: null };
    } catch (e) {
      return failed(e);
    }
  }

  /** Acknowledge alarm `id` at the hub; the board takes the hub's answer. */
  async acknowledge(id: string): Promise<Said> {
    try {
      const a = await this.session.core.call("hubAlarmAck", { id });
      if (a && typeof a.id === "string") this.setAlarms(withAlarm(this.view.alarms, a));
      return { text: "", bad: false, notice: null };
    } catch (e) {
      return failed(e);
    }
  }

  /** The health panel's source of hub alarms. */
  alarmSource(): HubAlarmSource {
    return {
      subscribe: (fn) => {
        this.alarmListeners.add(fn);
        fn(this.conditions());
        return () => this.alarmListeners.delete(fn);
      },
      acknowledge: async (key) => {
        const id = alarmIdOf(key);
        if (!id) return;
        const a = this.view.alarms.find((x) => x.id === id);
        if (a && a.state !== "raised") return;
        await this.acknowledge(id);
      },
    };
  }

  // -- reading --

  private async read(): Promise<void> {
    const core = this.session.core;
    let caps: HubCapsRecord | null;
    try {
      caps = await core.call("hubCaps", undefined);
    } catch {
      caps = null;
    }
    if (!caps?.onHub) {
      this.set({ caps, role: "", devices: [], health: [], alarms: [], stats: null, error: "", rules: [] /* -- WP P3WHB -- */ });
      this.rulesAt = 0; // -- WP P3WHB --
      return;
    }
    const role = await core.call("hubRole", undefined).then((r) => r?.role ?? "", () => this.view.role);
    const patch: Partial<HubView> = { caps, role, error: "" };
    const errors: string[] = [];
    const take = async <T>(on: boolean, run: () => Promise<T>, put: (v: T) => void, none: () => void) => {
      if (!on) return none();
      try {
        put(await run());
      } catch (e) {
        // Disconnected or refused: what was read before stays.
        if (!(e instanceof CoreCallError && (e.code === "unsupported" || e.code === "denied"))) errors.push(messageOf(e));
        else none();
      }
    };
    await take(caps.inventory, () => core.call("hubInventory", undefined), (v) => (patch.devices = v), () => (patch.devices = []));
    await take(caps.health, () => core.call("hubHealth", undefined), (v) => (patch.health = v), () => (patch.health = []));
    await take(caps.alarms, () => core.call("hubAlarms", {}), (v) => (patch.alarms = sortAlarms(v)), () => (patch.alarms = []));
    await take(caps.hubStats && role === "admin", () => core.call("hubStats", undefined), (v) => (patch.stats = v), () => (patch.stats = null));
    // -- WP P3WHB -- the alarm rules, every RULES_POLL_MS: a failed read keeps the last ones and is not the page's error.
    if (caps.alarms && (!this.rulesAt || Date.now() - this.rulesAt >= RULES_POLL_MS)) {
      try {
        patch.rules = await core.call("hubAlarmRulesGet", undefined);
        this.rulesAt = Date.now();
      } catch {
        // The board shows its cells uncoloured until a read goes.
      }
    } else if (!caps.alarms) patch.rules = [];
    if (errors.length) patch.error = errors[0]!;
    if (!this.running) return;
    this.set(patch);
    if (patch.alarms) await this.readNotices(patch.alarms);
  }

  /** The core's words for each alarm not yet worded (or changed since). */
  private async readNotices(list: HubAlarmRecord[]): Promise<void> {
    const notices = { ...this.view.notices };
    let changed = false;
    for (const a of list) {
      const have = notices[a.id];
      if (have && have.detail === a.message) continue;
      try {
        notices[a.id] = await this.session.core.call("hubAlarmNotice", { alarm: a, time: clockTime(a.raisedAt) });
        changed = true;
      } catch {
        // Without the notice the hub's message is shown.
      }
    }
    if (changed && this.running) this.set({ notices });
  }

  private onEvents(events: HubServerEventRecord[]): void {
    let devices = this.view.devices;
    let alarms = this.view.alarms;
    let dirty = false;
    for (const e of events) {
      if (e.kind === "inventory.changed" && e.serial) {
        devices = withDevice(devices, e.serial, e.device);
        dirty = true;
      } else if (e.kind === "alarm.changed" && e.alarm) {
        alarms = withAlarm(alarms, e.alarm);
        dirty = true;
      }
    }
    if (!dirty) return;
    this.set({ devices, alarms });
    void this.readNotices(alarms);
  }

  private setAlarms(alarms: HubAlarmRecord[]): void {
    this.set({ alarms });
    void this.readNotices(alarms);
  }

  private conditions(): Condition[] {
    const notices = this.view.notices;
    return alarmConditions(this.view.alarms, this.openSerial(), (a) => notices[a.id] ?? null, (s) => this.deviceName(s));
  }

  private set(patch: Partial<HubView>): void {
    this.view = { ...this.view, ...patch };
    for (const l of [...this.listeners]) l();
    const on = this.conditions();
    for (const l of [...this.alarmListeners]) l(on);
  }
}

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function failed(e: unknown): Said {
  const notice = noticeOf(e);
  return { text: notice ? notice.title : messageOf(e), bad: true, notice };
}
