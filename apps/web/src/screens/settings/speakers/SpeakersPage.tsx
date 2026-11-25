// WP ROLEW: Settings › Speakers (MPLAN section 2.2): which output drives what. Pre-filled from the
// stored layout, else from the core's proposal; Confirm keeps it as the person's own. The
// measurement and the tweeter rule take the roles from here. -- WP ROLES -- an agent sets them only
// from the person's words (through a desktop console), shown here as set by AI from your words.

import { useEffect, useState } from "preact/hooks";
import { useSessionState } from "../../../ui/components";
import type { Session } from "../../../ui/session";
import { outputLabel } from "../../main/model";
import { noticeRow } from "../../../ui/texts";
import { CLASS_WORDS, ROLE_WORDS, TWEETER_RULE, bannerOf, baseOf, evidenceWords, pillOf, stampWords, withRow, withoutRow, type SpeakerLayoutOutputRecord, type SpeakerLayoutRecord } from "./model";
import { useSpeakerLayout } from "./use-layout";
import { Group } from "../../../ui/Group"; // -- WP WEB7 --
import { Icon } from "../../../ui/icons"; // -- WP WEB7 --

const ROLE_IDS = Object.keys(ROLE_WORDS) as SpeakerLayoutOutputRecord["role"][];

/** The rows as the page compares them: output, role, speaker, group. */
function same(a: SpeakerLayoutRecord | null, b: SpeakerLayoutRecord | null): boolean {
  const key = (l: SpeakerLayoutRecord | null) => JSON.stringify((l?.outputs ?? []).map((o) => [o.output, o.role, o.speaker.trim(), o.group]));
  return key(a) === key(b);
}

