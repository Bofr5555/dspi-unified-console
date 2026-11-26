// One open device session as the UI sees it (WP WB5): the core client, what the
// fleet backend allows, and the last snapshot and Link phase it reported.
//
// A hub (WP WB4b) is reached in Link's own steps, each the core's: connect with
// the token kept for that hub; if it wants a PIN, `pairing` until the user gives
// it (the token is then kept); then either the DSPi the fleet named, or
// `choosing` from the hub's own list; then open. From then on the banner follows
// the core's phases (reconnecting, closed), which the Worker pushes.

import type { HeldUnit } from "../screens/measure/system/desk"; // -- WP WEBM3 --
import {
  CoreCallError,
  type CoreEvent,
  type CoreMethods,
  type DeviceSnapshot,
  type DeviceTarget,
  // -- WP WB8 --
  type HistoryMove,
  type HubAdminEventRecord, // -- WP HUBADMIN --
  type HubServerEventRecord, // -- WP P2WB1 --
  type LinkDeviceRecord,
  type LinkPhase,
  type ParamValue,
  // -- WP P1WB1 --
  type OpenedDocument,
  // -- WP P1WB2 --
  type LockStateRecord,
  type NoticeRecord,
  type DeviceProtocolRecord, // -- WP PGWEB --
} from "../core/protocol";
import type { CoreClient } from "../core/client";
import { VIEW_ONLY_REASON, type Access, type Source } from "../fleet/types";
// -- WP WB4b --
import type { HubToken, HubTokens } from "../fleet/hub-tokens";
// -- WP P1WB2 -- notices from the core's table; the limits profile this page keeps
import { loadNotices, noticeOf } from "./texts";
import { safeLocalStorage } from "../fleet/local";
import { loadFieldRanges, loadProfile } from "../screens/installer/model";
import { restoreSpeakerLayout } from "../screens/settings/speakers/use-layout"; // -- WP ROLEW --
// -- WP SECWEB -- the link's encryption (the browser's TLS or the `uconsole web` proxy's)
import { UNKNOWN_SECURITY, linkContext, routeLink, securityOf, type LinkContext, type LinkSecurity } from "../fleet/security";

export interface SessionState {
  /** -- WP WB4b -- `pairing`: a hub wants its PIN; `choosing`: pick one of a hub's DSPis. */
  status: "opening" | "pairing" | "choosing" | "open" | "failed" | "closed";
  snapshot: DeviceSnapshot | null;
  link: { phase: LinkPhase; pinSource?: string } | null;
  error: string;
  // -- WP SECWEB --
  /** A hub session's link encryption once read (null: no hub, or not read yet). */
  security?: LinkSecurity | null;
  // -- WP WB4b --
  /** The hub's DSPis, while `choosing`. */
  devices: LinkDeviceRecord[];
  /** The hub's name once it has said it. */
  hubName: string;
  /** Why the last PIN was not taken. */
  pinError: string;
  // -- WP WB6b -- (c6p WB6a-4)
  /**
   * What is unsaved against the active preset, in the core's sentences (CG7,
   * `presetChanges`), kept current after every change the session sees. Empty
   * when nothing is, and always while read only.
   */
  unsaved: string[];
  // -- WP WB8 --
  /** What undo and redo can do now (the core's journal); both false while read only. */
  history: { canUndo: boolean; canRedo: boolean };
  /** What the last undo or redo has to say: another writer's change left alone (`warn`), changes stepped over (`info`). */
  notice: { text: string; tone: "warn" | "info" } | null;
  // -- WP P1WB1 --
  /** What an offline session was opened from (`documentOpen`): the strip says "File: <name>". Null on a device. */
  document?: OpenedDocument | null;
  // -- WP P1WB2 --
  /** The error's notice from the core's table (P1NT), when it has one; `error` is then its detail. */
  errorNotice?: NoticeRecord | null;
  /** The device's installer lock (`installerLockRefresh`): the hub's or this console's. Null before it is read. */
  lock?: LockStateRecord | null;
  /** Counts the lock's and the pinned mode's reads, so the mode is resolved again after each. */
  lockRev?: number;
  // -- WP HUBADMIN --
  /** Counts the hub's admin events (`limits.changed`, `limits.held`, `mode.changed`): the Installer page reads the hub's copy again on each. */
  hubAdminRev?: number;
  /** The last of those events (its rows: what the hub held). */
  hubAdminEvent?: HubAdminEventRecord | null;
  /** The recovery code of a PIN just set on the hub, shown once until dismissed; "" for none. */
  hubRecovery?: string;
  // -- WP PGWEB --
  /**
   * The connection's protocol (`deviceProtocol`: generation, pages, extension envelope, board,
   * kind), read once per connection before the screens show (and again after a reconnect).
   * Null before it is read, or when the core could not say it.
   */
  protocol?: DeviceProtocolRecord | null;
  // -- WP UWN -- (UXP A15)
  /** Where the device is, as its view learnt it: the hub's location for it, its unit's name on the audio link ("" for none). */
  place?: { location: string; unit: string };
  // -- WP UWN -- (UXP A16)
  /** While the link reconnects: another hub (a new identity) answers at its address, so the core will never bind again. */
  otherHub?: boolean;
}

