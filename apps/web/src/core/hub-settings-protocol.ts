// WP P2WB2: the hub's settings families and its snapshot store, as the Worker answers them
// (crates/dspi-wasm/src/hub_server.rs over dspi_core::hub_server, and
// crates/dspi-wasm/src/hub_server_admin.rs over dspi_core::hub_server::admin). Records are the
// core's, camelCase; an `Option` the core has none of is `null`. Every call answers
// `unsupported` without the family's cap and sends nothing; the hub's refusals carry the core's
// `noticeKey`. A secret (a minted key, a webhook's new secret) comes as a plain string in the
// answer of the call that made it, once: hub-settings/model.ts wraps it in a `HubSecret` at once.

// The caps record and the `hubCaps` call are declared once, in protocol.ts (WP P2WB1); re-exported here for this page's importers.
export type { HubCapsRecord } from "./protocol";

/** One code's alarm rule; a field the code does not have is `null` (kept by the hub on a set). */
export interface HubAlarmRuleRecord {
  code: string;
  on: boolean;
  afterS: number | null;
  perMinute: number | null;
  muteOnStorm: boolean | null;
  aboveC: number | null;
  abovePct: number | null;
  forS: number | null;
}

/** One action of a schedule rule (`dspi_core::HubScheduleAction`). */
export interface HubScheduleActionRecord {
  t: string;
  slot: number | null;
  name: string;
  snapshot: string;
  scene: string;
  path: string;
  idx: number[];
  value: number | null;
  flag: boolean | null;
}

export interface HubScheduleRuleRecord {
  id: string;
  name: string;
  enabled: boolean;
  days: string[];
  time: string;
  once: string;
  actions: HubScheduleActionRecord[];
  targets: string[];
  /** Read only. */
  nextRun?: string;
  /** Read only. */
  last?: { at: string; ok: boolean; skipped: boolean } | null;
}

export interface HubSchedulesRecord {
  rules: HubScheduleRuleRecord[];
  zone: string;
  localTime: string;
  clockUnset: boolean;
}

export interface HubScheduleResultRecord {
  serial: string;
  action: string;
  ok: boolean;
  code: string;
  msg: string;
}

export interface HubScheduleRunRecord {
  rule: string;
  name: string;
  at: string;
  by: string;
  results: HubScheduleResultRecord[];
}

/** A notification target (`dspi_core::HubOutboundTarget`); the last block is read only. */
export interface HubOutboundTargetRecord {
  id: string;
  name: string;
  kind: string;
  url: string;
  enabled: boolean;
  kinds: string[];
  /** Read: `set` or `not set`. To set: `new`, `set`/"" (keep) or `not set`. Never typed. */
  secret: string;
  plaintext: boolean;
  caPem: string;
  pinSha256: string;
  queued?: number;
  dropped?: number;
  last?: { at: string; ok: boolean; status: number | null; msg: string } | null;
  encrypted?: boolean;
  trust?: string;
  notice?: string;
}

/** `hubOutboundSet`'s answer: `secrets` in this answer only. */
export interface HubOutboundSetRecord {
  targets: HubOutboundTargetRecord[];
  secrets: { target: string; secret: string }[];
}

export interface HubOutboundTestRecord {
  delivered: boolean;
  status: number | null;
  msg: string;
}

export interface HubMqttTlsRecord {
  mode: string;
  caPem: string;
  pinSha256: string;
  clientCertPem: string;
  /** Read: `set` or `not set`; `set` keeps it, a PEM sets it, "" clears it. */
  clientKeyPem: string;
  clientCertFile: string;
  clientKeyFile: string;
  allowPlaintextPassword: boolean;
}

export interface HubMqttSettingsRecord {
  enabled: boolean;
  broker: string;
  port: number | null;
  clientId: string;
  username: string;
  /** Read: `set` or `not set`; a typed password sets it; `set`/"" keeps it; `not set` clears it. */
  password: string;
  baseTopic: string;
  tls: HubMqttTlsRecord;
  haEnabled: boolean;
  haKey: number | null;
  notice?: string;
}

