// Hub tokens (WP WB4b): what a session presents to a hub instead of its PIN.
//
// Standalone, a token is kept per hub in this page's storage, beside the hub
// list: a page served by a hub is that hub's own origin, so its tokens stay with
// it. A fleet server that holds credentials for its hubs supplies them through
// `FleetBackend.hubCredentials`; the console then keeps a token it paired for
// only in memory, for the visit: it never stores a server's credential, nor one
// of its own beside the server's.
//
// A token goes out together with the hub's id it belongs to, and the core sends
// it only once the hub has said it is that hub (Link spec 6.2). Only a hub that
// refuses a token makes the console forget it.

import type { FleetBackend } from "./types";
import type { KeyValueStore } from "./local";

const KEY = "dspi-web.hub-tokens.v1";

export interface HubToken {
  token: string;
  /** The hub the token belongs to; "" when not known (a fleet's credential without one). */
  hubId: string;
}

export interface HubTokens {
  get(url: string): Promise<HubToken | null>;
  /** A pairing gave this token. */
  save(url: string, token: HubToken): void;
  /** The hub refused it. */
  forget(url: string): void;
}

/** Tokens in the page's storage (standalone); for the visit only if storage is refused. */
export class LocalHubTokens implements HubTokens {
  private memory: Record<string, HubToken>;

  constructor(private readonly storage: KeyValueStore | null) {
    this.memory = this.load();
  }

  async get(url: string): Promise<HubToken | null> {
    return this.memory[url] ?? null;
  }

  save(url: string, token: HubToken): void {
    if (!token.token) return;
    this.memory[url] = token;
    this.store();
  }

  forget(url: string): void {
    delete this.memory[url];
    this.store();
  }

  private load(): Record<string, HubToken> {
    try {
      const v: unknown = JSON.parse(this.storage?.getItem(KEY) ?? "{}");
      if (typeof v !== "object" || v === null) return {};
      const out: Record<string, HubToken> = {};
      for (const [url, t] of Object.entries(v as Record<string, unknown>)) {
        const r = t as { token?: unknown; hubId?: unknown } | null;
        if (r && typeof r.token === "string" && typeof r.hubId === "string") out[url] = { token: r.token, hubId: r.hubId };
      }
      return out;
    } catch {
      return {};
    }
  }

  private store(): void {
    try {
      this.storage?.setItem(KEY, JSON.stringify(this.memory));
    } catch {
      // Storage refused: the tokens last for this visit.
    }
  }
}

/** A fleet server's credentials first, then what this visit paired for (kept in memory only). */
export class FleetHubTokens implements HubTokens {
  private readonly paired = new Map<string, HubToken>();
  private readonly refused = new Set<string>();

  constructor(private readonly backend: FleetBackend) {}

  async get(url: string): Promise<HubToken | null> {
    const mine = this.paired.get(url);
    if (mine) return mine;
    if (this.refused.has(url)) return null;
    const c = await this.backend.hubCredentials?.(url);
    return c?.token ? { token: c.token, hubId: c.hubId ?? "" } : null;
  }

  save(url: string, token: HubToken): void {
    if (token.token) this.paired.set(url, token);
  }

  forget(url: string): void {
    // A refused server credential is not offered again this visit; the user pairs instead.
    if (!this.paired.delete(url)) this.refused.add(url);
  }
}

/** Where this console's hub tokens come from. */
export function hubTokensFor(backend: FleetBackend, storage: KeyValueStore | null): HubTokens {
  return backend.hubCredentials ? new FleetHubTokens(backend) : new LocalHubTokens(storage);
}