// -- WP WB6a --
/** The calls `Session.change` makes. */
export type ChangeMethod = "presetCopy" | "presetRevert" | "factoryReset"
  // -- WP WB6c --
  | "crossoverType"
  | "crosspoint"
  // -- WP WB8 -- a whole band during a graph drag
  | "filter"
  // -- WP P1WB2 -- every limiter on at -1 dBFS, pairs linked (one undo step)
  | "protectAllOutputs"
  // -- WP PAIRBAND -- a linked input pair's band, both sides (one undo step)
  | "inputPairFilter"
  | "inputPairBandBypass"
  // -- WP WB6G -- a linked input pair's trim, both sides (one undo step)
  | "inputPairPreamp"
  // -- WP WBCLIP -- Paste Parameters (one undo step, the linked partner too)
  | "pasteChannel";

// -- WP WB6b --
/** One write of `Session.writeAll`. */
export interface Write {
  path: string;
  value: ParamValue;
  indices?: number[];
}

export class Session {
  state: SessionState = {
    status: "opening",
    snapshot: null,
    link: null,
    error: "",
    devices: [],
    hubName: "",
    pinError: "",
    unsaved: [],
    history: { canUndo: false, canRedo: false },
    notice: null,
  };
  private readonly listeners = new Set<() => void>();
  private unsubscribe: () => void;
  // -- WP WB4b -- the hub this session is on, and the id its token belongs to
  private hub: { url: string; hubId: string } | null = null;
  // -- WP SECWEB -- the hub's address as listed (its tokens' key), and this page's link context
  private hubKey = "";
  private linkCtx: LinkContext | null = null;
  /** The serial of the hub DSPi this session opens or has open, if any. */
  deviceId = "";
  // -- WP P1WB2 --
  /** The mode a fleet backend pins for this device (`FleetBackend.pinnedMode`), "" for none. */
  fleetMode = "";
  // -- WP P2WB1 --
  /** Open another DSPi of this session's hub in the console (set by the page for a hub session); null where it cannot. */
  openDevice: ((serial: string) => void) | null = null;
  // -- WP WEBM1 --
  /** Why the page may not open the DSPi `serial` in this session (it is open in another one), "" when it may (set by the page). */
  refuseOpen: ((serial: string) => string) | null = null;
  // -- WP WEBM3 --
  /** The console's other session that has the DSPi `serial` (set by the page), null when none has: views that span units use its core. */
  heldBy: ((serial: string) => HeldUnit | null) | null = null;
  // -- WP UWN -- (UXP A16)
  /** Close this session and open its source again in a new one (Try Again, Connect Again); set by the page, null where it cannot. */
  reopen: (() => void) | null = null;
  /** Close this session and take its hub off the page's list (Remove); set by the page where the list may change. */
  forget: (() => void) | null = null;

  /** Whether the session's DSPi is open but its link is away (reconnecting, the DSPi gone from the hub): nothing is written then. */
  get away(): boolean {
    return this.state.status === "open" && !!this.state.link && this.state.link.phase !== "bound";
  }

  constructor(
    readonly id: string,
    readonly installationId: string,
    readonly installationName: string,
    readonly source: Source,
    readonly access: Access,
    readonly core: CoreClient,
    // -- WP WB4b --
    private readonly tokens: HubTokens | null = null,
  ) {
    this.unsubscribe = core.subscribe((e) => this.onEvent(e));
  }

  // -- WP SECWEB --
  /** This page's link context (its location, the proxy), once a hub session read it. */
  get linkContext(): LinkContext | null {
    return this.linkCtx;
  }

  /** Why writes are off, or "" when this session may write. */
  get readOnlyReason(): string {
    if (this.access === "view") return VIEW_ONLY_REASON;
    return this.state.snapshot?.device.readOnly ? this.state.snapshot.device.readOnlyReason : "";
  }

  // -- WP P1WB1 -- offline editing (P1OF): a document opened with no device
  /** Open the `.dspipreset` document `text` as this session's device; nothing touches USB. */
  async openDocument(text: string, fileName: string): Promise<void> {
    await this.applyAccess(); // -- WP ROMODE --
    try {
      const r = await this.core.call("documentOpen", { text, fileName });
      const { document, ...snapshot } = r;
      // Tokens are required by the core from the start (P1TZ): nothing to turn on before the page can write
      this.set({ status: "open", snapshot, document, error: "" });
      void this.afterOpen(); // -- WP P1WB2 --
    } catch (e) {
      this.set({ status: "failed", error: message(e) });
    }
  }

