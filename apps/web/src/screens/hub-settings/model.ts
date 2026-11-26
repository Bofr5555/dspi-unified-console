// WP P2WB2: the hub settings page's model. The page is thin: every rule (ranges, the private-address
// rule of an unencrypted webhook, who may change what) is the hub's or the core's, and the page shows
// their answers. What is here: which sections the hub's caps give, whether this session may edit
// (the hub's role for it, Admin only), the records the forms edit (built as the core takes them,
// the read-only fields left out) and the words for the lists.
//
// Secrets (a minted API key, a webhook's new secret) are `HubSecret` objects from the moment the
// Worker's answer arrives: read once with `reveal()` to show it, never logged (their `toString`,
// `toJSON` and Node's inspect say only `set`) and never kept anywhere but the one card that shows
// them until Done.

import type { ScreenDef } from "../../ui/screens";
import type {
  HubAlarmRuleRecord,
  HubApiKeyMintedWire,
  HubCapsRecord,
  HubMqttSettingsRecord,
  HubOutboundSetRecord,
  HubOutboundTargetRecord,
  HubScheduleActionRecord,
  HubScheduleResultRecord,
  HubScheduleRuleRecord,
} from "../../core/hub-settings-protocol";

// ---------------------------------------------------------------- secrets

/** A secret the hub made and shows once (the core's `HubSecret`, on the web). */
export class HubSecret {
  readonly #text: string;
  constructor(text: string) {
    this.#text = typeof text === "string" ? text : "";
  }
  /** The secret's text, to show once. */
  reveal(): string {
    return this.#text;
  }
  toString(): string {
    return this.#text ? "set" : "not set";
  }
  toJSON(): string {
    return this.toString();
  }
  [Symbol.for("nodejs.util.inspect.custom")](): string {
    return `HubSecret(${this.toString()})`;
  }
}

/** A key the hub just minted: `key` is shown once. */
export interface MintedKey {
  cid: number;
  name: string;
  role: string;
  plaintextOk: boolean;
  key: HubSecret;
}

/** The Worker's answer to a mint, with the key wrapped at once. */
export function mintedFrom(w: HubApiKeyMintedWire): MintedKey {
  return { cid: w.cid, name: w.name, role: w.role, plaintextOk: !!w.plaintextOk, key: new HubSecret(w.key) };
}

/** A webhook secret the hub just made, shown once. */
export interface NewSecret {
  target: string;
  /** The target's name, for the card. */
  name: string;
  secret: HubSecret;
}

/** The new secrets of an `outbound.set` answer, wrapped at once; its targets as they are now. */
export function outboundSetFrom(r: HubOutboundSetRecord): { targets: HubOutboundTargetRecord[]; secrets: NewSecret[] } {
  const targets = Array.isArray(r.targets) ? r.targets : [];
  const secrets = (Array.isArray(r.secrets) ? r.secrets : []).map((s) => ({
    target: s.target,
    name: targets.find((t) => t.id === s.target)?.name || s.target,
    secret: new HubSecret(s.secret),
  }));
  return { targets, secrets };
}

// ---------------------------------------------------------------- the page

/** The page's id among the session's screens (its own route: the hub page links to it by this id). */
export const HUB_SETTINGS_SCREEN = "hub-settings";

/** The sections the hub's caps give, in the page's order. */
export type Section = "alarms" | "schedule" | "outbound" | "mqtt" | "apiKeys";

export function sectionsOf(caps: HubCapsRecord | null | undefined): Section[] {
  if (!caps?.onHub) return [];
  const all: Section[] = ["alarms", "schedule", "outbound", "mqtt", "apiKeys"];
  return all.filter((s) => caps[s] === true);
}

/** The page is offered only when the hub has one of its families (an upstream hub has none). */
export function hubSettingsShown(caps: HubCapsRecord | null | undefined): boolean {
  return sectionsOf(caps).length > 0;
}

