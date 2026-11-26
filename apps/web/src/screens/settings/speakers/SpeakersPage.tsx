// WP ROLEW: Settings › Speakers (MPLAN section 2.2): which output drives what. Pre-filled from the
// stored layout, else from the core's proposal; Confirm keeps it as the person's own. The
// measurement and the tweeter rule take the roles from here. -- WP ROLES -- an agent sets them only
// from the person's words (through a desktop console), shown here as set by AI from your words.

import { useEffect, useState } from "preact/hooks";
import { useSessionState } from "../../../ui/components";
import type { Session } from "../../../ui/session";
import { outputLabel } from "../../main/model";
import { CLASS_WORDS, SUGGEST, TWEETER_RULE, bannerOf, baseOf, evidenceWords, stampWords, type SpeakerLayoutRecord } from "./model";
import { SpeakerTable } from "./SpeakerTable"; // -- WP UB7 --
import { useSpeakerLayout } from "./use-layout";
import { Group } from "../../../ui/Group"; // -- WP WEB7 --
import { Icon } from "../../../ui/icons"; // -- WP WEB7 --

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
      {/* -- WP UB7 -- (UXP B7) the one table, with its headings */}
      <SpeakerTable
        s={s}
        rows={rows}
        label={(o) => outputLabel(s, snap.outputLabels, o)}
        draft={draft}
        stored={view.stored}
        proposal={view.proposal}
        readOnly={ro ?? ""}
        onDraft={(next) => {
          setDraft(next);
          setStatus(null);
        }}
      />
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
          {SUGGEST /* -- WP UB7 -- */}
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
