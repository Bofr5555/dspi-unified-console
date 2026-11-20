// Shared wording (WP WB5). Link phases come from the core's table (CG10),
// generated into ci/generated/web/ by ci/drift.sh until WB3's binding answers it
// at run time. The About data is assets/about.json, which every client reads.

import linkWords from "../../../../ci/generated/web/link-words.json";
import about from "../../../../assets/about.json";
import { CoreCallError, type LinkPhase, type NoticeRecord } from "../core/protocol"; // -- WP P1WB2 -- notices

export interface LinkText {
  key: string;
  phase: string;
  text: string;
  action_key: string | null;
  action: string | null;
}

const table = (linkWords as { link: LinkText[] }).link;

/** The wording for a phase; `pinSource` matters only to `needs_pairing`. */
export function linkText(phase: LinkPhase, pinSource = ""): LinkText {
  const key = phase === "needs_pairing" ? `needs_pairing.${pinSource === "fixed" ? "fixed" : "window"}` : phase;
  const hit = table.find((t) => t.key === key);
  if (!hit) throw new Error(`no wording for Link phase ${phase}`);
  return hit;
}

export interface AboutData {
  draft?: boolean;
  credit_lines: { id: string; text: string }[];
  makers: { id: string; name: string; url: string; made: string[] }[];
  links_title: string;
  links_note: string;
  links: { id: string; title: string; detail: string; url: string }[];
  credits: { id: string; name: string; what: string; url?: string }[];
  // -- WP LIC1 -- the project's licence and its GPL v3 section 7 terms
  licence: {
    id: string;
    title: string;
    spdx: string;
    copyright: string[];
    statement: string;
    terms_title: string;
    terms_note: string;
    terms: { id: string; title: string; text: string }[];
    freedoms: string;
    notices: string;
  };
}

export const aboutData = about as AboutData;

// -- WP P1WB2 -- the core's notice table (P1NT, `assets/notices/notices.json`): one row per
// error key, with a title, the reason and what to do, in the words every client shows. Read
// once from a core (`noticeTexts`); the rows are the same in every session.


let rows: Map<string, NoticeRecord> | null = null;
let loading: Promise<void> | null = null;

/** Read the table from a core, once per page; a failure leaves it unread (asked again next time). */
export function loadNotices(core: { call(method: "noticeTexts", params: undefined): Promise<NoticeRecord[]> }): Promise<void> {
  if (rows) return Promise.resolve();
  loading ??= core.call("noticeTexts", undefined).then(
    (list) => {
      if (Array.isArray(list)) rows = new Map(list.map((n) => [n.key, n]));
      else loading = null;
    },
    () => {
      loading = null;
    },
  );
  return loading;
}

/** The row for `key`, or null before the table is read or for a key it does not have. */
export function noticeRow(key: string): NoticeRecord | null {
  return rows?.get(key) ?? null;
}

/** A failure's notice: its key's row, with the core's own words as the detail. Null without one. */
export function noticeOf(e: unknown): NoticeRecord | null {
  if (!(e instanceof CoreCallError) || !e.noticeKey) return null;
  const row = noticeRow(e.noticeKey);
  return row ? { ...row, detail: e.message } : null;
}

/** Tests only: forget the table. */
export function resetNotices(): void {
  rows = null;
  loading = null;
}

// -- WP REVERTW -- a revert that was not fully successful (owner 2026-11-19): the core's
// `revert.*` rows (`revert.incomplete`, `revert.failed`, `revert.not_written`, `revert.unknown`),
// carried as `revertNotice` on a guard's end and a system apply, and as a tune's issue key.

/** The words every `revert.*` row shares, for a table not read yet. */
export const REVERT_INCOMPLETE_TITLE = "The revert was not fully successful";
export const REVERT_INCOMPLETE_ACTION = "Check the settings, or restore a backup (a snapshot).";

/** A `revert.*` notice's title, reason and action; null for no key (a clean end). */
export function revertNotice(
  key: string | null | undefined,
  row: (key: string) => NoticeRecord | null = noticeRow,
): { key: string; title: string; reason: string; action: string } | null {
  if (!key) return null;
  const r = row(key);
  return {
    key,
    title: r?.title || REVERT_INCOMPLETE_TITLE,
    reason: r?.reason ?? "",
    action: r?.action || REVERT_INCOMPLETE_ACTION,
  };
}
