// WP P3WRO: the firmware rollout's logic on the web (PLAN-P3 F6 and F8, section 1.1;
// hub-api.md section 21). An Admin of a hub with the `rollout` cap sees the hub's image
// store (version, platform, the 1.20 V stamp, verified or not), uploads an image, starts
// a rollout on a DSPi or a group, follows it step by step from `fw.rollout.changed`, and
// reads its result (version, extension channel, the core voltage read back, settings
// unchanged or changed), with "Recall the export" when the settings changed.
//
// `RolloutFeed` talks to the hub only through the core's typed calls (`hubFirmwareImages`,
// `hubFirmwarePut`, `hubRolloutStart`, `hubRollouts`, `hubRolloutCancel`,
// `hubSnapshotsRecall`, `hubGroupParamSet`); the rest of this file is pure.

import {
  CoreCallError,
  type HubAlarmRecord,
  type HubCapsRecord,
  type HubFirmwareImageRecord,
  type HubFleetTarget,
  type HubGroupRecord,
  type HubRolloutDeviceRecord,
  type HubRolloutRecord,
  type HubServerEventRecord,
  type NoticeRecord,
} from "../../core/protocol";
import type { Session } from "../../ui/session";
import { noticeOf } from "../../ui/texts";
import { alarmOn } from "./model";

/** The hub's cap on one image, bytes (hub-api.md 21.2); a bigger file is refused before upload. */
export const MAX_IMAGE_BYTES = 8 * 1024 * 1024;

/** The steps of a rollout, in order (hub-api.md 21.3; the core's ROLLOUT_STEPS). */
export const STEPS = ["check", "guard", "export", "bootloader", "find", "flash", "wait", "verify"] as const;

/** The steps before which a cancel still takes effect (it is refused from the bootloader entry on). */
const CANCELLABLE = new Set(["check", "guard", "export"]);

/** The core voltage a stock board runs at, mV: an unstamped image counts as this. */
const STOCK_MV = 1150;

/** The notice key of a plaintext session's refusal (hub-api.md 21.1). */
export const TLS_NOTICE = "dlink.tls_required";

/** The alarm rules the hub keeps on whatever is sent (`alarm.rules.set` cannot turn them off). */
export const ALWAYS_ON_ALARMS: ReadonlySet<string> = new Set(["rollout_failed"]);

/** Whether the rollout is shown: a hub with the `rollout` cap, to an Admin. */
export function rolloutShown(caps: HubCapsRecord | null, role: string): boolean {
  return !!caps?.onHub && !!caps.rollout && role === "admin";
}

/** Volts from mV, two decimals: "1.20 V". */
export function volts(mv: number): string {
  return `${(mv / 1000).toFixed(2)} V`;
}

