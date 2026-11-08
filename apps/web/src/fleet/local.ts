// The standalone backend (WP WB5): no server, no sign-in, full control.
//
// One installation, "This console", always holds the demo device and USB. Hubs
// the user adds are kept in this page's storage, so they are this browser
// profile's own list; if storage is unavailable (private window, blocked site
// data) the list lasts for the visit only. When the page is served by a hub,
// that hub is listed first: a hub only lets its own pages in (Link spec 4, the
// Origin rule), so it is the one hub such a page is sure to reach.

import type { Access, FleetBackend, Installation, Source } from "./types";

const KEY = "dspi-web.hubs.v1";

export interface KeyValueStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export interface LocalOptions {
  storage?: KeyValueStore | null;
  /** The hub that served this page, if it was one. */
  servingHub?: string | null;
}

interface SavedHub {
  url: string;
  name: string;
}

export class LocalBackend implements FleetBackend {
  readonly kind = "local";
  private memory: SavedHub[] = [];
  private readonly storage: KeyValueStore | null;

  constructor(private readonly opts: LocalOptions = {}) {
    this.storage = opts.storage === undefined ? safeLocalStorage() : opts.storage;
    this.memory = this.load();
  }

  async installations(): Promise<Installation[]> {
    const sources: Source[] = [{ kind: "demo" }, { kind: "usb" }];
    const own: Installation = { id: "local", name: "This console", sources, editable: true };
    const hubs: Source[] = [];
    if (this.opts.servingHub) hubs.push({ kind: "hub", url: this.opts.servingHub });
    for (const h of this.memory) {
      if (!hubs.some((s) => s.kind === "hub" && s.url === h.url)) hubs.push({ kind: "hub", url: h.url });
    }
    own.sources.push(...hubs);
    return [own];
  }

  async addHub(_installationId: string | null, url: string, name: string): Promise<Installation> {
    const clean = normaliseHubUrl(url);
    if (!this.memory.some((h) => h.url === clean)) {
      this.memory.push({ url: clean, name: name.trim() || clean });
      this.save();
    }
    const all = await this.installations();
    return all[0]!;
  }

  async removeSource(_installationId: string, source: Source): Promise<void> {
    if (source.kind !== "hub") return;
    this.memory = this.memory.filter((h) => h.url !== source.url);
    this.save();
  }

  async access(): Promise<Access> {
    return "control";
  }

  async user(): Promise<null> {
    return null;
  }

  private load(): SavedHub[] {
    try {
      const raw = this.storage?.getItem(KEY);
      if (!raw) return [];
      const v: unknown = JSON.parse(raw);
      if (!Array.isArray(v)) return [];
      return v.filter(
        (h): h is SavedHub => typeof h === "object" && h !== null && typeof h.url === "string" && typeof h.name === "string",
      );
    } catch {
      return [];
    }
  }

  private save(): void {
    try {
      this.storage?.setItem(KEY, JSON.stringify(this.memory));
    } catch {
      // Storage refused: the list lasts for this visit.
    }
  }
}

/** `host:port`, `ws://host:port` or `http://host:port/` all become `ws://host:port`. */
export function normaliseHubUrl(input: string): string {
  let s = input.trim();
  if (!/^[a-z]+:\/\//i.test(s)) s = `ws://${s}`;
  const u = new URL(s);
  const scheme = u.protocol === "https:" || u.protocol === "wss:" ? "wss:" : "ws:";
  return `${scheme}//${u.host}`;
}

function safeLocalStorage(): KeyValueStore | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}
