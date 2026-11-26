// -- WP UB14 -- (B14, D14: revisits OD-P1b) where the web keeps its held snapshots: the browser's
// IndexedDB, one list per DSPi serial, so a reload or the next evening finds them again. Where the
// browser keeps no site data (a private window, blocked storage) they last until the page closes,
// and the page says so. Export… in a row's menu saves one as a file. The hub's shared copies are
// the hub's (HubSnapshots); this is the browser's own.
//
// Each change reads the list again before it writes, so two tabs on the same DSPi add to and
// delete from one list (the last writer of the same moment wins; nothing is merged inside a document).

import { IdbTextStore, MemoryTextStore, type TextStore } from "../files/model";

/** One kept snapshot: the document's text, as the desk holds it (its list words are read again from it). */
export interface KeptDoc {
  /** Stable across visits and tabs (the desk's row ids are not). */
  uid: string;
  fileName: string;
  text: string;
  origin: "captured" | "file";
}

/** The key of a DSPi's list in the page's text store. */
export const SNAPSHOTS_KEY = "dspi-web.snapshots.";

/** Where held snapshots are kept: `persistent` when they outlive the page (the browser's IndexedDB). */
export class SnapshotStore {
  constructor(
    private readonly texts: TextStore,
    readonly persistent: boolean,
  ) {}

  /** The snapshots kept for `serial`, oldest first; none when nothing (or nothing readable) is kept. */
  async list(serial: string): Promise<KeptDoc[]> {
    const raw = await this.texts.get(SNAPSHOTS_KEY + serial);
    if (!raw) return [];
    try {
      const v = JSON.parse(raw) as { docs?: unknown };
      return Array.isArray(v.docs) ? v.docs.filter(isKept) : [];
    } catch {
      return [];
    }
  }

  /** Keep `doc` for `serial` (once: a uid already kept is left as it is). */
  async add(serial: string, doc: KeptDoc): Promise<void> {
    const docs = await this.list(serial);
    if (docs.some((d) => d.uid === doc.uid)) return;
    await this.write(serial, [...docs, doc]);
  }

  /** Let the snapshot `uid` of `serial` go. */
  async remove(serial: string, uid: string): Promise<void> {
    const docs = await this.list(serial);
    const left = docs.filter((d) => d.uid !== uid);
    if (left.length === docs.length) return;
    if (left.length) await this.write(serial, left);
    else await this.texts.remove(SNAPSHOTS_KEY + serial);
  }

  private write(serial: string, docs: KeptDoc[]): Promise<void> {
    return this.texts.set(SNAPSHOTS_KEY + serial, JSON.stringify({ v: 1, docs }));
  }
}

function isKept(d: unknown): d is KeptDoc {
  const x = d as Partial<KeptDoc> | null;
  return !!x && typeof x.uid === "string" && typeof x.text === "string" && typeof x.fileName === "string" && (x.origin === "captured" || x.origin === "file");
}

/** This page's memory only (no IndexedDB): the snapshots last until the page closes. */
export function memorySnapshotStore(): SnapshotStore {
  return new SnapshotStore(new MemoryTextStore(), false);
}

/** The browser's IndexedDB when the page has it, otherwise this page's memory. */
export function browserSnapshotStore(): SnapshotStore {
  try {
    if (typeof indexedDB !== "undefined") return new SnapshotStore(new IdbTextStore(), true);
  } catch {
    // blocked site data: until the page closes
  }
  return memorySnapshotStore();
}

/** A new kept snapshot's uid. */
export function newUid(): string {
  const c = (globalThis as { crypto?: { randomUUID?: () => string } }).crypto;
  return c?.randomUUID ? c.randomUUID() : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}
