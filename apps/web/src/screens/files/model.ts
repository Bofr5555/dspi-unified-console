// WP WB6K: the Files screen's filter files, its saves to the device, and the AutoEQ database the
// page keeps (M28, M30, M31, M32 over CG6). Every format is the core's: a filter file is read,
// inspected and applied by `filtersExport`/`filtersInspect`/`filtersImport` (one undo step), and
// a database is checked, searched and read back by `autoeqDatabase*`/`autoeqSearch`. The page
// only moves bytes: it downloads, reads the chosen file, makes the Download's GET (its answer,
// whatever the status, goes to the core) and keeps the files in its own storage: the user's
// database in IndexedDB (too large for localStorage), the favourites in localStorage. The
// built-in database is served beside the console (`autoeq_database.json`).

import type { AutoEqDatabaseRecord, AutoEqHit, FilterFileInspect, FilterImportReport } from "../../core/protocol";
import { safeLocalStorage, type KeyValueStore } from "../../fleet/local";
import type { Session } from "../../ui/session";
import type { SnapshotDesk } from "../snapshots/model";

// ---------------------------------------------------------------- filter files (M28)

/** The inputs a filter file carries: the device's. */
export function filterInputs(session: Session): number {
  return session.state.snapshot?.state.numInputChannels ?? 2;
}

/** The export's file name, as the Console offers it. */
export const FILTERS_FILE = "DSPi Filters.txt";

/** The device's bands as a filter file's text, or null when it failed (the desk shows why). */
export function exportFilters(desk: SnapshotDesk): Promise<string | null> {
  const session = desk.session;
  return desk.act("Export Filters", async () => (await session.core.call("filtersExport", { inputs: filterInputs(session) })).text, false);
}

/** What a filter file holds, for the picker; an error in words when nothing in it applies. */
export async function inspectFilters(session: Session, text: string): Promise<FilterFileInspect> {
  return session.core.call("filtersInspect", { text, inputs: filterInputs(session) });
}

/** Why the picker offers nothing, or "" when it offers channels. */
export function inspectProblem(i: FilterFileInspect): string {
  if (i.kind === "error") return i.error || "The file could not be read.";
  if (!i.channels.length) return "No filters this device can use were found in the file.";
  return "";
}

/** The picker's wires, as ticked. */
export function chosenWires(channels: { wire: number }[], ticked: Set<number>): number[] {
  return channels.map((c) => c.wire).filter((w) => ticked.has(w));
}

/** Apply a filter file to `wires`, one undo step; the report as the desk's outcome. */
export async function importFilters(desk: SnapshotDesk, text: string, name: string, wires: number[]): Promise<boolean> {
  const session = desk.session;
  const r = await desk.act("Import Filters", async () => {
    const report: FilterImportReport = await session.core.call("filtersImport", { text, wires });
    await session.refresh();
    return report;
  });
  if (!r) return false;
  if (!r.ok) {
    desk.report({ title: "Import Filters: not done", lines: [r.error, ...r.lines].filter(Boolean), notes: [], tone: "error" });
    return false;
  }
  desk.report({ title: `Imported filters from “${name}”`, lines: r.lines, notes: [], tone: r.lines.length ? "warn" : "info" });
  return true;
}

// ---------------------------------------------------------------- saves to the device (M30)

/** The saved master volume in words, as the Console says it. */
export function masterVolumeWords(db: number): string {
  return db <= -128 ? "−∞ dB (mute)" : `${db.toFixed(1)} dB`;
}

/** Save Master Volume: the live master volume is what the device starts with. */
export async function saveMasterVolume(desk: SnapshotDesk): Promise<boolean> {
  const session = desk.session;
  const db = session.state.snapshot?.state.masterVolumeDb ?? 0;
  const done = await desk.act("Save Master Volume", async () => {
    await session.core.call("write", { path: "vol.master.save", value: "trigger" });
    return true;
  });
  if (!done) return false;
  desk.report({ title: "Save Master Volume", lines: [`Master volume saved (${masterVolumeWords(db)}). It will be applied on next boot.`], notes: [], tone: "info" });
  return true;
}