/**
 * The session's screens with the hub settings page: in Advanced and Installer (the Pro layer),
 * before Settings (which stays last), when the hub has its families.
 */
export function withHubSettings(list: ScreenDef[], mode: string, shown: boolean, def: ScreenDef): ScreenDef[] {
  if (!shown || mode === "simple" || list.some((x) => x.id === def.id)) return list;
  const at = list.findIndex((x) => x.id === "settings");
  return at < 0 ? [...list, def] : [...list.slice(0, at), def, ...list.slice(at)];
}

/** Only an Admin of the hub changes its settings, and not in the client's read-only mode. */
export function mayEdit(role: string, readOnlyReason: string): boolean {
  return role === "admin" && !readOnlyReason;
}

/** The core's row saying why the page is read-only below Admin (the sibling of `installer.hub_role`). */
export const HUB_SETTINGS_ROLE_KEY = "installer.hub_settings_role";

export function roleWord(role: string): string {
  return role === "admin" ? "Admin" : role === "control" ? "Control" : role === "viewer" ? "Viewer" : role || "unknown";
}

// ---------------------------------------------------------------- alarm rules

export type RuleField = "afterS" | "perMinute" | "aboveC" | "abovePct" | "forS";

/** A rule's number fields (the ones the code has: not null), with the hub's ranges as hints. */
export const RULE_FIELDS: { field: RuleField; label: string; min: number; max: number; step: number }[] = [
  { field: "afterS", label: "After (s)", min: 1, max: 86400, step: 1 },
  { field: "perMinute", label: "Events a minute", min: 1, max: 1000, step: 1 },
  { field: "aboveC", label: "At or above (°C)", min: 0, max: 150, step: 0.5 },
  { field: "abovePct", label: "At or above (%)", min: 1, max: 100, step: 1 },
  { field: "forS", label: "For (s)", min: 1, max: 3600, step: 1 },
];

export function ruleFields(r: HubAlarmRuleRecord): typeof RULE_FIELDS {
  return RULE_FIELDS.filter((f) => typeof r[f.field] === "number");
}

const ALARM_WORDS: Record<string, string> = {
  device_offline: "DSPi not attached",
  unresponsive: "DSPi not answering",
  device_restarted: "Restarted",
  clip: "Clipping",
  temperature: "Running hot",
  cpu: "Processing load high",
  schedule_failed: "A schedule did not complete",
  clock_unset: "The hub's clock is not set",
};

/** A rule's name: the code's notice title where the page knows it, else the code. */
export function alarmLabel(code: string, title?: string): string {
  return (title && !title.includes("{") ? title : "") || ALARM_WORDS[code] || code;
}

/** `rules` with `code`'s rule changed. */
export function withRule(rules: HubAlarmRuleRecord[], code: string, patch: Partial<HubAlarmRuleRecord>): HubAlarmRuleRecord[] {
  return rules.map((r) => (r.code === code ? { ...r, ...patch } : r));
}

/** A number typed into a field; null when it is not one (the hub then keeps its value). */
export function numberOf(text: string): number | null {
  const t = text.trim();
  if (!t) return null;
  const v = Number(t);
  return Number.isFinite(v) ? v : null;
}

// ---------------------------------------------------------------- schedules

export const DAYS: { id: string; label: string }[] = [
  { id: "mon", label: "Mon" },
  { id: "tue", label: "Tue" },
  { id: "wed", label: "Wed" },
  { id: "thu", label: "Thu" },
  { id: "fri", label: "Fri" },
  { id: "sat", label: "Sat" },
  { id: "sun", label: "Sun" },
];

export const ACTION_TYPES: { t: string; label: string }[] = [
  { t: "param.set", label: "Set a value" },
  { t: "preset.load", label: "Load a preset" },
  { t: "snapshot.recall", label: "Recall a snapshot" },
  { t: "system.scene.apply", label: "Apply a scene" },
];

