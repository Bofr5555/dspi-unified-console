// The fleet model (WP WB5, owner's web decisions of 2026-11-08).
//
// The web console's main use is a hub page that gathers several DSPi
// installations. It runs standalone, and the same code must drop into a
// server-based fleet product that has sign-in and access control. So:
//
// - Nothing assumes one device, one hub or one user. An Installation is a place
//   (a room, a venue, a customer) with its sources; a page can show many, and
//   open sessions to several devices at once.
// - Where the list comes from, who the user is and what they may do are behind
//   FleetBackend. Standalone, LocalBackend keeps a hub list in this page's
//   storage and grants control everywhere. A fleet server supplies its own
//   backend (its API, its sign-in, its roles) without changing the UI.
// - The console never signs anyone in itself, never stores a server credential
//   and never decides access: it asks the backend.

/** Where a device can be reached from this page. */
export type Source =
  | { kind: "demo" }
  /**
   * USB through the browser (WebUSB). Without `serial`, "a DSPi on USB": the
   * browser's picker chooses one; with it, that DSPi (WP WB4a).
   */
  | { kind: "usb"; serial?: string }
  /**
   * A hub. Without `deviceId`, the console asks the hub which DSPis it has (Link's
   * choose_device step); with it (a fleet backend's `hubDevices`), that DSPi, by
   * its serial (WP WB4b).
   */
  | { kind: "hub"; url: string; deviceId?: string; name?: string };

/** A place with DSPi devices: one or more sources under one name. */
export interface Installation {
  id: string;
  name: string;
  sources: Source[];
  /** False when the user may not add or remove sources here. */
  editable: boolean;
}

/** What a user may do with the devices of an installation. */
export type Access = "control" | "view";

/** The signed-in user, when the backend has one. Standalone has none. */
export interface FleetUser {
  id: string;
  name: string;
}

export interface FleetBackend {
  /** A short name for logs and the About box: `local`, or the server's own. */
  readonly kind: string;
  /** Called again whenever the page wants a fresh list. */
  installations(): Promise<Installation[]>;
  /** Missing when the server manages the list. */
  addHub?(installationId: string | null, url: string, name: string): Promise<Installation>;
  removeSource?(installationId: string, source: Source): Promise<void>;
  /** What this user may do with a device of this installation. */
  access(installationId: string, source: Source, deviceId: string): Promise<Access>;
  /** Missing or null when nobody signs in (standalone). */
  user?(): Promise<FleetUser | null>;
  /**
   * Credentials for a hub, when a server hands them out: a Link token held for the
   * fleet, with the hub's id it belongs to when known (then it goes to that hub
   * only). Missing (standalone): the user pairs with the hub once and the token
   * is kept in this page's storage. A token the hub refuses is not offered again
   * in this visit; the user is asked for the PIN instead (WP WB4b).
   */
  hubCredentials?(url: string): Promise<{ token: string; hubId?: string } | null>;
  // -- WP WB3 -- the seam for the owner's decision of 2026-11-08; WB4b uses it.
  /**
   * The DSPis behind a hub, when the server knows them: `deviceId` is the DSPi's
   * serial, and the picker lists each, opening it without asking the hub. Missing,
   * or null for a hub (the default): the console asks the hub itself over Link
   * (its choose_device step).
   */
  hubDevices?(url: string): Promise<{ deviceId: string; name: string }[] | null>;
}

/** The wording shown when access is the backend's "view". */
export const VIEW_ONLY_REASON = "You can view this DSPi but not change it";

export function sourceKey(s: Source): string {
  // -- WP WB4a -- one key per USB DSPi
  if (s.kind === "usb" && s.serial) return `usb:${s.serial}`;
  // -- WP WB4b -- one per DSPi a fleet names on a hub
  if (s.kind === "hub" && s.deviceId) return `hub:${s.url}#${s.deviceId}`;
  return s.kind === "hub" ? `hub:${s.url}` : s.kind;
}
