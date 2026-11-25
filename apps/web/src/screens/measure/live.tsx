// WP P5WMS: the live view (PLAN-P5 T9, T5; P5LIVE's calls): one output's short sweeps or finite
// pink noise with the microphone RTA, the measured curve with the target and the predicted curve
// of a pending change; "cut this peak" at a point and "fit this range" over a span, each
// previewed and applied as one undo step (the session's Undo takes it back); the RTA's averaging,
// hold and compare. The core's live tick runs in the Worker; every curve is the core's.

import { useEffect, useRef, useState } from "preact/hooks";
import {
  measureCall,
  type MeasureCoupler,
  type MeasureLiveFrameRecord,
  type MeasureLiveMode,
  type MeasureLiveProposalRecord,
  type MeasurePlacement,
  type MeasureRole,
  type MeasureRtaAveraging,
} from "../../core/measure-protocol";
import { noticeRow } from "../../ui/texts";
import type { MeasureTabProps } from "./MeasureScreen";
import { MeasureGraph, type Series } from "./graph";
import { AVERAGING, COUPLERS, LIVE_PLACEMENT_FIRST, ROLES, SMOOTHING, highestPoint, hz, issueText, liveText, livePlaceOf, smoothingLabel } from "./model";
import { readTextFile } from "./setup";
import { plainNumber, readNumber } from "../../ui/number"; // -- WP WEB --
import { Glyph, StatusChip } from "../../ui/design"; // -- WP WEB4 (INTD: ui/) --

interface LiveDraft {
  output: number;
  role: MeasureRole;
  mode: MeasureLiveMode;
  /** WP P5FOLLOW: null until chosen (here or on the Setup tab): no default. */
  placement: MeasurePlacement | null;
  coupler: MeasureCoupler;
  smoothing: number;
  averaging: MeasureRtaAveraging;
}

const LIVE_PLACEMENTS: { id: MeasurePlacement; label: string }[] = [
  { id: "listeningPosition", label: "Listening position" },
  { id: "nearField", label: "Near-field" },
  { id: "headphones", label: "Headphones on a coupler" },
];

/** A number field's value, or null when it is not one. */
function num(s: string): number | null {
  // -- WP WEB -- "1,5 kHz" and the like, as Qt's fields (USR1); the unit Hz after it
  const t = plainNumber(s).toLowerCase();
  const v = readNumber(t.replace(/k?hz$/, ""));
  return v === null ? null : t.endsWith("khz") ? v * 1000 : v;
}