  /** The document this session edits, or null on a device. */
  get document(): OpenedDocument | null {
    return this.state.document ?? null;
  }

  /** Read the device again and show it (after a recall, an import, a revert). */
  async refresh(): Promise<void> {
    try {
      const snapshot = await this.core.call("state", undefined);
      this.set({ snapshot });
    } catch (e) {
      this.set({ error: message(e) });
    }
  }

  // -- WP UWN -- (UXP A15)
  /** What the device's view learnt of where it is (the hub's inventory, the audio link's units): its tab says it. */
  setPlace(place: { location: string; unit: string }): void {
    const was = this.state.place;
    if (was && was.location === place.location && was.unit === place.unit) return;
    if (!was && !place.location && !place.unit) return;
    this.set({ place });
  }

  /** Show `text` as the session's error line, or clear it with "". */
  report(text: string): void {
    this.set({ error: text });
  }

  // -- WP ROMODE --
  /**
   * A view-only session (`?readonly`, or a fleet's view access) puts its core in the client
   * read-only mode before anything opens, so a write that got past the page is refused by the
   * core too, with nothing sent. A core without the call (the tests' stub) is left as it is.
   */
  private async applyAccess(): Promise<void> {
    if (this.access !== "view") return;
    await this.core.call("setClientReadOnly", { on: true }).catch(() => undefined);
  }

  async open(target: DeviceTarget): Promise<void> {
    await this.applyAccess(); // -- WP ROMODE --
    if (target.kind === "hub") return this.openHub(target.url, target.deviceId);
    try {
      await this.load(target);
    } catch (e) {
      // -- WP PGWEB -- a refused open in its notice's words (the probe's: newer protocol, unrecognised device)
      await loadNotices(this.core).catch(() => undefined);
      this.set({ status: "failed", error: message(e), errorNotice: noticeOf(e) });
    }
  }

  private async load(target: DeviceTarget): Promise<void> {
    const snapshot = await this.core.call("open", target);
    // -- WP PGWEB -- the protocol once per connection, before the screens choose what to show
    const protocol = await this.readProtocol();
    // Tokens are required by the core from the start (P1TZ): nothing to turn on before the page can write
    this.set({ status: "open", snapshot, error: "", protocol });
    // -- WP P1WB2 -- every kind: the notices, the limits this page keeps, the lock
    await this.afterOpen();
  }

  // -- WP P1WB2 --
  /**
   * Once a device (or a document) is open (tokens are required by the core from the start, P1TZ):
   * the notice table is read, the limits profile this page
   * keeps for the device is applied (P1LM: the web has no files), and the lock is read.
   * Each step's failure leaves the session as it was (the tests' stub core has none of them).
   */
  private async afterOpen(): Promise<void> {
    try {
      await loadNotices(this.core).catch(() => undefined);
      await loadFieldRanges(this.core).catch(() => undefined); // -- WP P1EFIX --
      const serial = this.state.snapshot?.device.serial || this.deviceId;
      // -- WP HUBADMIN -- this page's copy is for a direct (USB) DSPi; through a hub, the hub's copy is in force
      const viaHub = await this.core.call("installerTarget", undefined).then((t) => t?.copy === "hub", () => false);
      const text = serial && !this.readOnlyReason && !viaHub ? loadProfile(safeLocalStorage(), serial) : "";
      if (text) await this.core.call("limitsApply", { text }).catch(() => undefined);
      // -- WP ROLEW -- the speaker layout this page keeps for the device (the web has no files)
      if (serial) await restoreSpeakerLayout(this).catch(() => undefined);
      await this.refreshLock();
    } catch {
      // Nothing of it is needed to show the device.
    }
  }

  // -- WP PGWEB --
  /** `deviceProtocol`, null when the core cannot say it (the tests' stub, a document). */
  private async readProtocol(): Promise<DeviceProtocolRecord | null> {
    return this.core.call("deviceProtocol", undefined).then(
      (p) => (p && typeof p === "object" && typeof p.generation === "string" ? p : null),
      () => null,
    );
  }