/** Save Output Configuration: the live output configuration is what the device starts with. */
export async function saveOutputConfig(desk: SnapshotDesk): Promise<boolean> {
  const session = desk.session;
  const done = await desk.act("Save Output Configuration", async () => {
    const token = await session.token("save_output_config");
    await session.core.call("write", { path: "dev.save.io", value: "trigger", ...(token ? { token } : {}) });
    return true;
  });
  if (!done) return false;
  desk.report({ title: "Save Output Configuration", lines: ["Output configuration saved. It will be applied on next boot."], notes: [], tone: "info" });
  return true;
}

// ---------------------------------------------------------------- the AutoEQ database (M31, M32)

/** Where the user's database is kept (IndexedDB) and the favourites (localStorage). */
export const DATABASE_KEY = "dspi-web.autoeq.database";
export const FAVOURITES_KEY = "dspi-web.autoeq.favourites";

/** A store for a large text: the page's IndexedDB, or this visit's memory without it. */
export interface TextStore {
  get(key: string): Promise<string | null>;
  set(key: string, text: string): Promise<void>;
  remove(key: string): Promise<void>;
}

/** This visit only: where the page has no IndexedDB (a private window, blocked site data). */
export class MemoryTextStore implements TextStore {
  private readonly map = new Map<string, string>();
  async get(key: string) {
    return this.map.get(key) ?? null;
  }
  async set(key: string, text: string) {
    this.map.set(key, text);
  }
  async remove(key: string) {
    this.map.delete(key);
  }
}

/** The page's IndexedDB, one object store of texts by key. */
export class IdbTextStore implements TextStore {
  private db: Promise<IDBDatabase> | null = null;
  private open(): Promise<IDBDatabase> {
    this.db ??= new Promise((resolve, reject) => {
      const r = indexedDB.open("dspi-web", 1);
      r.onupgradeneeded = () => r.result.createObjectStore("texts");
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error ?? new Error("the page's storage could not be opened"));
    });
    return this.db;
  }
  private async run<T>(mode: IDBTransactionMode, f: (s: IDBObjectStore) => IDBRequest<T>): Promise<T> {
    const db = await this.open();
    return new Promise((resolve, reject) => {
      const r = f(db.transaction("texts", mode).objectStore("texts"));
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error ?? new Error("the page's storage refused it"));
    });
  }
  async get(key: string) {
    const v = await this.run("readonly", (s) => s.get(key) as IDBRequest<unknown>);
    return typeof v === "string" ? v : null;
  }
  async set(key: string, text: string) {
    await this.run("readwrite", (s) => s.put(text, key));
  }
  async remove(key: string) {
    await this.run("readwrite", (s) => s.delete(key));
  }
}

function defaultTextStore(): TextStore {
  try {
    if (typeof indexedDB !== "undefined") return new IdbTextStore();
  } catch {
    // blocked site data: this visit only
  }
  return new MemoryTextStore();
}

/** A GET the page makes: what the server answered, whatever the status. */
export interface PageResponse {
  status: number;
  body: string;
  retryAfterS?: number;
  rateLimitRemaining?: number;
  rateLimitReset?: number;
}
export type PageFetch = (url: string) => Promise<PageResponse>;

function header(h: Headers, name: string): number | undefined {
  const v = h.get(name);
  if (v === null || !/^\d+$/.test(v.trim())) return undefined;
  return Number(v.trim());
}

/** The browser's fetch, its answer as the core reads one (`Retry-After` in seconds only). */
export const browserFetch: PageFetch = async (url) => {
  const r = await fetch(url, { credentials: "omit", cache: "no-cache" });
  return {
    status: r.status,
    body: await r.text(),
    retryAfterS: header(r.headers, "Retry-After"),
    rateLimitRemaining: header(r.headers, "X-RateLimit-Remaining"),
    rateLimitReset: header(r.headers, "X-RateLimit-Reset"),
  };
};

/** Words for a failure: the core's, with a capital. */
export function failureWords(e: unknown): string {
  const m = e instanceof Error ? e.message : String(e);
  return m ? m[0]!.toUpperCase() + m.slice(1) : "It did not work.";
}

