// WP FEATUI (AIVIS's web part): the hub's `presence` family as the page reads it
// (crates/dspi-wasm/src/hub_server.rs `hubPresenceList`, dspi-core hub_server/presence.rs).

/** Who works on the hub's devices beside the people: an AI agent, through another console. */
export interface HubPresenceRecord {
  /** `agent`. */
  kind: string;
  /** The agent's name, as its console says it (text from elsewhere, cleaned by the hub). */
  label: string;
  /** The console it works through, in the hub's words. */
  by: string;
  /** The device it works on; "" for none named. */
  serial: string;
  /** What it does now, in words. */
  what: string;
  /** Seconds left unless its console says it again. */
  ttlS: number;
}

export interface PresenceMethods {
  /** The list (`presence.list`), every role. `unsupported` without the `presence` cap. */
  hubPresenceList: { params: undefined; result: HubPresenceRecord[] };
}