export function SpeakersPage(props: { session: Session }) {
  const { session } = props;
  const st = useSessionState(session);
  const { view, loaded, keep } = useSpeakerLayout(session);
  const [draft, setDraft] = useState<SpeakerLayoutRecord | null>(null);
  const [status, setStatus] = useState<{ error: boolean; text: string } | null>(null);
  const [showOff, setShowOff] = useState(false);
  useEffect(() => {
    setDraft(baseOf(view));
  }, [view]);
  const ro = session.readOnlyReason;
  const snap = st.snapshot;
  if (!snap) return null;
  const s = snap.state;
  const banner = loaded ? bannerOf(view) : null;
  const edited = !same(draft, view.stored);
  const confirmed = !!view.stored?.confirmedByUser && !edited;
  // -- WP ADETW -- the last detection's summary and evidence, under the rows
  const detection = view.stored?.detected ?? null;
  const change = (o: number, patch: Partial<Omit<SpeakerLayoutOutputRecord, "output">>) => {
    setDraft(withRow(draft, o, patch));
    setStatus(null);
  };
  const use = (o: number, on: boolean) => {
    setDraft(on ? withRow(draft, o, { speaker: view.proposal?.outputs.find((x) => x.output === o)?.speaker ?? "" }) : draft ? withoutRow(draft, o) : null);
    setStatus(null);
  };
  const confirm = async () => {
    if (!draft) return;
    const why = await keep(draft, true);
    setStatus(why ? { error: true, text: `Not confirmed: ${why}` } : { error: false, text: "Confirmed: the measurement takes its roles from this layout." });
  };
  const suggestion = () => {
    setDraft(view.proposal);
    setStatus(null);
  };
  // The outputs that are off are left out unless the layout has them or the person asks.
  const all = Array.from({ length: s.numOutputChannels }, (_, o) => o);
  const rows = all.filter((o) => showOff || s.outputEnabled?.[o] !== false || !!draft?.outputs.some((x) => x.output === o));
  const hidden = all.length - rows.length;

  return (
    // -- WP WEB7 -- Qt's SpeakersPage: one group, "Speaker layout", its icon and title above the panel
    <section class="speakers-page" aria-label="Speakers">
      <Group title="Speaker layout" icon={<Icon name="volume-2" />}>
      <p class="st-note">Which output drives what? The measurement and its safety checks take each output's role from here.</p>
      {banner ? (
        <div class="banner" role="status" aria-label="Speaker layout" data-kind={banner.kind} style={{ flexWrap: "wrap", margin: 0 }}>
          <span style={{ flex: "1 1 14em", minWidth: 0 }}>{banner.here}</span>
        </div>
      ) : null}
      {view.problem ? (
        <p class="error" role="alert" aria-label="Speaker layout problem">
          {view.problem}
        </p>
      ) : null}
      {ro ? <p class="note">{ro}: the speaker layout is shown, not changed.</p> : null}
      <ul class="speakers-rows" aria-label="Speaker layout" style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: "10px" }}>
        {rows.map((o) => {
          const name = `OUT${o + 1} ${outputLabel(s, snap.outputLabels, o)}`;
          const r = draft?.outputs.find((x) => x.output === o);
          const off = s.outputEnabled?.[o] === false;
          const pill = pillOf(draft, view.stored, o); // -- WP ADETW --
          return (
            <li key={o} aria-label={name} style={{ display: "flex", flexWrap: "wrap", gap: "4px 8px", alignItems: "center" }}>
              <label style={{ flex: "1 1 12em", minWidth: 0 }}>
                <input type="checkbox" aria-label={`${name} is in the layout`} disabled={!!ro} checked={!!r} onChange={(e) => use(o, (e.currentTarget as HTMLInputElement).checked)} /> {name}
                {off ? <span class="note"> (off)</span> : null}
              </label>
              {pill ? (
                <span class="speaker-pill" data-kind={pill.kind} aria-label={`${name}: ${pill.word}`} title={(pill.note && noticeRow(pill.note)?.reason) || pill.reason}>
                  {pill.word}
                </span>
              ) : null}
              <select aria-label={`Role of ${name}`} disabled={!!ro || !r} value={r?.role ?? "full"} onChange={(e) => change(o, { role: (e.currentTarget as HTMLSelectElement).value as SpeakerLayoutOutputRecord["role"] })}>
                {ROLE_IDS.map((id) => (
                  <option key={id} value={id}>
                    {ROLE_WORDS[id]}
                  </option>
                ))}
              </select>
              <label class="note">
                Speaker{" "}
                <input type="text" aria-label={`Speaker of ${name}`} disabled={!!ro || !r} size={4} maxLength={64} value={r?.speaker ?? ""} onInput={(e) => change(o, { speaker: (e.currentTarget as HTMLInputElement).value })} />
              </label>
              <label class="note" title="The outputs of one speaker share a group: they are measured in one run and aligned to each other">
                Group{" "}
                <input type="number" aria-label={`Group of ${name}`} disabled={!!ro || !r} min={0} max={15} step={1} style={{ width: "4em" }} value={r?.group ?? 0} onInput={(e) => change(o, { group: Math.max(0, Math.min(15, Math.trunc(Number((e.currentTarget as HTMLInputElement).value) || 0))) })} />
              </label>
            </li>
          );
        })}
      </ul>
      {hidden ? (
        <button type="button" onClick={() => setShowOff(true)} style={{ alignSelf: "flex-start" }}>
          Show the outputs that are off ({hidden})
        </button>
      ) : null}
      {view.proposal?.notes.length && !view.stored ? (
        <ul class="note" aria-label="About the suggestion">
          {view.proposal.notes.map((n, i) => (
            <li key={i}>{n}</li>
          ))}
        </ul>
      ) : null}
      {detection ? (
        <details class="note speakers-detection">
          <summary aria-label="Speaker detection">{detection.summary || "The speaker detection's result."}</summary>
          <table aria-label="Speaker detection details">
            <thead>
              <tr>
                <th>Output</th>
                <th>Found</th>
                <th>Heard</th>
                <th>Top fall</th>
                <th>−6 dB at</th>
                <th>Lowest played</th>
              </tr>
            </thead>
            <tbody>
              {detection.outputs.map((d) => {
                const e = evidenceWords(d);
                return (
                  <tr key={d.output}>
                    <td>OUT{d.output + 1}</td>
                    <td>
                      {CLASS_WORDS[d.class]}
                      {d.confidence !== "high" && !["nothing", "unknown", "off"].includes(d.class) ? "?" : ""}
                    </td>
                    <td>{e.heard}</td>
                    <td>{e.topFall}</td>
                    <td>{e.fLow}</td>
                    <td>{e.lowest}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </details>
      ) : null}
      <p class="note">{TWEETER_RULE}</p>
      <div style={{ display: "flex", flexWrap: "wrap", gap: "8px", alignItems: "center" }}>
        <button type="button" class="primary" disabled={!!ro || !loaded || !draft || !draft.outputs.length || confirmed} onClick={() => void confirm()}>
          Confirm
        </button>
        <button type="button" disabled={!!ro || !view.proposal || same(draft, view.proposal)} onClick={suggestion}>
          Use the suggestion
        </button>
        <span class="note" role="status" aria-label="Speaker layout state">
          {/* -- WP ADETW -- every kept layout says how far it is trusted: confirmed, detected, or kept */}
          {edited && view.stored ? "Changed: not kept until you confirm." : !edited ? stampWords(view.stored) : ""}
        </span>
      </div>
      {status ? (
        <p class={status.error ? "error" : "note"} role="status" aria-label="Speaker layout saved">
          {status.text}
        </p>
      ) : null}
      </Group>
    </section>
  );
}