/** `param.set`'s paths a schedule may set: a level in dB, or a mute (on or off). */
export const PARAM_PATHS: { path: string; label: string; flag: boolean; idx: boolean }[] = [
  { path: "vol.master", label: "Master volume (dB)", flag: false, idx: false },
  { path: "vol.user", label: "User volume (dB)", flag: false, idx: false },
  { path: "vol.mute", label: "Mute", flag: true, idx: false },
  { path: "out.mute", label: "Output mute", flag: true, idx: true },
];

export function pathOf(path: string) {
  return PARAM_PATHS.find((p) => p.path === path) ?? PARAM_PATHS[0]!;
}

const BLANK_ACTION: HubScheduleActionRecord = { t: "", slot: null, name: "", snapshot: "", scene: "", path: "", idx: [], value: null, flag: null };

/** A new action of type `t`, with a value to start from. */
export function newAction(t: string): HubScheduleActionRecord {
  switch (t) {
    case "preset.load":
      return { ...BLANK_ACTION, t, slot: 0 };
    case "snapshot.recall":
    case "system.scene.apply":
      return { ...BLANK_ACTION, t };
    default:
      return { ...BLANK_ACTION, t: "param.set", path: "vol.master", value: -20 };
  }
}

/** The action with another `param.set` path: a mute takes on or off, a level a number. */
export function withPath(a: HubScheduleActionRecord, path: string): HubScheduleActionRecord {
  const p = pathOf(path);
  return {
    ...a,
    path: p.path,
    flag: p.flag ? (a.flag ?? true) : null,
    value: p.flag ? null : (a.value ?? -20),
    idx: p.idx ? (a.idx.length ? a.idx : [0]) : [],
  };
}

/** A new weekly rule for `serial` (the open DSPi), weekdays at 18:00. */
export function newRule(serial: string): HubScheduleRuleRecord {
  return {
    id: "",
    name: "",
    enabled: true,
    days: ["mon", "tue", "wed", "thu", "fri"],
    time: "18:00",
    once: "",
    actions: [newAction("param.set")],
    targets: serial ? [serial] : [],
  };
}

/** The rule as `schedule.set` takes it: the hub's read-only fields left out. */
export function ruleForSet(r: HubScheduleRuleRecord): HubScheduleRuleRecord {
  const weekly = !r.once;
  return {
    id: r.id,
    name: r.name.trim(),
    enabled: r.enabled,
    days: weekly ? r.days : [],
    time: weekly ? r.time : "",
    once: r.once,
    actions: r.actions,
    targets: r.targets,
  };
}

/** Serials typed into one field, separated by commas or spaces. */
export function targetsOf(text: string): string[] {
  return text
    .split(/[\s,;]+/)
    .map((s) => s.trim())
    .filter(Boolean);
}

/** When a rule runs, in words. */
export function whenText(r: HubScheduleRuleRecord): string {
  if (r.once) return `Once, ${r.once.replace("T", " ")}`;
  const days = DAYS.filter((d) => r.days.includes(d.id)).map((d) => d.label);
  const which = days.length === 7 ? "Every day" : days.length ? days.join(", ") : "No day";
  return `${which} at ${r.time || "–"}`;
}

/** One action in words. */
export function actionText(a: HubScheduleActionRecord): string {
  switch (a.t) {
    case "preset.load":
      return a.slot != null ? `Load preset slot ${a.slot + 1}` : `Load preset “${a.name}”`;
    case "snapshot.recall":
      return `Recall snapshot “${a.snapshot || a.name}”`;
    case "system.scene.apply":
      return `Apply scene “${a.scene}”`;
    case "param.set": {
      const p = pathOf(a.path);
      const where = p.idx && a.idx.length ? ` ${a.idx[0]! + 1}` : "";
      if (p.flag) return `${p.label}${where} ${a.flag ? "on" : "off"}`;
      return `${p.label.replace(/ \(dB\)$/, "")}${where} to ${a.value ?? "–"} dB`;
    }
    default:
      return a.t;
  }
}

