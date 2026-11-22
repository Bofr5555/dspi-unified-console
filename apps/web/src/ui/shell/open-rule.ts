// -- WP WEBM1 -- One session per DSPi (MPLAN 1.2): a unit already open here (over USB, or through
// any hub) is not opened a second time, as the core's `CoreSet` refuses it for the desktop app.
// Two writers on one unit are never allowed. Demo devices and documents are no real unit: every
// one is its own.

import type { Source } from "../../fleet/types";
import type { SessionState } from "../session";

/** What the rule reads of a session. */
export interface Holder {
  readonly source: Source;
  readonly deviceId: string;
  readonly installationName: string;
  readonly state: Pick<SessionState, "status" | "snapshot">;
}

/** The serial `source` names before anything is opened, "" when it names none (or no real unit). */
export function serialOf(source: Source): string {
  if (source.kind === "usb") return source.serial ?? "";
  return source.kind === "hub" ? (source.deviceId ?? "") : "";
}

/** Whether `s` has (or is opening) the unit `serial`. A lost or closed session has given it up. */
function holds(s: Holder, serial: string): boolean {
  if (s.source.kind !== "usb" && s.source.kind !== "hub") return false;
  if (s.state.status === "failed" || s.state.status === "closed") return false;
  return serialOf(s.source) === serial || s.deviceId === serial || s.state.snapshot?.device.serial === serial;
}

/** The session, other than `except`, that already has the unit `serial`; null when none has (or `serial` is ""). */
export function holderOf<S extends Holder>(sessions: readonly S[], serial: string, except: S | null = null): S | null {
  if (!serial) return null;
  return sessions.find((s) => s !== except && holds(s, serial)) ?? null;
}

/** Why a second session is not opened, naming the one that has it. */
export function alreadyOpenText(holder: Holder): string {
  const name = holder.state.snapshot?.device.name || "DSPi";
  return `This DSPi is already open: ${holder.installationName} · ${name}, ${transportName(holder.source)}`;
}

/** How a session reaches its device, as its tab says it. */
export function transportName(source: Source): string {
  switch (source.kind) {
    case "usb":
      return "USB";
    case "hub":
      return "Hub";
    case "demo":
      return "Demo";
    case "document":
      return "File";
  }
}