  // -- WP MERGEP1D --
  /**
   * The hub's installer lock for the bound DSPi (`installerLockRefresh`), read
   * after the bind and after a reconnect, so the core refuses a write in the
   * hub's words (`installer_lock.hub`). -- WP P1WB2 -- kept in the state for the one lock
   * prompt (`heldBy`); a fleet's pinned mode goes to the core after it (the refresh sets
   * the hub's).
   */
  async refreshLock(): Promise<void> {
    let lock: unknown = null;
    try {
      lock = await this.core.call("installerLockRefresh", undefined);
    } catch {
      // The last answer stands (a hub that could not answer now).
    }
    // After the refresh, which sets the hub's own pin (or none).
    if (this.fleetMode) await this.core.call("modeSetPinned", { mode: this.fleetMode }).catch(() => undefined);
    this.set({ lock: isLock(lock) ? lock : (this.state.lock ?? null), lockRev: (this.state.lockRev ?? 0) + 1 });
  }

  // -- WP P1WB2 --
  /** Unlock with the installer's PIN (the hub's, when the hub holds the lock). Answers the refusal's words, "" when unlocked. */
  async unlock(pin: string): Promise<string> {
    try {
      const lock = await this.core.call("installerUnlock", { pin });
      if (isLock(lock)) this.set({ lock });
      return "";
    } catch (e) {
      void this.refreshLockState();
      const n = noticeOf(e);
      return n ? `${n.title}: ${n.reason}` : message(e);
    }
  }

  /** Lock again (the end of an unlock). */
  async relock(): Promise<void> {
    try {
      const lock = await this.core.call("installerLock", undefined);
      if (isLock(lock)) this.set({ lock });
    } catch (e) {
      this.fail(e);
    }
  }

  /** The lock as the core holds it now, no hub round trip (after a refusal: the wait). */
  private async refreshLockState(): Promise<void> {
    try {
      const lock = await this.core.call("installerLockState", undefined);
      if (isLock(lock)) this.set({ lock });
    } catch {
      // Kept.
    }
  }

  /**
   * A confirmation token for `op` (`save_preset`, `delete_preset`, `copy_preset`,
   * `factory_reset`, `config_import`), minted when the user has confirmed and spent by the
   * call right after (P1TK). Undefined when the core has none (the tests' stub).
   */
  async token(op: string): Promise<string | undefined> {
    return this.core.call("confirmToken", { op }).catch(() => undefined);
  }

  /** `write` of a gated path with a token for `op` minted now. */
  async writeConfirmed(op: string, path: string, value: ParamValue, indices?: number[]): Promise<boolean> {
    return this.write(path, value, indices, await this.token(op));
  }

  /**
   * Several writes as one undo step named `label` (P1UG: Clear All, Direct 1:1): the group
   * is ended whatever `run` does. Answers what `run` answered.
   */
  async group<T>(label: string, run: () => Promise<T>): Promise<T> {
    await this.core.call("beginGroup", { label }).catch(() => undefined);
    try {
      return await run();
    } finally {
      await this.core.call("endGroup", undefined).catch(() => undefined);
      this.historyAsked = null;
      void this.refreshHistory();
    }
  }

  /** Show `e` as the session's error, with its notice row when the core's table has one. */
  fail(e: unknown): void {
    this.set({ error: message(e), errorNotice: noticeOf(e) });
  }

  // -- WP WB4b --
  private async openHub(url: string, deviceId: string): Promise<void> {
    this.deviceId = deviceId;
    // -- WP SECWEB -- the address the core opens: `wss://` on an https page, the proxy on `uconsole web`'s
    const key = url;
    this.hubKey = key;
    const ctx = await linkContext().catch((): LinkContext => ({ loc: null, proxy: false }));
    this.linkCtx = ctx;
    url = routeLink(url, ctx);
    try {
      const saved = (await this.tokens?.get(key)) ?? null;
      const connect = (t: HubToken | null) => this.core.call("linkConnect", { url, token: t?.token ?? "", hubId: t?.hubId ?? "" });
      let hello: Awaited<ReturnType<typeof connect>>;
      try {
        hello = await connect(saved);
      } catch (e) {
        // -- WP WB7 -- (c6m N2) another hub answers where the token's hub was (it was
        // replaced, or the address moved): that token is for no hub here. Forget it and
        // connect without one; this hub then asks for its PIN.
        if (!(saved && e instanceof CoreCallError && e.code === "wrong_hub")) throw e;
        this.tokens?.forget(key);
        hello = await connect(null);
      }
      if (hello.tokenRefused) this.tokens?.forget(key);
      this.hub = { url, hubId: hello.hubId };
      void this.readSecurity(); // -- WP SECWEB --
      this.set({ hubName: hello.hubName });
      if (hello.needsPairing) {
        this.set({ status: "pairing", link: { phase: "needs_pairing", pinSource: hello.pinSource } });
        return;
      }
      await this.afterAuth();
    } catch (e) {
      // -- WP SECWEB -- the proxy's refusal (a changed key, TLS refused) in its own words
      const sec = ctx.proxy ? await securityOf(url, ctx).catch(() => null) : null;
      this.set({ status: "failed", error: sec?.message || message(e), security: sec });
    }
  }