/** A size in words. */
export function sizeText(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KiB`;
  return `${bytes} bytes`;
}

/** One image of the store as the list shows it. */
export interface ImageRow {
  name: string;
  version: string;
  platform: string;
  /** The core-voltage stamp: "1.20 V", or the unstamped words. */
  stamp: string;
  /** Stamped for the K340's 1.20 V. */
  k340: boolean;
  verified: boolean;
  /** "verified" or "not verified" (the core's SECFWV label). */
  label: string;
  size: string;
  added: string;
}

export function imageRows(images: readonly HubFirmwareImageRecord[]): ImageRow[] {
  return images.map((i) => ({
    name: i.name,
    version: i.firmwareVersion || "–",
    platform: i.platform || "–",
    stamp: i.stampMv === null ? `Unstamped (counts as ${volts(STOCK_MV)})` : volts(i.stampMv),
    k340: i.stampMv === 1200,
    verified: i.verified,
    label: i.verified ? "verified" : "not verified",
    size: sizeText(i.size),
    added: i.addedBy ? `${i.addedAt ? i.addedAt.slice(0, 10) : ""} by ${i.addedBy}`.trim() : i.addedAt.slice(0, 10),
  }));
}

/** Why a chosen file cannot be uploaded, before a byte is sent; "" when it can. */
export function fileProblem(name: string, size: number): string {
  if (size <= 0) return "The file is empty.";
  if (size > MAX_IMAGE_BYTES) return `The file is ${sizeText(size)}; the hub takes images up to ${sizeText(MAX_IMAGE_BYTES)}.`;
  if (!/^[A-Za-z0-9._+-]{1,64}$/.test(name) || !name.toLowerCase().endsWith(".uf2"))
    return "The name must be 1 to 64 letters, digits or . - _ + and end in .uf2.";
  if (!name.endsWith(".uf2")) return "The name must end in .uf2 (lower case).";
  return "";
}

/** The bytes as base64 (RFC 4648, padded), in pieces so a large image does not overflow the stack. */
export function base64Of(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(bin);
}

/** A rollout's target as the core takes it, from the page's choice `serial:<s>` or `group:<id>`. */
export function targetOf(choice: string): HubFleetTarget | null {
  if (choice.startsWith("serial:") && choice.length > 7) return { serials: [choice.slice(7)] };
  if (choice.startsWith("group:") && choice.length > 6) return { group: choice.slice(6) };
  return null;
}

/** A target's words: the device's name, or the group's. */
export function targetName(target: string, names: { device: (serial: string) => string; groups: readonly HubGroupRecord[] }): string {
  if (target.startsWith("group:")) {
    const id = target.slice(6);
    const g = names.groups.find((x) => x.id === id);
    return g ? `the group ${g.name}` : `group ${id}`;
  }
  return names.device(target);
}

/** One step's row of a device's progress. */
export interface StepRow {
  step: string;
  /** `done`, `running`, `failed`, `pending`, `skipped`. */
  state: "done" | "running" | "failed" | "pending" | "skipped";
  /** The hub's line for a step done or failed; "" otherwise. */
  text: string;
  at: string;
}

/** Every step of a device, in order: what is done, what runs, where it failed, what is left. */
export function stepRows(d: HubRolloutDeviceRecord): StepRow[] {
  const ended = d.state !== "running" && d.state !== "pending";
  return STEPS.map((step) => {
    const done = d.steps.filter((s) => s.step === step).at(-1);
    if (d.failed && d.failed.step === step) return { step, state: "failed", text: done && !done.ok ? done.text : d.failed.text, at: done?.at ?? "" };
    if (done) return { step, state: done.ok ? "done" : "failed", text: done.text, at: done.at };
    if (d.state === "running" && d.step === step) return { step, state: "running", text: "", at: "" };
    return { step, state: ended ? "skipped" : "pending", text: "", at: "" };
  });
}

/** The device a rollout is working on (or the last one it worked on). */
export function currentDevice(r: HubRolloutRecord): HubRolloutDeviceRecord | null {
  return r.devices.find((d) => d.state === "running") ?? [...r.devices].reverse().find((d) => d.state !== "pending" && d.state !== "not_tried") ?? r.devices[0] ?? null;
}

/** Whether a cancel would still take effect: the rollout runs and its device is before its bootloader entry. */
export function mayCancel(r: HubRolloutRecord): boolean {
  if (r.state !== "running") return false;
  const live = r.devices.find((d) => d.state === "running");
  // Between two members of a group: the next one has not reached its entry.
  if (!live) return r.devices.some((d) => d.state === "pending");
  return CANCELLABLE.has(live.step);
}

/** The result's lines for a device that came back (verify, as the hub read it). */
export function resultLines(d: HubRolloutDeviceRecord): string[] {
  const out: string[] = [];
  const a = d.after;
  if (!a) return out;
  const b = d.before;
  out.push(`Firmware ${a.fw || "unknown"}${b?.fw ? ` (was ${b.fw})` : ""}`);
  out.push(
    `Extension channel: ${a.extList.length} commands${b ? (a.extList.length >= b.extList.length ? `, none lost (${b.extList.length} before)` : `, ${b.extList.length - a.extList.length} fewer than before`) : ""}`,
  );
  if (b && b.requiredMv > STOCK_MV) {
    const v = a.vreg;
    const want = b.requiredMv;
    if (v && v.liveMv === want && v.storedMv === want) out.push(`Core voltage ${volts(want)} read back`);
    else out.push(`Core voltage not ${volts(want)}: live ${v?.liveMv != null ? volts(v.liveMv) : "unknown"}, stored ${v?.storedMv != null ? volts(v.storedMv) : "unknown"}`);
  }
  out.push(a.settings === "unchanged" ? "Settings unchanged" : a.settings === "changed" ? "Settings changed since the export" : "Settings could not be compared with the export");
  return out;
}

/** The export to offer to recall: the device's settings changed and the hub kept them first. */
export function recallOffer(d: HubRolloutDeviceRecord): { serial: string; id: string } | null {
  if (d.after?.settings !== "changed" || !d.before?.export) return null;
  return { serial: d.serial, id: d.before.export };
}

/** A rollout's words for its state. */
export function stateWord(state: string): string {
  switch (state) {
    case "running":
      return "Running";
    case "ok":
      return "Done";
    case "failed":
      return "Failed";
    case "cancelled":
      return "Cancelled";
    case "pending":
      return "Waiting";
    case "not_tried":
      return "Not tried";
    // -- WP PGWEB -- a group member without BOOTSEL (PLAN-PG 1.9): never tried
    case "excluded":
      return "Excluded";
    default:
      return state;
  }
}

// -- WP PGWEB --
/**
 * The update check's reasons the hub names (`failed.code`, PLAN-PG sections 1.8 and 1.9) in
 * words; "" for another code (its own text says it).
 */
export function updateReasonWords(code: string): string {
  switch (code) {
    case "no_bootsel":
      return "No BOOTSEL: this board (a DSPi-Zero, or one that could not be identified) has no BOOTSEL bootloader, so no rollout flashes it.";
    case "lineage_loss":
      return "Lineage: this DSPi runs this fork's firmware and the image may be upstream's; a rollout never gives that firmware away (no one types the confirmation there).";
    default:
      return "";
  }
}

/** `list` with `r` in it (replacing the record of its id), newest first. */
export function withRollout(list: readonly HubRolloutRecord[], r: HubRolloutRecord): HubRolloutRecord[] {
  const out = list.filter((x) => x.id !== r.id);
  out.push(r);
  return out.sort((a, b) => idNumber(b.id) - idNumber(a.id));
}

function idNumber(id: string): number {
  const n = Number(id.replace(/^\D+/, ""));
  return Number.isFinite(n) ? n : 0;
}

/**
 * What a `fw.rollout.changed` says (P3CC's delta): the record, and the device whose entry
 * changed with the step it is at; null for another event.
 */
export function rolloutChange(e: HubServerEventRecord): { rollout: HubRolloutRecord; serial: string; step: string } | null {
  if (e.kind !== "fw.rollout.changed" || !e.rollout) return null;
  const d = e.serial ? e.rollout.devices.find((x) => x.serial === e.serial) : currentDevice(e.rollout);
  return { rollout: e.rollout, serial: d?.serial ?? e.serial, step: d?.step ?? "" };
}

/** The `rollout_failed` alarms that are on, with the step the device's newest rollout stopped at. */
export function failedAlarms(alarms: readonly HubAlarmRecord[], rollouts: readonly HubRolloutRecord[]): { alarm: HubAlarmRecord; step: string }[] {
  return alarms
    .filter((a) => a.code === "rollout_failed" && alarmOn(a))
    .map((alarm) => {
      const d = rollouts.flatMap((r) => r.devices).find((x) => x.serial === alarm.serial && x.failed);
      const fromText = STEPS.find((s) => new RegExp(`\\b${s}\\b`).test(alarm.message)) ?? "";
      return { alarm, step: d?.failed?.step ?? fromText };
    });
}

/** Whether a failure is the hub's "encrypted connection needed". */
export function isTlsRefusal(e: unknown): boolean {
  return e instanceof CoreCallError && (e.noticeKey === TLS_NOTICE || /\btls_required\b/.test(e.message));
}

/** What one action said: a line for the user, and the notice it came with. */
export interface Said {
  text: string;
  bad: boolean;
  notice: NoticeRecord | null;
  /** The hub's code, where it gave one (`confirmation_required` asks for "Mute first"). */
  code?: string;
}

export interface RolloutView {
  /** Null before the first read. */
  images: HubFirmwareImageRecord[] | null;
  maxImages: number;
  rollouts: HubRolloutRecord[];
  groups: HubGroupRecord[];
  /** The plaintext refusal's notice: the rollout is closed to this connection. */
  tls: NoticeRecord | null;
  error: string;
  /** An upload under way: its name and size. */
  uploading: { name: string; size: number } | null;
}

/** How often the records are read again while one runs (events keep them current meanwhile), ms. */
export const ROLLOUT_POLL_MS = 3000;

export interface RolloutFeedOptions {
  periodMs?: number;
}

/** One session's view of its hub's image store and rollouts. */
export class RolloutFeed {
  view: RolloutView = { images: null, maxImages: 4, rollouts: [], groups: [], tls: null, error: "", uploading: null };
  private readonly listeners = new Set<() => void>();
  private unhub: (() => void) | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private running = false;
  private readonly periodMs: number;

  constructor(
    private readonly session: Session,
    opts: RolloutFeedOptions = {},
  ) {
    this.periodMs = opts.periodMs ?? ROLLOUT_POLL_MS;
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    this.unhub = this.session.watchHub((events) => this.onEvents(events));
    void this.read();
  }

  stop(): void {
    this.running = false;
    this.unhub?.();
    this.unhub = null;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Read the store, the rollouts and the groups. */
  async read(): Promise<void> {
    const core = this.session.core;
    const patch: Partial<RolloutView> = { error: "" };
    try {
      const store = await core.call("hubFirmwareImages", undefined);
      patch.images = store.images;
      patch.maxImages = store.maxImages || 4;
      patch.rollouts = await core.call("hubRollouts", {});
      patch.tls = null;
    } catch (e) {
      if (isTlsRefusal(e)) {
        patch.tls = noticeOf(e) ?? { key: TLS_NOTICE, kind: "error", title: "Encrypted connection needed", reason: "", action: "", detail: messageOf(e) };
        patch.images = [];
      } else patch.error = messageOf(e);
    }
    try {
      patch.groups = await core.call("hubGroups", undefined);
    } catch {
      patch.groups = [];
    }
    if (!this.running) return;
    this.set(patch);
    this.again();
  }

  /** Upload `bytes` as `name` (Admin); checked here first, so nothing goes for a file the hub would refuse by size. */
  async upload(name: string, bytes: Uint8Array): Promise<Said> {
    const problem = fileProblem(name, bytes.length);
    if (problem) return { text: problem, bad: true, notice: null };
    this.set({ uploading: { name, size: bytes.length } });
    try {
      const r = await this.session.core.call("hubFirmwarePut", { name, data: base64Of(bytes) });
      const images = [...(this.view.images ?? []).filter((i) => i.name !== r.image.name), r.image];
      this.set({ images, uploading: null });
      return { text: r.created ? `${name} is in the hub's store.` : `${name} was already in the hub's store.`, bad: false, notice: null };
    } catch (e) {
      this.set({ uploading: null });
      return failed(e);
    }
  }

  /** Mute the target (`vol.mute` on every device it names) through the hub, as the Admin. */
  async muteFirst(target: HubFleetTarget): Promise<Said> {
    try {
      const r = await this.session.core.call("hubGroupParamSet", { target, path: "vol.mute", value: { bool: true } });
      const bad = r.results.filter((x) => !x.ok);
      if (bad.length) return { text: `Not muted: ${bad.map((x) => `${x.serial} (${x.msg || x.code})`).join(", ")}`, bad: true, notice: null };
      return { text: "Muted.", bad: false, notice: null };
    } catch (e) {
      return failed(e);
    }
  }

  /** Whether every device of `target` is muted, as the hub reads them; null when it cannot say. */
  async muted(target: HubFleetTarget): Promise<boolean | null> {
    try {
      const r = await this.session.core.call("hubGroupParamGet", { target, path: "vol.mute" });
      if (!r.results.length || r.results.some((x) => !x.ok)) return null;
      return r.results.every((x) => typeof x.value === "object" && x.value !== null && "bool" in x.value && x.value.bool);
    } catch {
      return null;
    }
  }

  /** Start a rollout (Admin); the record goes into the list. */
  async startRollout(target: HubFleetTarget, image: string, whileLive: boolean): Promise<Said & { rollout?: HubRolloutRecord }> {
    try {
      const ask = () => this.session.core.call("hubRolloutStart", { target, image, whileLive });
      // After a cancel the hub's record says cancelled at once, but its worker keeps the one
      // rollout slot until it leaves its current step (ms on a device, longer on the bench): a
      // start in that moment is asked once more. A normal end frees the slot before it is announced.
      const justEnded = !this.view.rollouts.some((x) => x.state === "running");
      const r = await ask().catch(async (e: unknown) => {
        if (!(justEnded && e instanceof CoreCallError && /one at a time/.test(e.message))) throw e;
        await new Promise((ok) => setTimeout(ok, 1000));
        return ask();
      });
      this.set({ rollouts: withRollout(this.view.rollouts, r) });
      this.again();
      return { text: `Rollout ${r.id} started.`, bad: false, notice: null, rollout: r };
    } catch (e) {
      return failed(e);
    }
  }

  /** Cancel rollout `id`: before its device's bootloader entry, else the hub says it is too late. */
  async cancel(id: string): Promise<Said> {
    try {
      const r = await this.session.core.call("hubRolloutCancel", { rollout: id });
      this.set({ rollouts: withRollout(this.view.rollouts, r) });
      return { text: r.state === "cancelled" ? `Rollout ${id} cancelled before the bootloader entry.` : `Rollout ${id}: ${stateWord(r.state)}.`, bad: false, notice: null };
    } catch (e) {
      const s = failed(e);
      if (e instanceof CoreCallError && /\bconflict\b|bootloader entry|has ended/.test(e.message))
        return { ...s, text: "Too late to cancel: the device is past its bootloader entry, and a flash is never interrupted. The rollout goes on." };
      return s;
    }
  }

  /** Recall the export the hub made before the flash (`snapshot.recall`, judged as any recall). */
  async recall(serial: string, id: string): Promise<Said> {
    try {
      const r = await this.session.core.call("hubSnapshotsRecall", { serial, id });
      return { text: `Recalled the export ${r.name || id}.`, bad: false, notice: null };
    } catch (e) {
      return failed(e);
    }
  }

  // -- events --

  private onEvents(events: HubServerEventRecord[]): void {
    let list = this.view.rollouts;
    let dirty = false;
    for (const e of events) {
      const c = rolloutChange(e);
      if (!c) continue;
      list = withRollout(list, c.rollout);
      dirty = true;
    }
    if (dirty) {
      this.set({ rollouts: list });
      this.again();
    }
  }

  /** While a rollout runs, read the records again now and then: an event missed is caught. */
  private again(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    if (!this.running || !this.view.rollouts.some((r) => r.state === "running")) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.refreshRollouts();
    }, this.periodMs);
  }

  private async refreshRollouts(): Promise<void> {
    try {
      const rollouts = await this.session.core.call("hubRollouts", {});
      if (this.running) this.set({ rollouts });
    } catch {
      // The next event or read catches up.
    }
    this.again();
  }

  private set(patch: Partial<RolloutView>): void {
    this.view = { ...this.view, ...patch };
    for (const l of [...this.listeners]) l();
  }
}

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function failed(e: unknown): Said {
  const notice = noticeOf(e);
  const code = e instanceof CoreCallError ? e.code : undefined;
  const msg = messageOf(e);
  if (isTlsRefusal(e)) return { text: notice?.title ?? "Encrypted connection needed", bad: true, notice: notice ?? { key: TLS_NOTICE, kind: "error", title: "Encrypted connection needed", reason: "", action: "", detail: msg }, code };
  return { text: notice ? `${notice.title}: ${msg}` : msg, bad: true, notice, code };
}
