// WP P2WB2: the "On the hub" section's model. The merged list and its rules (the hub's copy wins on
// a differing id; `where` is `local`, `hub` or `both`) are the core's (`snapshotListMerged`); the web
// keeps no snapshot store of its own, so its rows are the hub's. A document over the hub's
// 61 440-byte compact cap is refused by the core with its own words, nothing sent.

import { CoreCallError } from "../../core/protocol";
import type { MergedSnapshotRecord } from "../../core/hub-settings-protocol";
import { noticeOf } from "../../ui/texts";

/** The core's `where`, in words. */
export function whereLabel(where: string): string {
  return where === "hub" ? "On the hub" : where === "both" ? "Here and on the hub" : where === "local" ? "Here" : where;
}

/** A row's words: when it was taken, its note. */
export function rowNote(r: MergedSnapshotRecord): string {
  const t = r.snapshot.savedMs ? new Date(r.snapshot.savedMs).toLocaleString() : "";
  return [t, r.snapshot.note, r.localCopy ? "this console's copy" : ""].filter(Boolean).join(" · ");
}

/**
 * A refusal in the core's words: the notice row's title, then the core's own message (for the
 * snapshot cap, the size and the limit); the message alone without a row.
 */
export function hubFailure(e: unknown): string {
  const n = noticeOf(e);
  const m = e instanceof Error ? e.message : String(e);
  const said = m ? m[0]!.toUpperCase() + m.slice(1) : m;
  if (n) return `${n.title}: ${said || n.reason}`;
  return e instanceof CoreCallError || e instanceof Error ? said : String(e);
}

/** The "Save to the hub" source: the device now, or one of the held documents by id. */
export const SAVE_DEVICE = "device";
