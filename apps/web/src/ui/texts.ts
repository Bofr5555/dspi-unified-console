// Shared wording (WP WB5). Link phases come from the core's table (CG10),
// generated into ci/generated/web/ by ci/drift.sh until WB3's binding answers it
// at run time. The About data is assets/about.json, which every client reads.

import linkWords from "../../../../ci/generated/web/link-words.json";
import about from "../../../../assets/about.json";
import type { LinkPhase } from "../core/protocol";

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