  // -- WP SECWEB --
  /** Read the hub link's encryption again (after a connect and each reconnect). */
  private async readSecurity(): Promise<void> {
    if (!this.hub || !this.linkCtx) return;
    const sec = await securityOf(this.hub.url, this.linkCtx).catch(() => UNKNOWN_SECURITY);
    this.set({ security: sec });
  }

  /** The PIN the user typed; a wrong one keeps the prompt, with why. */
  async pair(pin: string): Promise<void> {
    if (!this.hub || this.state.status !== "pairing") return;
    try {
      const { token } = await this.core.call("linkPair", { pin });
      this.tokens?.save(this.hubKey || this.hub.url, { token, hubId: this.hub.hubId }); // -- WP SECWEB -- by the listed address
      this.set({ pinError: "" });
      await this.afterAuth();
    } catch (e) {
      if (e instanceof CoreCallError && e.code !== "wrong_pin" && e.code !== "link") this.set({ status: "failed", error: message(e) });
      else this.set({ pinError: message(e) });
    }
  }

  /** Open one of the hub's DSPis (`choosing`). */
  async choose(serial: string): Promise<void> {
    if (!this.hub || this.state.status !== "choosing") return;
    // -- WP WEBM1 -- one session per DSPi
    const refused = this.refuseOpen?.(serial) ?? "";
    if (refused) return this.report(refused);
    this.deviceId = serial;
    try {
      await this.load({ kind: "hub", url: this.hub.url, deviceId: serial });
    } catch (e) {
      this.deviceId = "";
      this.set({ error: message(e) });
    }
  }

  private async afterAuth(): Promise<void> {
    if (!this.hub) return;
    if (this.deviceId) {
      await this.load({ kind: "hub", url: this.hub.url, deviceId: this.deviceId });
      return;
    }
    this.set({ status: "choosing", devices: await this.core.call("linkDevices", undefined), error: "" });
  }

  private refreshDevices(): void {
    this.core.call("linkDevices", undefined).then(
      (devices) => {
        if (this.state.status === "choosing") this.set({ devices });
      },
      () => undefined,
    );
  }

