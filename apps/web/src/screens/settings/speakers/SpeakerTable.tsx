// WP UB7 (UXP B7, C3): the one speaker table, Qt's SpeakersTable. Settings › Speakers shows it;
// Measure and Auto-tune show the layout's summary (LayoutSummary) and only their own tick boxes.
// Headings Output · Role · Speaker · Measured together; a speaker's name sets the group (the same
// name joins its group, a new name gets its own), shown as the names measured together. "Choose
// which outputs are measured together" is the override (on by itself while the groups are not the
// names'); off again puts the groups back to the names.

import { useState } from "preact/hooks";
import type { DspState } from "../../../core/protocol";
import { noticeRow } from "../../../ui/texts";
import { Pill } from "../../../ui/Pill";
import { outputColour } from "../../eq/model";
import { ROLE_WORDS, followNames, groupName, groupsFollowNames, pillOf, withRow, withSpeaker, withoutRow, type SpeakerLayoutOutputRecord, type SpeakerLayoutRecord } from "./model";
import "./speakers.css";

const ROLE_IDS = Object.keys(ROLE_WORDS) as SpeakerLayoutOutputRecord["role"][];

export interface SpeakerTableProps {
  s: DspState;
  /** The outputs shown, in order. */
  rows: number[];
  /** An output's name as the pages give it ("Left woofer", else "Out 1"). */
  label: (o: number) => string;
  draft: SpeakerLayoutRecord | null;
  stored: SpeakerLayoutRecord | null;
  /** The suggestion (an output taken in starts with its speaker). */
  proposal: SpeakerLayoutRecord | null;
  /** Why the layout may not be changed here, or "". */
  readOnly: string;
  onDraft: (next: SpeakerLayoutRecord | null) => void;
}

export function SpeakerTable(props: SpeakerTableProps) {
  const { s, rows, label, draft, stored, proposal, readOnly, onDraft } = props;
  const ro = !!readOnly;
  const [asked, setAsked] = useState(false);
  const byHand = asked || !groupsFollowNames(draft);
  const change = (o: number, patch: Partial<Omit<SpeakerLayoutOutputRecord, "output">>) => onDraft(withRow(draft, o, patch));
  const use = (o: number, on: boolean) => {
    if (!on) return onDraft(draft ? withoutRow(draft, o) : null);
    onDraft(withSpeaker(withRow(draft, o, {}), o, proposal?.outputs.find((x) => x.output === o)?.speaker ?? ""));
  };
  const nameOf = (o: number) => groupName(draft, o, (x) => `OUT${x + 1}`);
  // The groups to pick from: each group of the layout by its names, in output order, then one of its own
  const choices: { group: number; text: string }[] = [];
  for (const r of draft?.outputs ?? []) if (!choices.some((c) => c.group === r.group)) choices.push({ group: r.group, text: nameOf(r.output) });
  let free = 0;
  while (choices.some((c) => c.group === free) && free < 15) free++;

  return (
    <div class="speaker-table">
      <div class="speaker-table-head" aria-hidden="true">
        <span class="section-label">Output</span>
        <span class="section-label">Role</span>
        <span class="section-label">Speaker</span>
        <span class="section-label">Measured together</span>
      </div>
      <ul class="speaker-table-rows" aria-label="Speaker layout">
        {rows.map((o) => {
          // -- WP FIN015 -- (REVF D4) the channel pill and the label once; the controls' names say both
          const name = `OUT${o + 1} ${label(o)}`;
          const r = draft?.outputs.find((x) => x.output === o);
          const off = s.outputEnabled?.[o] === false;
          const pill = pillOf(draft, stored, o); // -- WP ADETW --
          const together = r ? nameOf(o) : "";
          return (
            <li key={o} aria-label={name} class="speaker-table-row">
              <span class="speaker-table-who">
                <label class="speakers-who">
                  <input type="checkbox" aria-label={`${name} is in the layout`} disabled={ro} checked={!!r} onChange={(e) => use(o, (e.currentTarget as HTMLInputElement).checked)} />
                  <Pill text={`OUT${o + 1}`} colour={outputColour(s, o)} />
                  <span class="speakers-name">{label(o)}</span>
                  {off ? <span class="note">(off)</span> : null}
                </label>
                {pill ? (
                  <span class="speaker-pill" data-kind={pill.kind} aria-label={`${name}: ${pill.word}`} title={(pill.note && noticeRow(pill.note)?.reason) || pill.reason}>
                    {pill.word}
                  </span>
                ) : null}
              </span>
              <select aria-label={`Role of ${name}`} disabled={ro || !r} value={r?.role ?? "full"} onChange={(e) => change(o, { role: (e.currentTarget as HTMLSelectElement).value as SpeakerLayoutOutputRecord["role"] })}>
                {ROLE_IDS.map((id) => (
                  <option key={id} value={id}>
                    {ROLE_WORDS[id]}
                  </option>
                ))}
              </select>
              <input
                type="text"
                aria-label={`Speaker of ${name}`}
                placeholder="Not set"
                disabled={ro || !r}
                size={4}
                maxLength={64}
                value={r?.speaker ?? ""}
                onInput={(e) => onDraft(withSpeaker(draft, o, (e.currentTarget as HTMLInputElement).value))}
              />
              {byHand ? (
                <select
                  aria-label={`${name} is measured with`}
                  disabled={ro || !r}
                  value={r ? String(choices.some((c) => c.group === r.group) ? r.group : free) : ""}
                  onChange={(e) => change(o, { group: Number((e.currentTarget as HTMLSelectElement).value) })}
                >
                  {choices.map((c) => (
                    <option key={c.group} value={String(c.group)}>
                      {c.text}
                    </option>
                  ))}
                  <option value={String(free)}>On its own</option>
                </select>
              ) : (
                <span class="speaker-table-together" aria-label={r ? `${name} is measured with ${together}` : undefined}>
                  {together}
                </span>
              )}
            </li>
          );
        })}
      </ul>
      <p class="note">Outputs with the same speaker name are measured together in one run and aligned to each other.</p>
      <label class="speaker-table-byhand">
        <input
          type="checkbox"
          disabled={ro}
          checked={byHand}
          onChange={(e) => {
            const on = (e.currentTarget as HTMLInputElement).checked;
            setAsked(on);
            if (!on && draft && !groupsFollowNames(draft)) onDraft(followNames(draft));
          }}
        />{" "}
        Choose which outputs are measured together
      </label>
    </div>
  );
}