/** A run's row in words: the device, the action, and the hub's code and reason when it failed. */
export function resultText(r: HubScheduleResultRecord): string {
  const who = r.serial || "All units";
  return r.ok ? `${who}: ${r.action}: done` : `${who}: ${r.action}: ${[r.code, r.msg].filter(Boolean).join(": ") || "failed"}`;
}

/** An ISO time from the hub in this browser's words ("" stays ""). */
export function localTime(iso: string | null | undefined): string {
  if (!iso) return "";
  const t = Date.parse(iso);
  return Number.isNaN(t) ? iso : new Date(t).toLocaleString();
}

// ---------------------------------------------------------------- notifications out

export const EVENT_KINDS: { id: string; label: string }[] = [
  { id: "alarm.raised", label: "Alarm raised" },
  { id: "alarm.cleared", label: "Alarm cleared" },
  { id: "restart", label: "Restart" },
  { id: "schedule.ran", label: "Schedule ran" },
  { id: "schedule.failed", label: "Schedule failed" },
];

/** A new webhook: every kind, a new secret, https. */
export function newTarget(): HubOutboundTargetRecord {
  return {
    id: "",
    name: "",
    kind: "webhook",
    url: "https://",
    enabled: true,
    kinds: EVENT_KINDS.map((k) => k.id),
    secret: "new",
    plaintext: false,
    caPem: "",
    pinSha256: "",
  };
}

/** The list as `outbound.set` takes it: the settable fields; a secret `set` is kept by the hub. */
export function targetsForSet(list: HubOutboundTargetRecord[]): HubOutboundTargetRecord[] {
  return list.map((t) => ({
    id: t.id,
    name: t.name,
    kind: t.kind,
    url: t.url.trim(),
    enabled: t.enabled,
    kinds: t.kinds,
    secret: t.secret === "set" ? "" : t.secret,
    plaintext: t.plaintext,
    caPem: t.caPem,
    pinSha256: t.pinSha256.trim(),
  }));
}

/** How a target is sent, as the hub reports it. */
export function transportText(t: HubOutboundTargetRecord): string {
  if (!t.id) return "";
  if (t.encrypted) return `Encrypted (${t.trust === "pin" ? "key pin" : t.trust === "ca" ? "own CA" : "public CAs"})`;
  return "Unencrypted";
}

/** A target's last delivery in words. */
export function lastText(t: HubOutboundTargetRecord): string {
  const l = t.last;
  if (!l) return t.id ? "Not tried yet" : "";
  const at = localTime(l.at);
  if (l.ok) return `Delivered ${at}${l.status ? ` (${l.status})` : ""}`;
  return `Not delivered ${at}${l.status ? ` (${l.status})` : ""}${l.msg ? `: ${l.msg}` : ""}`;
}

// ---------------------------------------------------------------- MQTT

/** What the password field sends: the typed password, `not set` to clear it, else "" (the hub keeps it). */
export function passwordToSend(typed: string, clear: boolean): string {
  if (clear) return "not set";
  return typed;
}

/**
 * The settings as `mqtt.set` takes them. `password` and the client key: typed text sets them,
 * `clear` empties them, otherwise the hub keeps what it has (`set`). `tls` is replaced whole by
 * the hub, so every field of it is sent back.
 */
export function mqttForSet(
  s: HubMqttSettingsRecord,
  password: { typed: string; clear: boolean },
  clientKey: { typed: string; clear: boolean },
): HubMqttSettingsRecord {
  const keyWas = s.tls.clientKeyPem === "set" ? "set" : "";
  return {
    enabled: s.enabled,
    broker: s.broker.trim(),
    port: s.port,
    clientId: s.clientId.trim(),
    username: s.username,
    password: passwordToSend(password.typed, password.clear),
    baseTopic: s.baseTopic.trim(),
    tls: {
      ...s.tls,
      mode: s.tls.mode === "on" ? "on" : "off",
      clientKeyPem: clientKey.clear ? "" : clientKey.typed || keyWas,
    },
    haEnabled: s.haEnabled,
    haKey: s.haKey,
  };
}

