// WP P2WB1: the backend of a console served by a hub (PRO phase 2). The serving
// hub is an installation of its own whose DSPis come from the hub's inventory:
// every DSPi it has seen, online or not (an offline one is listed, not
// offered). What a user may do there is what the hub's role for this page's
// own DSPi Link session says (a Viewer views); the mode a role is pinned to
// is the hub's `mode.pinned`, which each session's core applies itself.
//
// Everything else is the standalone backend's: this console's demo and USB,
// the hubs the user added, and the serving hub's own entry, which still asks
// the hub (Link's choose step, its PIN when it wants one).
//
// The hub is asked through a core of the backend's own (a Worker with its own
// DSPi Link session, the typed calls `hubCaps`, `hubInventory`, `hubRole`):
// the console never speaks the hub's JSON itself. A hub that
// wants a PIN first, or lacks the inventory (an upstream hub), gives no
// installation of its own: the page is the standalone one.

import type { CoreClient } from "../core/client";
import type { HubDeviceRecord, LinkHelloRecord } from "../core/protocol";
import { LocalHubTokens, type HubTokens } from "./hub-tokens";
import { LocalBackend, safeLocalStorage, type KeyValueStore } from "./local";
import type { Access, FleetBackend, Installation, PinnedMode, Source } from "./types";

/** The serving hub's installation. */
export const HUB_INSTALLATION_ID = "hub";

export interface HubBackendOptions {
  /** The serving hub's DSPi Link address (`ws://host:port/...`). */
  servingHub: string;
  /** A core of the backend's own (its own Worker). */
  spawnCore: () => CoreClient;
  storage?: KeyValueStore | null;
  /** The tokens this page keeps per hub (default: the page's storage, as the sessions use). */
  tokens?: HubTokens;
}

interface Conn {
  core: CoreClient;
  hello: LinkHelloRecord;
}

/** The hub installation's sources: one per DSPi of the inventory, offline ones too. */
export function inventorySources(url: string, devices: readonly HubDeviceRecord[]): Source[] {
  return devices.map((d) => ({ kind: "hub" as const, url, deviceId: d.serial, name: d.name || d.serial, online: d.online }));
}

/** A role's access: a Viewer views, Control and Admin control. */
export function accessOf(role: string): Access {
  return role === "viewer" ? "view" : "control";
}

export class HubBackend implements FleetBackend {
  readonly kind = "hub";
  private readonly local: LocalBackend;
  private readonly tokens: HubTokens;
  private conn: Promise<Conn | null> | null = null;
  /** The hub has no inventory (an upstream hub): it is not asked again. */
  private noInventory = false;

  constructor(private readonly opts: HubBackendOptions) {
    const storage = opts.storage === undefined ? safeLocalStorage() : opts.storage;
    this.local = new LocalBackend({ storage, servingHub: opts.servingHub });
    this.tokens = opts.tokens ?? new LocalHubTokens(storage);
  }

  async installations(): Promise<Installation[]> {
    const own = await this.local.installations();
    const hub = await this.hubInstallation().catch(() => null);
    return hub ? [hub, ...own] : own;
  }

  addHub(installationId: string | null, url: string, name: string): Promise<Installation> {
    return this.local.addHub(installationId, url, name);
  }

  removeSource(installationId: string, source: Source): Promise<void> {
    return this.local.removeSource(installationId, source);
  }

  async access(installationId: string, _source: Source, _deviceId: string): Promise<Access> {
    if (installationId !== HUB_INSTALLATION_ID) return this.local.access();
    const c = await this.connect();
    if (!c) return "control";
    const role = await c.core.call("hubRole", undefined).then((r) => r?.role ?? "", () => "");
    return accessOf(role);
  }

  async user(): Promise<null> {
    return null;
  }

  /**
   * The hub's `mode.pinned` reaches each session through its own core (the lock state's
   * `hubPinnedMode`, applied by `modeResolve`), for the role the hub gave that session: the
   * backend pins nothing on top of it.
   */
  async pinnedMode(): Promise<PinnedMode | null> {
    return null;
  }

  /** The serving hub as an installation, or null (no inventory, a PIN first, not reached). */
  private async hubInstallation(): Promise<Installation | null> {
    if (this.noInventory) return null;
    const c = await this.connect();
    if (!c) return null;
    const caps = await c.core.call("hubCaps", undefined).catch(() => null);
    if (!caps?.onHub) return null;
    if (!caps.inventory) {
      // Nothing of the backend's own is needed from this hub: its session goes.
      this.noInventory = true;
      this.drop(c);
      return null;
    }
    let devices: HubDeviceRecord[];
    try {
      devices = await c.core.call("hubInventory", undefined);
    } catch {
      // The session went: connect again next time.
      this.drop(c);
      return null;
    }
    return {
      id: HUB_INSTALLATION_ID,
      name: c.hello.hubName || "This hub",
      sources: inventorySources(this.opts.servingHub, devices),
      editable: false,
    };
  }

  /** The backend's own DSPi Link session to the serving hub; null when the hub wants a PIN first or is not reached. */
  private connect(): Promise<Conn | null> {
    this.conn ??= (async () => {
      const url = this.opts.servingHub;
      let core: CoreClient | null = null;
      try {
        core = this.opts.spawnCore();
        const saved = await this.tokens.get(url);
        const hello = await core.call("linkConnect", { url, token: saved?.token ?? "", hubId: saved?.hubId ?? "" });
        if (hello.tokenRefused) this.tokens.forget(url);
        if (hello.needsPairing) {
          // The user pairs through the hub's own entry; this backend asks again next time.
          await core.dispose().catch(() => undefined);
          this.conn = null;
          return null;
        }
        return { core, hello };
      } catch {
        await core?.dispose().catch(() => undefined);
        this.conn = null;
        return null;
      }
    })();
    return this.conn;
  }

  private drop(c: Conn): void {
    this.conn = null;
    void c.core.dispose().catch(() => undefined);
  }
}
