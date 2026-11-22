// WP WEBM3: the History page's words (Library › History, MODLAY 2): the core's journal of this
// connection (`history`, newest first) as rows a person reads. The paths' names are the
// registry's (`describe`), as the session's step notes name them.

import type { HistoryEntry, ParamValue } from "../../core/protocol";

/** How many changes the page shows (the core keeps the newest 1000). */
export const HISTORY_ROWS = 200;

/** One row of the page. */
export interface HistoryRow {
  key: string;
  time: string;
  what: string;
  values: string;
  /** Who made it, when not the person here: "Undo", "Redo", "Another writer". */
  source: string;
  undoable: boolean;
}

/** A value as the row shows it; "" for a trigger or a value the page cannot say shortly. */
export function valueWords(v: ParamValue | null): string {
  if (v === null || typeof v !== "object") return "";
  if ("bool" in v) return v.bool ? "On" : "Off";
  if ("float" in v) return String(Math.round(v.float * 100) / 100).replace("-", "−");
  if ("int" in v) return String(v.int);
  if ("choice" in v) return `#${v.choice}`;
  if ("mask" in v) return `0x${v.mask.toString(16)}`;
  if ("text" in v) return `“${v.text}”`;
  return "";
}

/** What the change was to: the path's name, then its indices counted from 1 ("Gain · 3"). */
export function whatWords(label: string, indices: readonly number[]): string {
  return [label, ...indices.map((i) => String(i + 1))].join(" · ");
}

/** Who made a change, in words; "" for the person's own. */
export function sourceWords(source: string): string {
  switch (source) {
    case "user":
    case "":
      return "";
    case "undo":
      return "Undo";
    case "redo":
      return "Redo";
    default:
      return source[0]!.toUpperCase() + source.slice(1).replace(/_/g, " ");
  }
}

/** The clock time today, the date and time before. */
export function timeWords(atMs: number, now: number): string {
  const d = new Date(atMs);
  const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}:${String(d.getSeconds()).padStart(2, "0")}`;
  return new Date(now).toDateString() === d.toDateString() ? hm : `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${hm}`;
}

/** The page's rows, newest first, with the names `label` gives each path. */
export function historyRows(entries: readonly HistoryEntry[], label: (path: string) => string, now: number): HistoryRow[] {
  return entries.map((e, i) => {
    const before = valueWords(e.before);
    const after = valueWords(e.after);
    return {
      key: `${e.atMs}:${i}:${e.path}`,
      time: timeWords(e.atMs, now),
      what: whatWords(label(e.path), e.indices),
      values: before && after ? `${before} → ${after}` : after,
      source: sourceWords(e.source),
      undoable: e.undoable,
    };
  });
}

// -- WP WEBM5 -- (review RC16) the page reads the journal again on every change of the device's
// state; when it is the same list the rows are not made again.
/** True when `b` is the same journal as `a` (the same changes, in the same order). */
export function sameHistory(a: readonly HistoryEntry[], b: readonly HistoryEntry[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i]!;
    const y = b[i]!;
    if (x.atMs !== y.atMs || x.path !== y.path || x.source !== y.source || x.undoable !== y.undoable || x.indices.join() !== y.indices.join()) return false;
  }
  return true;
}