/** Only an http(s) address is fetched; anything else is refused before a request. */
export function downloadAddress(input: string): string | null {
  try {
    const u = new URL(input.trim(), typeof location === "undefined" ? undefined : location.href);
    return u.protocol === "https:" || u.protocol === "http:" ? u.href : null;
  } catch {
    return null;
  }
}

export interface LibraryState {
  /** The database the sessions use, null before one is loaded. */
  info: AutoEqDatabaseRecord | null;
  /** Why no database could be loaded, "" when one is. */
  problem: string;
  favourites: string[];
  busy: string;
}

/**
 * The page's AutoEQ database and favourites, one for every session of the page (as the Console
 * keeps one database). Each session's core holds its own copy of the database, loaded when it
 * is first asked and again after a change (`generation`).
 */
export class AutoEqLibrary {
  state: LibraryState = { info: null, problem: "", favourites: [], busy: "" };
  private readonly listeners = new Set<() => void>();
  private generation = 1;
  private readonly loaded = new WeakMap<Session, number>();
  private text: Promise<{ text: string; user: boolean }> | null = null;

  constructor(
    private readonly builtInUrl: string | URL,
    private readonly store: TextStore = defaultTextStore(),
    private readonly prefs: KeyValueStore | null = safeLocalStorage(),
    private readonly get: PageFetch = browserFetch,
  ) {}

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private set(patch: Partial<LibraryState>): void {
    this.state = { ...this.state, ...patch };
    for (const l of [...this.listeners]) l();
  }

  /** The database's text: the user's kept one, else the built-in one. */
  private source(): Promise<{ text: string; user: boolean }> {
    this.text ??= (async () => {
      const kept = await this.store.get(DATABASE_KEY).catch(() => null);
      if (kept) return { text: kept, user: true };
      const r = await this.get(String(this.builtInUrl));
      if (r.status !== 200) throw new Error(`the built-in AutoEQ database could not be loaded (status ${r.status})`);
      return { text: r.body, user: false };
    })();
    this.text.catch(() => (this.text = null));
    return this.text;
  }

  /** The session's core holds the page's database (loaded once per change). Answers whether it does. */
  async ready(session: Session): Promise<boolean> {
    if (this.loaded.get(session) === this.generation) return true;
    const gen = this.generation;
    try {
      const src = await this.source();
      let info: AutoEqDatabaseRecord;
      try {
        info = await session.core.call("autoeqDatabaseLoad", src);
      } catch (e) {
        if (!src.user) throw e;
        // A kept file that no longer reads: the built-in one, and Reset stays offered.
        info = { ...(await session.core.call("autoeqDatabaseLoad", { ...(await this.builtIn()), user: false })), hasUserDatabase: true, problem: failureWords(e) };
      }
      if (gen === this.generation) this.loaded.set(session, gen);
      this.set({ info, problem: "" });
      await this.readFavourites(session);
      return true;
    } catch (e) {
      this.set({ problem: failureWords(e) });
      return false;
    }
  }

  private async builtIn(): Promise<{ text: string }> {
    const r = await this.get(String(this.builtInUrl));
    if (r.status !== 200) throw new Error(`the built-in AutoEQ database could not be loaded (status ${r.status})`);
    return { text: r.body };
  }

  private changed(text: string | null, user: boolean, info: AutoEqDatabaseRecord, session: Session): void {
    this.generation++;
    this.text = text === null ? null : Promise.resolve({ text, user });
    this.loaded.set(session, this.generation);
    this.set({ info, problem: "" });
  }

  private async busy<T>(what: string, f: () => Promise<T>): Promise<T> {
    this.set({ busy: what });
    try {
      return await f();
    } finally {
      this.set({ busy: "" });
    }
  }

  /** Import File…: the core checks it; kept only when it is a database. Throws the core's refusal. */
  importFile(session: Session, text: string): Promise<AutoEqDatabaseRecord> {
    return this.busy("Import File", async () => {
      const info = await session.core.call("autoeqDatabaseLoad", { text, user: true });
      await this.store.set(DATABASE_KEY, text);
      this.changed(text, true, info, session);
      return info;
    });
  }