export interface HubMqttStatusRecord {
  state: string;
  msg: string;
  since: string;
  queued: number;
  dropped: number;
  encrypted: boolean;
  notice: string;
}

export interface HubApiKeyRecord {
  cid: number;
  name: string;
  role: string;
  plaintextOk: boolean;
  online: boolean;
  created: string;
  lastSeen: string;
}

/** `hubApiKeyMint`'s answer as the Worker sends it: `key` in this answer only. */
export interface HubApiKeyMintedWire {
  cid: number;
  name: string;
  role: string;
  plaintextOk: boolean;
  key: string;
}

/** A snapshot's entry (`dspi_core::SnapshotInfo`). */
export interface SnapshotInfoRecord {
  id: string;
  name: string;
  note: string;
  serial: string;
  savedMs: number;
  platform: string;
  inputChannels: number;
  outputChannels: number;
  wireFormat: number;
}

/** One row of the merged list: `where` is `local`, `hub` or `both` (the core's word). */
export interface MergedSnapshotRecord {
  snapshot: SnapshotInfoRecord;
  where: string;
  localCopy: boolean;
}

export interface SnapshotMergedListRecord {
  entries: MergedSnapshotRecord[];
  hubListed: boolean;
  hubNoticeKey: string;
}

export interface HubSnapshotRecallRecord {
  serial: string;
  snapshot: string;
  name: string;
  image: boolean;
  lines: string[];
  notes: string[];
  guard: string;
}

export interface HubSettingsMethods {
  // `hubCaps` is declared in HubServerMethods (protocol.ts, WP P2WB1)
  hubAlarmRulesGet: { params: undefined; result: HubAlarmRuleRecord[] };
  hubAlarmRulesSet: { params: { rules: HubAlarmRuleRecord[] }; result: HubAlarmRuleRecord[] };
  hubSchedules: { params: undefined; result: HubSchedulesRecord };
  hubScheduleSet: { params: { rule: HubScheduleRuleRecord }; result: HubScheduleRuleRecord };
  hubScheduleDelete: { params: { id: string }; result: null };
  hubScheduleRunNow: { params: { id: string }; result: HubScheduleRunRecord };
  hubOutboundGet: { params: undefined; result: HubOutboundTargetRecord[] };
  hubOutboundSet: { params: { targets: HubOutboundTargetRecord[] }; result: HubOutboundSetRecord };
  hubOutboundTest: { params: { id: string }; result: HubOutboundTestRecord };
  hubMqttGet: { params: undefined; result: HubMqttSettingsRecord };
  hubMqttSet: { params: { settings: HubMqttSettingsRecord }; result: HubMqttSettingsRecord };
  hubMqttStatus: { params: undefined; result: HubMqttStatusRecord };
  hubApiKeys: { params: undefined; result: HubApiKeyRecord[] };
  hubApiKeyMint: { params: { name: string; role: string; plaintextOk?: boolean }; result: HubApiKeyMintedWire };
  hubApiKeyRevoke: { params: { cid: number }; result: null };
  // The hub's snapshot store
  snapshotListMerged: { params: { serial?: string }; result: SnapshotMergedListRecord };
  hubSnapshotsGet: { params: { id: string }; result: { snapshot: SnapshotInfoRecord; document: string } };
  hubSnapshotsPut: { params: { serial?: string; text: string }; result: { snapshot: SnapshotInfoRecord; created: boolean } };
  hubSnapshotsRename: { params: { id: string; name: string }; result: SnapshotInfoRecord };
  hubSnapshotsDelete: { params: { id: string }; result: null };
  hubSnapshotsRecall: {
    params: { serial?: string; id: string; guard?: boolean; timeoutMs?: number };
    result: HubSnapshotRecallRecord;
  };
}