export function LiveTab(props: MeasureTabProps) {
  const { desk, outputs, outputName } = props;
  const [d, setD] = useState<LiveDraft>(() => ({ output: outputs[0] ?? 0, role: "full", mode: "sweep", placement: livePlaceOf(desk.draft), coupler: desk.draft?.coupler ?? "iec60318_4", smoothing: 6, averaging: "slow" }));
  const [frame, setFrame] = useState<MeasureLiveFrameRecord | null>(null);
  const [at, setAt] = useState("");
  const [span, setSpan] = useState<[string, string]>(["", ""]);
  const [proposal, setProposal] = useState<MeasureLiveProposalRecord | null>(null);
  const [said, setSaid] = useState("");
  const live = desk.live;
  const active = !!live?.active;
  const set = (p: Partial<LiveDraft>) => setD({ ...d, ...p });
  const alive = useRef(true);
  useEffect(
    () => () => {
      alive.current = false;
    },
    [],
  );

  const refresh = async () => {
    try {
      const f = await measureCall(desk.core, "measureLiveFrame", undefined);
      if (!alive.current) return;
      setFrame(f);
      // A proposal that changes nothing stays shown (its reason) until the next one.
      setProposal((p) => f.pending ?? (p && !p.changes.length ? p : null));
    } catch {
      // The next state brings the next frame.
    }
  };
  useEffect(() => {
    void refresh();
  }, [live?.seq, live?.pending, live?.active, live?.runs]);

  const start = async () => {
    // WP P5FOLLOW: never without the person's placement (the button is disabled then too).
    if (!d.placement) return;
    setSaid("");
    setProposal(null);
    await desk.startLive({
      output: d.output,
      role: d.role,
      mode: d.mode,
      placement: d.placement,
      coupler: d.placement === "headphones" ? d.coupler : null,
      micDocument: desk.draft?.micDocument ?? desk.micDocument,
      targetDocument: desk.targetDocument,
      smoothing: d.smoothing,
      averaging: d.averaging,
    });
  };
  const propose = async (p: Promise<MeasureLiveProposalRecord>) => {
    setSaid("");
    try {
      const r = await p;
      setProposal(r);
      await refresh();
    } catch (e) {
      desk.fail(e);
    }
  };
  const cut = () => {
    const f = num(at);
    if (f === null) return setSaid("Type the frequency of the peak, or click it on the graph.");
    void propose(measureCall(desk.core, "measureLiveCutPeak", { output: d.output, freqHz: f }));
  };
  const fit = () => {
    const a = num(span[0]);
    const b = num(span[1]);
    if (a === null || b === null || !(b > a)) return setSaid("Type the range (from below to), or drag across it on the graph.");
    void propose(measureCall(desk.core, "measureLiveFitRange", { output: d.output, f1Hz: a, f2Hz: b }));
  };
  const apply = async () => {
    if (!proposal) return;
    try {
      await measureCall(desk.core, "measureLiveApply", { proposalId: proposal.id });
      setSaid(`Applied: ${proposal.label}. Undo takes it back as one step.`);
      setProposal(null);
      await refresh();
    } catch (e) {
      desk.fail(e);
    }
  };
  const discard = async () => {
    try {
      await measureCall(desk.core, "measureLiveDiscard", undefined);
    } catch (e) {
      desk.fail(e);
    }
    setProposal(null);
    await refresh();
  };
  const hold = async (on: boolean) => {
    try {
      await measureCall(desk.core, "measureLiveHold", { on });
      await refresh();
    } catch (e) {
      desk.fail(e);
    }
  };
  const averaging = async (a: MeasureRtaAveraging) => {
    set({ averaging: a });
    if (!active || d.mode !== "pinkRta") return;
    try {
      await measureCall(desk.core, "measureLiveAveraging", { averaging: a });
    } catch (e) {
      desk.fail(e);
    }
  };
  const importTarget = async (input: HTMLInputElement) => {
    try {
      const f = await readTextFile(input);
      if (!f) return;
      const r = await measureCall(desk.core, "targetImport", {
        name: f.name.replace(/\.[^.]*$/, ""),
        text: f.text,
        use: d.placement === "headphones" ? "headphones" : "speakers",
        coupler: d.placement === "headphones" ? d.coupler : "",
      });
      desk.setDocs({ targetDocument: r.text, targetName: f.name });
    } catch (e) {
      desk.fail(e);
    }
  };

  const series: Series[] = [];
  const f = frame;
  const measured = f?.measured ?? null;
  const rta = f?.rta ?? null;
  if (d.mode === "pinkRta" && rta?.current) series.push({ label: "Microphone RTA", kind: "measured", curve: rta.current });
  else if (measured) series.push({ label: `${outputName(f!.output)} measured`, kind: "measured", curve: measured });
  if (rta?.held) series.push({ label: "Held", kind: "held", curve: rta.held });
  if (f?.target) series.push({ label: desk.targetName ? `Target (${desk.targetName})` : "Target", kind: "target", curve: f.target });
  const predicted = proposal?.predicted ?? f?.predicted ?? null;
  if (predicted) series.push({ label: "Predicted", kind: "predicted", curve: predicted });
  const spl = !!(measured?.spl || rta?.current?.spl);
  const issue = f?.issue ?? live?.issue ?? null;
  const targetField = useRef<HTMLInputElement>(null); // -- WP WEB4 --
  const issueShown = !!issue && !active && issue.key !== "measure.stopped";
  const mark = num(span[0]) !== null && num(span[1]) !== null ? { f1: num(span[0])!, f2: num(span[1])! } : num(at) !== null ? { f1: num(at)! } : null;

  return (
    // -- WP WEB4 -- DL2's Live: no heading and no card; Start Live (the one primary) with a status chip,
    // why it waits as a notice row, the choices in one row, the target with Import Target…, the graph
    // in its frame, Direct adjustments as a section
    <section class="measure-live" aria-label="Live response">
      <div class="m-line">
        {active ? (
          <button type="button" onClick={() => void desk.stopLive()}>
            Stop Live
          </button>
        ) : (
          <button type="button" class="primary" disabled={desk.running || !d.placement} onClick={() => void start()}>
            Start Live
          </button>
        )}
        <StatusChip kind={active ? "ok" : issueShown ? "warn" : "idle"}>
          <span aria-live="polite" data-seq={live?.seq ?? 0}>
            {liveText(live)}
          </span>
        </StatusChip>
      </div>
      {!active && !d.placement ? (
        <div class="notice-row notice-row-warning" role="status" aria-label="Live placement needed">
          <Glyph name="warning" class="notice-icon" />
          <span class="notice-text">{LIVE_PLACEMENT_FIRST}</span>
        </div>
      ) : null}
      {issue && !active ? (
        issue.key === "measure.stopped" ? (
          <p class="m-caption" data-key={issue.key}>
            {issueText(issue, (k) => noticeRow(k)?.title ?? null, outputName)}
          </p>
        ) : (
          <div class="notice-row notice-row-error" role="alert" data-key={issue.key}>
            <Glyph name="error" class="notice-icon" />
            <span class="notice-text">{issueText(issue, (k) => noticeRow(k)?.title ?? null, outputName)}</span>
          </div>
        )
      ) : null}
      <fieldset class="m-line m-choices" disabled={active || desk.running}>
        <select aria-label="Live output" title="Output" value={String(d.output)} onChange={(e) => set({ output: Number((e.currentTarget as HTMLSelectElement).value) })}>
          {outputs.map((o) => (
            <option key={o} value={o}>
              {outputName(o)}
            </option>
          ))}
        </select>
        <select aria-label="Live way" title="Way" value={d.role} onChange={(e) => set({ role: (e.currentTarget as HTMLSelectElement).value as MeasureRole })}>
          {ROLES.map((r) => (
            <option key={r.id} value={r.id}>
              {r.label}
            </option>
          ))}
        </select>
        <select aria-label="Live placement" title="Placement" value={d.placement ?? ""} onChange={(e) => set({ placement: (e.currentTarget as HTMLSelectElement).value as MeasurePlacement })}>
          {d.placement ? null : (
            <option value="" disabled>
              Choose the placement
            </option>
          )}
          {LIVE_PLACEMENTS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
            </option>
          ))}
        </select>
        {d.placement === "headphones" ? (
          <select aria-label="Live coupler" title="Coupler" value={d.coupler} onChange={(e) => set({ coupler: (e.currentTarget as HTMLSelectElement).value as MeasureCoupler })}>
            {COUPLERS.map((c) => (
              <option key={c.id} value={c.id}>
                {c.label}
              </option>
            ))}
          </select>
        ) : null}
        <div role="radiogroup" aria-label="Live mode" class="segments">
          <button type="button" role="radio" aria-checked={d.mode === "sweep"} aria-pressed={d.mode === "sweep"} onClick={() => set({ mode: "sweep" })}>
            Sweeps
          </button>
          <button type="button" role="radio" aria-checked={d.mode === "pinkRta"} aria-pressed={d.mode === "pinkRta"} onClick={() => set({ mode: "pinkRta" })}>
            Pink noise (RTA)
          </button>
        </div>
        <select aria-label="Smoothing" title="Smoothing" value={String(d.smoothing)} onChange={(e) => set({ smoothing: Number((e.currentTarget as HTMLSelectElement).value) })}>
          {SMOOTHING.map((n) => (
            <option key={n} value={n}>
              {smoothingLabel(n)}
            </option>
          ))}
        </select>
      </fieldset>
      <div class="m-line m-spread">
        <span class="m-caption">{desk.targetName ? `Target: ${desk.targetName}` : "Target: the placement's default (our own curve)"}</span>
        <span class="m-line">
          {desk.targetName ? (
            <button type="button" disabled={active || desk.running} onClick={() => desk.setDocs({ targetDocument: "", targetName: "" })}>
              Default target
            </button>
          ) : null}
          <button type="button" disabled={active || desk.running} onClick={() => targetField.current?.click()}>
            Import Target…
          </button>
        </span>
        <input ref={targetField} type="file" class="visually-hidden" tabIndex={-1} aria-label="Import a target file" accept=".txt,.csv,.frd,text/plain" onChange={(e) => void importTarget(e.currentTarget as HTMLInputElement)} />
      </div>
      {d.mode === "pinkRta" ? (
        <div class="m-line">
          <label class="m-inline">
            <span>Averaging</span>
            <select aria-label="Averaging" value={d.averaging} onChange={(e) => void averaging((e.currentTarget as HTMLSelectElement).value as MeasureRtaAveraging)}>
              {AVERAGING.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.label}
                </option>
              ))}
            </select>
          </label>
          <button type="button" disabled={!active || !rta?.current} onClick={() => void hold(true)}>
            Hold
          </button>
          <button type="button" disabled={!rta?.held} onClick={() => void hold(false)}>
            Clear hold
          </button>
          {rta ? <span class="m-caption">{rta.frames} frames averaged</span> : null}
        </div>
      ) : null}
      <MeasureGraph
        label="Live response"
        series={series}
        unit={spl ? "dB SPL" : "dB (relative)"}
        mark={mark}
        onPick={(fq) => {
          setAt(String(Math.round(fq)));
          setSpan(["", ""]);
        }}
        onSpan={(a, b) => {
          setSpan([String(Math.round(a)), String(Math.round(b))]);
          setAt("");
        }}
      />
      {rta?.compare ? <MeasureGraph label="Compared with the held spectrum" series={[{ label: "Now minus held", kind: "compare", curve: rta.compare }]} unit="dB" /> : null}

      <div role="group" aria-label="Direct adjustments" class="m-sec">
        <h4 class="section-label">Direct adjustments</h4>
        <div class="m-line">
          <label class="m-inline">
            <span>Peak at, Hz</span>
            <input type="text" class="m-num m-freq" inputMode="decimal" aria-label="Peak frequency" size={7} value={at} onInput={(e) => setAt((e.currentTarget as HTMLInputElement).value)} />
          </label>
          <button type="button" disabled={!active} onClick={() => setAt(String(Math.round(highestPoint(measured) ?? 1000)))} title="The highest trusted point from 100 Hz to 5 kHz">
            Find the Highest Peak
          </button>
          <button type="button" disabled={!active || !measured} onClick={cut}>
            Cut This Peak
          </button>
        </div>
        <div class="m-line">
          <label class="m-inline">
            <span>From, Hz</span>
            <input type="text" class="m-num m-freq" inputMode="decimal" aria-label="Range from" size={7} value={span[0]} onInput={(e) => setSpan([(e.currentTarget as HTMLInputElement).value, span[1]])} />
          </label>
          <label class="m-inline">
            <span>to</span>
            <input type="text" class="m-num m-freq" inputMode="decimal" aria-label="Range to" size={7} value={span[1]} onInput={(e) => setSpan([span[0], (e.currentTarget as HTMLInputElement).value])} />
          </label>
          <button type="button" disabled={!active || !measured} onClick={fit}>
            Fit This Range
          </button>
        </div>
        {/* -- WP WEB7 -- (REVE D7) the pending change in the shared notice row, not a page-local copy */}
        {proposal ? (
          <div class="notice-row notice-row-info m-question" role="group" aria-label="Pending change">
            {proposal.changes.length ? (
              <>
                <strong>{proposal.label}</strong>
                <ul class="change-list">
                  {proposal.changes.map((c) => (
                    <li key={c.band}>{c.words}</li>
                  ))}
                  {proposal.gainAfterDb !== proposal.gainBeforeDb ? (
                    <li>
                      Output gain {proposal.gainBeforeDb.toFixed(1)} → {proposal.gainAfterDb.toFixed(1)} dB (headroom)
                    </li>
                  ) : null}
                </ul>
                <p class="m-caption">
                  Error to the target: {proposal.rmsBeforeDb.toFixed(1)} dB RMS now, {proposal.rmsAfterDb.toFixed(1)} dB RMS predicted (dotted).
                </p>
                <div class="m-actions">
                  <button type="button" class="primary" onClick={() => void apply()}>
                    Apply
                  </button>
                  <button type="button" onClick={() => void discard()}>
                    Discard
                  </button>
                </div>
              </>
            ) : (
              <p data-key={proposal.issue?.key}>{proposal.issue ? issueText(proposal.issue, (k) => noticeRow(k)?.title ?? null, outputName) : "Nothing to change there."}</p>
            )}
          </div>
        ) : null}
        {said ? (
          <p class="m-caption" role="status">
            {said}
          </p>
        ) : null}
        {mark ? <span class="m-caption">{mark.f2 ? `Range ${hz(mark.f1)} to ${hz(mark.f2)}` : `At ${hz(mark.f1)}`}</span> : null}
      </div>
    </section>
  );
}