  /** Download…: the page's GET of `url`; the core reads the answer; kept only when it is a database. */
  download(session: Session, url: string): Promise<AutoEqDatabaseRecord> {
    return this.busy("Download", async () => {
      let r: PageResponse;
      try {
        r = await this.get(url);
      } catch {
        // No answer at all (offline, refused by the server's sharing rules): no status to read.
        throw new Error(`no answer came from ${url}`);
      }
      const info = await session.core.call("autoeqDatabaseDownload", { url, ...r });
      await this.store.set(DATABASE_KEY, r.body);
      this.changed(r.body, true, info, session);
      return info;
    });
  }

  /** Reset to Built-in: the kept file goes, the built-in database comes back. */
  resetToBuiltIn(session: Session): Promise<AutoEqDatabaseRecord> {
    return this.busy("Reset to Built-in", async () => {
      await this.store.remove(DATABASE_KEY);
      const src = await this.builtIn();
      const info = await session.core.call("autoeqDatabaseLoad", { text: src.text, user: false });
      this.changed(src.text, false, info, session);
      return info;
    });
  }

  // ---- favourites (M31): ids in the page's storage, as the Console keeps them in its defaults

  /** The favourites file's text as last kept this visit (it holds when storage refused it). */
  private favText: string | null = null;

  private favouritesText(): string {
    if (this.favText !== null) return this.favText;
    try {
      return this.prefs?.getItem(FAVOURITES_KEY) ?? "";
    } catch {
      return "";
    }
  }

  private async readFavourites(session: Session): Promise<void> {
    const r = await session.core.call("autoeqFavourites", { text: this.favouritesText() });
    this.set({ favourites: r.ids });
  }

  /** The favourites the database holds, in the order they were added. */
  async favouriteHits(session: Session): Promise<AutoEqHit[]> {
    if (!(await this.ready(session))) return [];
    return (await session.core.call("autoeqFavourites", { text: this.favouritesText() })).hits;
  }

  isFavourite(id: string): boolean {
    return this.state.favourites.includes(id);
  }

  private async keep(session: Session, ids: string[]): Promise<void> {
    const { text } = await session.core.call("autoeqFavouritesText", { ids });
    this.favText = text;
    try {
      this.prefs?.setItem(FAVOURITES_KEY, text);
    } catch {
      // storage refused: they hold for this visit
    }
    this.set({ favourites: ids });
  }

  /** The heart: add or remove `id`. */
  toggleFavourite(session: Session, id: string): Promise<void> {
    const ids = this.state.favourites;
    return this.keep(session, ids.includes(id) ? ids.filter((i) => i !== id) : [...ids, id]);
  }

  /** Clear Favorites. */
  clearFavourites(session: Session): Promise<void> {
    return this.keep(session, []);
  }

  /** Profiles matching `query`. */
  async search(session: Session, query: string): Promise<{ hits: AutoEqHit[]; total: number }> {
    if (!query.trim() || !(await this.ready(session))) return { hits: [], total: 0 };
    return session.core.call("autoeqSearch", { query, limit: 50 });
  }

  /** One profile as autoeqPreview takes it. */
  async entry(session: Session, id: string): Promise<unknown> {
    await this.ready(session);
    return session.core.call("autoeqEntry", { id });
  }
}

// -- The page's one library: the built-in database beside the console's script.
let pageLibrary: AutoEqLibrary | null = null;

/** The page's AutoEQ library (every session of the page shares it). */
export function autoEqLibrary(): AutoEqLibrary {
  pageLibrary ??= new AutoEqLibrary(new URL("./autoeq_database.json", import.meta.url));
  return pageLibrary;
}

/** "Current database: <date>, <n> profiles", as the Update Database dialog says it. */
export function databaseLine(info: AutoEqDatabaseRecord | null): string {
  if (!info) return "No AutoEQ database is loaded yet.";
  const when = info.generatedAt ? dateWords(info.generatedAt) : "Unknown";
  const which = info.hasUserDatabase ? "your own" : "the built-in one";
  return `Current database: ${when}, ${info.entryCount} profiles (${which}).`;
}

function dateWords(iso: string): string {
  const t = Date.parse(iso);
  return Number.isFinite(t) ? new Date(t).toISOString().slice(0, 10) : iso;
}