  /**
   * Writes and shows what the device read back; a refusal leaves the old value
   * shown. -- WP WB6a -- Answers whether the write went through.
   */
  async write(path: string, value: ParamValue, indices?: number[], token?: string): Promise<boolean> {
    if (this.away) return false; // -- WP UWN -- (UXP A16) fenced while the link is away: the strip says why, no failure notice
    if (this.readOnlyReason) {
      this.set({ error: this.readOnlyReason });
      return false;
    }
    try {
      // -- WP P1WB2 -- a gated path's confirmation token, when the caller minted one
      const t = token ? { token } : {};
      await this.core.call("write", indices ? { path, indices, value, ...t } : { path, value, ...t });
      const held = path === "preset.load" ? await this.loadNotice() : null;
      const snapshot = await this.core.call("state", undefined);
      this.set({ snapshot, error: "", ...(held ? { notice: held } : {}) });
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }

  // -- WP INTI -- (owner 2026-11-23) the person's preset load runs silenced and holds a tweeter
  // left without its high-pass (the core's guard): what it held, or a volume it could not bring back
  private async loadNotice(): Promise<SessionState["notice"]> {
    try {
      const text = await this.core.call("presetLoadNotice", undefined);
      return text ? { text, tone: "warn" } : null;
    } catch {
      return null;
    }
  }

  // -- WP WB6a --
  /**
   * A core call that changes the device without a path (the preset flows:
   * `presetCopy`, `presetRevert`, `factoryReset`), then the read-back, as
   * `write`. Refused while read only. Answers whether it went through.
   */
  async change<M extends ChangeMethod>(method: M, params: CoreMethods[M]["params"]): Promise<boolean> {
    if (this.away) return false; // -- WP UWN -- (UXP A16) fenced while the link is away: the strip says why, no failure notice
    if (this.readOnlyReason) {
      this.set({ error: this.readOnlyReason });
      return false;
    }
    try {
      await this.core.call(method, params);
      const held = method === "presetRevert" ? await this.loadNotice() : null;
      const snapshot = await this.core.call("state", undefined);
      this.set({ snapshot, error: "", ...(held ? { notice: held } : {}) });
      return true;
    } catch (e) {
      this.fail(e); // -- WP P1WB2 --
      return false;
    }
  }

  /**
   * Asked before the user closes this session (Disconnect): the session's
   * view sets it (the unsaved-changes guard, M42; -- WP WB6b -- for every
   * screen, not one). Answers whether to go on.
   */
  leaveGuard: (() => Promise<boolean>) | null = null;

  async mayLeave(): Promise<boolean> {
    return this.leaveGuard ? this.leaveGuard() : true;
  }

  // -- WP WB6b --
  /**
   * Several writes in a row (the fields of one filter band), then one
   * read-back of the state, as `write`. Stops at the first refusal, which
   * the error shows; answers whether all went through.
   */
  async writeAll(writes: Write[]): Promise<boolean> {
    if (this.away) return false; // -- WP UWN -- (UXP A16) fenced while the link is away: the strip says why, no failure notice
    if (this.readOnlyReason) {
      this.set({ error: this.readOnlyReason });
      return false;
    }
    let ok = true;
    try {
      for (const w of writes) await this.core.call("write", w.indices ? { path: w.path, indices: w.indices, value: w.value } : w);
    } catch (e) {
      ok = false;
      this.fail(e); // -- WP P1WB2 --
    }
    try {
      const snapshot = await this.core.call("state", undefined);
      this.set(ok ? { snapshot, error: "" } : { snapshot });
    } catch (e) {
      if (ok) this.set({ error: message(e) });
      ok = false;
    }
    return ok;
  }

  /**
   * Save the live state to `slot`. (c6p WB6a-3) A slot without a name is
   * named "Preset N" first, as the Console and the Qt app do.
   */
  async savePreset(slot: number): Promise<boolean> {
    const name = this.state.snapshot?.state.presetNames[slot] ?? "";
    if (!name.trim() && !(await this.write("preset.name", { text: `Preset ${slot + 1}` }, [slot]))) return false;
    return this.writeConfirmed("save_preset", "preset.save", "trigger", [slot]); // -- WP P1WB2 -- with its token
  }

  /** Whether anything is unsaved (`unsaved`); the page's leave prompt and a host page ask this. */
  get dirty(): boolean {
    return this.state.unsaved.length > 0;
  }

  private unsavedAsked: DeviceSnapshot | null = null;
  private unsavedRunning = false;

  /**
   * After a new snapshot: what is unsaved now (`presetChanges`, the core's
   * comparison, no device I/O). One call at a time; a snapshot that comes
   * meanwhile is asked about once that one is answered.
   */
  private async refreshUnsaved(): Promise<void> {
    if (this.unsavedRunning) return;
    this.unsavedRunning = true;
    try {
      while (this.unsavedAsked !== this.state.snapshot) {
        const snap = this.state.snapshot;
        this.unsavedAsked = snap;
        if (!snap || this.readOnlyReason || this.state.status !== "open") {
          if (this.state.unsaved.length) this.set({ unsaved: [] });
          continue;
        }
        try {
          const r = await this.core.call("presetChanges", undefined);
          if (!sameLines(r.changes, this.state.unsaved)) this.set({ unsaved: r.changes });
        } catch {
          // Kept: the last answer stands.
        }
      }
    } finally {
      this.unsavedRunning = false;
    }
  }

  // -- WP WB8 -- undo and redo through the core's journal (P0W)

  /** Step back (`forward` false) or forward through the core's journal, then the read-back. */
  async step(forward: boolean): Promise<void> {
    if (this.readOnlyReason || this.state.status !== "open" || this.away) return; // -- WP UWN -- away
    try {
      const m = await this.core.call(forward ? "redo" : "undo", undefined);
      const snapshot = await this.core.call("state", undefined);
      this.set({ snapshot, error: "", notice: await this.stepNotice(m) });
    } catch (e) {
      this.fail(e); // -- WP P1WB2 --
      void this.refreshHistory();
    }
  }

  undo(): Promise<void> {
    return this.step(false);
  }

  redo(): Promise<void> {
    return this.step(true);
  }

  /** The words for what a step left alone: the core's note naming the other writer, else what it stepped over. */
  private async stepNotice(m: HistoryMove): Promise<SessionState["notice"]> {
    if (m.note) return { text: capitalise(m.note), tone: "warn" };
    if (!m.skipped.length) return null;
    const names = await Promise.all(m.skipped.map((e) => this.changeLabel(e.path)));
    return {
      text: names.length === 1 ? `Stepped over a change that cannot be undone: ${names[0]}` : `Stepped over changes that cannot be undone: ${names.join("; ")}`,
      tone: "info",
    };
  }

  /** A journal path in the registry's words (its label), as the core's notes name it. */
  private async changeLabel(path: string): Promise<string> {
    if (path === "eq.band") return "Filter band";
    try {
      return (await this.core.call("describe", { path })).label || path;
    } catch {
      return path;
    }
  }

  dismissNotice(): void {
    if (this.state.notice) this.set({ notice: null });
  }

  private drags = 0;
  private dragSent = false;

  /**
   * A continuous drag (a slider, a graph handle) starts or ends: the core joins
   * its writes into one undo step. Counted, so two pointers at once end the
   * mark only when both have let go. Nothing is sent while read only.
   */
  setDragging(on: boolean): void {
    if (on) {
      if (this.drags++ === 0) {
        this.watchDragEnd(true);
        if (!this.readOnlyReason) {
          this.dragSent = true;
          void this.core.call("setDragging", { dragging: true }).catch(() => undefined);
        }
      }
    } else if (this.drags > 0 && --this.drags === 0) this.endDragMark();
  }

  // -- fixup wb8 -- a safety net: the page hidden or the window left mid-drag ends the
  // mark, whatever the controls hear (a browser may deliver no pointer event then), so a
  // typed value afterwards is never joined into the drag's step. The controls' own later
  // end is then a no-op.
  private readonly dragEndNet = () => {
    if (typeof document !== "undefined" && document.visibilityState === "visible" && document.hasFocus?.()) return;
    if (this.drags > 0) {
      this.drags = 0;
      this.endDragMark();
    }
  };

  private watchDragEnd(on: boolean): void {
    if (typeof window === "undefined" || typeof document === "undefined") return;
    if (on) {
      window.addEventListener("blur", this.dragEndNet);
      document.addEventListener("visibilitychange", this.dragEndNet);
    } else {
      window.removeEventListener("blur", this.dragEndNet);
      document.removeEventListener("visibilitychange", this.dragEndNet);
    }
  }

  private endDragMark(): void {
    this.watchDragEnd(false);
    if (!this.dragSent) return;
    this.dragSent = false;
    void this.core.call("setDragging", { dragging: false }).catch(() => undefined);
  }

  private historyAsked: DeviceSnapshot | null = null;
  private historyRunning = false;

  /** After a new snapshot: what undo and redo can do now (no device I/O). One call at a time, as `refreshUnsaved`. */
  private async refreshHistory(): Promise<void> {
    if (this.historyRunning) return;
    this.historyRunning = true;
    try {
      do {
        const snap = this.state.snapshot;
        this.historyAsked = snap;
        const none = { canUndo: false, canRedo: false };
        let next = none;
        if (snap && !this.readOnlyReason && this.state.status === "open") {
          try {
            next = (await this.core.call("canUndo", undefined)) ?? this.state.history;
          } catch {
            next = this.state.history;
          }
        }
        const h = this.state.history;
        if (next.canUndo !== h.canUndo || next.canRedo !== h.canRedo) this.set({ history: { canUndo: next.canUndo, canRedo: next.canRedo } });
      } while (this.historyAsked !== this.state.snapshot);
    } finally {
      this.historyRunning = false;
    }
  }

  async close(): Promise<void> {
    clearTimeout(this.otherHubTimer); // -- WP UWN --
    this.otherHubTimer = undefined;
    this.unsubscribe();
    await this.core.dispose();
    this.set({ status: "closed" });
  }

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  // -- WP ODEC --
  private readonly elsewhereListeners = new Set<(before: DeviceSnapshot | null, after: DeviceSnapshot) => void>();

  /** Follow the device's changes that come from elsewhere (its notifications: another console, an agent, the device); returns the way to stop. */
  watchElsewhere(fn: (before: DeviceSnapshot | null, after: DeviceSnapshot) => void): () => void {
    this.elsewhereListeners.add(fn);
    return () => this.elsewhereListeners.delete(fn);
  }

  private onEvent(e: CoreEvent): void {
    if (e.event === "state") {
      const before = this.state.snapshot;
      this.set({ snapshot: e.snapshot });
      for (const l of [...this.elsewhereListeners]) l(before, e.snapshot); // -- WP ODEC --
    }
    else if (e.event === "link") {
      // -- WP MERGEP1D -- bound again after a reconnect: the hub's lock again
      if (e.phase === "bound" && this.state.link?.phase === "reconnecting") void this.refreshLock();
      if (e.phase === "bound" && this.state.link?.phase === "reconnecting") void this.readSecurity(); // -- WP SECWEB --
      // -- WP PGWEB -- a reconnect is a new connection: the generation may have changed (a re-flash behind the hub)
      if (e.phase === "bound" && this.state.link?.phase === "reconnecting") void this.readProtocol().then((protocol) => this.set({ protocol }));
      // -- WP UWN -- (UXP A16) bound again: what failed while the link was away is not true any more
      const back = e.phase === "bound" && !!this.state.link && this.state.link.phase !== "bound";
      this.set({ link: { phase: e.phase, pinSource: e.pinSource }, ...(back ? { error: "", otherHub: false } : {}) });
      this.watchOtherHub();
      // -- WP WB4b -- the hub's list may have changed (a DSPi came or went)
      if (this.state.status === "choosing") this.refreshDevices();
    }
    else if (e.event === "lost") this.set({ status: "failed", error: e.reason });
    // -- WP HUBADMIN -- the hub changed its copy: the lock as the core read it again, and a new read of the page
    else if (e.event === "hubAdmin") this.onHubAdmin(e.events);
    // -- WP P2WB1 -- the hub's own families' events, to whoever follows them
    else if (e.event === "hubServer") for (const l of [...this.hubListeners]) l(e.events);
  }

  // -- WP UWN -- (UXP A16)
  private otherHubTimer: ReturnType<typeof setTimeout> | undefined;

  /**
   * While the link reconnects, look at why each redial failed (`linkStatus`'s reason, the core's words): when another
   * hub answers at the address (a hub restarted with a new identity), the core goes on trying for ever, so the strip
   * says so with Connect Again. Every 3 s while reconnecting; nothing otherwise.
   */
  private watchOtherHub(): void {
    const reconnecting = this.state.link?.phase === "reconnecting" && this.state.status === "open";
    if (!reconnecting) {
      clearTimeout(this.otherHubTimer);
      this.otherHubTimer = undefined;
      return;
    }
    if (this.otherHubTimer !== undefined) return;
    this.otherHubTimer = setTimeout(() => {
      this.otherHubTimer = undefined;
      void this.core.call("linkStatus", undefined).then(
        (st) => {
          if (this.state.link?.phase !== "reconnecting") return;
          const other = isOtherHub(st?.closeReason ?? "");
          if (other !== !!this.state.otherHub) this.set({ otherHub: other });
          this.watchOtherHub();
        },
        () => this.watchOtherHub(),
      );
    }, 3000);
  }

  // -- WP P2WB1 --
  private readonly hubListeners = new Set<(events: HubServerEventRecord[]) => void>();

  /** Follow the hub's `event.added`, `inventory.changed`, `alarm.changed`, ... of each DSPi Link turn; returns the way to stop. */
  watchHub(fn: (events: HubServerEventRecord[]) => void): () => void {
    this.hubListeners.add(fn);
    return () => this.hubListeners.delete(fn);
  }

  // -- WP P2WB3 --
  /** The hub took a new name (`hubRename`, Admin): the name this session shows for it. */
  hubRenamed(name: string): void {
    if (name && name !== this.state.hubName) this.set({ hubName: name });
  }

  // -- WP HUBADMIN --
  /** Show (or, with "", dismiss) the recovery code of a PIN just set on the hub. Kept only in this session's state. */
  showHubRecovery(code: string): void {
    this.set({ hubRecovery: code });
  }

  /** The hub's admin events of one Link turn: the lock of the last one for this device; the page reads the hub's copy again. */
  private onHubAdmin(events: HubAdminEventRecord[]): void {
    const serial = this.state.snapshot?.device.serial || this.deviceId;
    const mine = events.filter((x) => !x.serial || !serial || x.serial === serial);
    if (!mine.length) return;
    const last = mine[mine.length - 1]!;
    this.set({
      ...(isLock(last.lock) ? { lock: last.lock, lockRev: (this.state.lockRev ?? 0) + 1 } : {}),
      hubAdminEvent: last,
      hubAdminRev: (this.state.hubAdminRev ?? 0) + 1,
    });
  }

  private set(patch: Partial<SessionState>): void {
    // -- WP P1WB2 -- a new error line without a notice has none
    if ("error" in patch && !("errorNotice" in patch)) patch = { ...patch, errorNotice: null };
    this.state = { ...this.state, ...patch };
    for (const l of this.listeners) l();
    // -- WP WB6b --
    if (this.state.snapshot !== this.unsavedAsked) void this.refreshUnsaved();
    // -- WP WB8 --
    if (this.state.snapshot !== this.historyAsked) void this.refreshHistory();
  }
}

function message(e: unknown): string {
  if (e instanceof CoreCallError) return capitalise(e.message);
  return e instanceof Error ? e.message : String(e);
}

// -- WP P1WB2 --
/** A lock state as the core answers it (a test's fake core may answer anything). */
function isLock(v: unknown): v is LockStateRecord {
  return !!v && typeof v === "object" && typeof (v as LockStateRecord).pinSet === "boolean";
}

// -- WP UWN -- (UXP A16)
/** Whether a redial's failure (`linkStatus.closeReason`) is the core's "another hub answers at this address". */
export function isOtherHub(reason: string): boolean {
  return /another hub answers/i.test(reason);
}

function sameLines(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((x, i) => x === b[i]);
}

function capitalise(s: string): string {
  return s ? s[0]!.toUpperCase() + s.slice(1) : s;
}
