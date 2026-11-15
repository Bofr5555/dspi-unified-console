// WP WB6j: dashboard cards per row (M59, the Console's DashboardLayoutPanel): Auto, or a fixed
// number of cards per row for the main screen's cards. The console's choice, kept in this page's
// storage, the same for every session it shows.

import { useEffect, useState } from "preact/hooks";
import { safeLocalStorage, type KeyValueStore } from "../../fleet/local";

/** 0 is Auto; 1..3 fixed (the Console's range). */
export const MAX_CARDS_PER_ROW = 3;
const KEY = "dspi-web.dashboard";

export function tidyPerRow(v: unknown): number {
  return typeof v === "number" && Number.isInteger(v) && v >= 0 && v <= MAX_CARDS_PER_ROW ? v : 0;
}

export function loadPerRow(store: KeyValueStore | null): number {
  try {
    const t = store?.getItem(KEY);
    return tidyPerRow(t ? (JSON.parse(t) as { cardsPerRow?: unknown }).cardsPerRow : 0);
  } catch {
    return 0;
  }
}

export function savePerRow(store: KeyValueStore | null, n: number): void {
  try {
    store?.setItem(KEY, JSON.stringify({ cardsPerRow: tidyPerRow(n) }));
  } catch {
    // Refused storage: the choice lasts for this page.
  }
}

/** A fixed choice's name, and Auto's. */
export function perRowLabel(n: number): string {
  if (n === 0) return "Auto";
  return n === 1 ? "1 card per row" : `${n} cards per row`;
}

/** What the choice does, in the Console's words. */
export function perRowSummary(n: number): string {
  if (n === 0) return "Fits as many cards per row as the window allows.";
  return n === 1 ? "Up to 1 card per row." : `Up to ${n} cards per row.`;
}

/** The columns for `count` cards: the fixed choice up to the cards there are, at least one. */
export function columnsFor(chosen: number, count: number): number {
  return Math.max(1, Math.min(chosen, count));
}

const listeners = new Set<() => void>();
let shared: number | null = null;

/** The console's choice and its setter; every main screen on the page follows a change at once. */
export function usePerRow(store: KeyValueStore | null = safeLocalStorage()): [number, (n: number) => void] {
  shared ??= loadPerRow(store);
  const [n, setN] = useState(shared);
  useEffect(() => {
    const l = () => setN(shared ?? 0);
    listeners.add(l);
    return () => {
      listeners.delete(l);
    };
  }, []);
  const set = (v: number) => {
    shared = tidyPerRow(v);
    savePerRow(store, shared);
    for (const l of listeners) l();
  };
  return [n, set];
}