export function mqttStateText(state: string): string {
  return state === "connected" ? "Connected" : state === "connecting" ? "Connecting" : state === "failed" ? "Failed" : "Off";
}

// ---------------------------------------------------------------- API keys

export const KEY_ROLES = ["viewer", "control", "admin"] as const;

// -- WP UB6 -- (UXP B6, W10) One unsaved bar for the page ("3 changes · Discard · Save") in place of a Save per
// section: what each section has changed against the hub's copy, counted as the user sees it.

/** Whether `a` and `b` agree on `keys` (a list or an object compared as a whole). */
function sameOn<T extends object>(a: T, b: T, keys: readonly (keyof T)[]): boolean {
  return keys.every((k) => (typeof a[k] === "object" && a[k] !== null ? JSON.stringify(a[k]) === JSON.stringify(b[k]) : a[k] === b[k]));
}

/** The keys a rule has on either side. */
function ruleKeys(a: HubAlarmRuleRecord, b: HubAlarmRuleRecord): (keyof HubAlarmRuleRecord)[] {
  return [...new Set([...Object.keys(a), ...Object.keys(b)])] as (keyof HubAlarmRuleRecord)[];
}

/** The alarm rules changed against the hub's: one per rule. */
export function ruleChanges(saved: readonly HubAlarmRuleRecord[] | null, now: readonly HubAlarmRuleRecord[] | null): number {
  if (!saved || !now) return 0;
  const by = new Map(saved.map((r) => [r.code, r]));
  return now.filter((r) => {
    const was = by.get(r.code);
    return !was || !sameOn(was, r, ruleKeys(was, r));
  }).length;
}

/** What the user edits of a notification target (the rest is the hub's report). */
const TARGET_KEYS = ["name", "kind", "url", "enabled", "kinds", "secret", "plaintext", "caPem", "pinSha256"] as const;

/** The notification targets changed against the hub's: one per target added, removed or edited. */
export function targetChanges(saved: readonly HubOutboundTargetRecord[] | null, now: readonly HubOutboundTargetRecord[] | null): number {
  if (!saved || !now) return 0;
  const by = new Map(saved.map((t) => [t.id, t]));
  const kept = new Set(now.map((t) => t.id).filter((id) => id));
  const edited = now.filter((t) => {
    const was = t.id ? by.get(t.id) : undefined;
    return !was || !sameOn(was, t, TARGET_KEYS);
  }).length;
  return edited + saved.filter((t) => !kept.has(t.id)).length;
}

const MQTT_KEYS = ["enabled", "broker", "port", "clientId", "username", "baseTopic", "haEnabled", "haKey"] as const;
const MQTT_TLS_KEYS = ["mode", "caPem", "pinSha256", "clientCertPem", "clientCertFile", "clientKeyFile", "allowPlaintextPassword"] as const;

/** The MQTT settings changed against the hub's: one per field, the password and the client key each one when typed or cleared. */
export function mqttChanges(
  saved: HubMqttSettingsRecord | null,
  now: HubMqttSettingsRecord | null,
  password: { typed: string; clear: boolean },
  clientKey: { typed: string; clear: boolean },
): number {
  if (!saved || !now) return 0;
  const fields = MQTT_KEYS.filter((k) => !sameOn(saved, now, [k])).length;
  const tls = MQTT_TLS_KEYS.filter((k) => !sameOn(saved.tls, now.tls, [k])).length;
  return fields + tls + (password.typed || password.clear ? 1 : 0) + (clientKey.typed || clientKey.clear ? 1 : 0);
}

/** "1 change", "3 changes". */
export function changesText(n: number): string {
  return `${n} ${n === 1 ? "change" : "changes"}`;
}
