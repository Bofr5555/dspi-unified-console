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

const row = { display: "flex", flexWrap: "wrap", gap: "8px 16px", alignItems: "flex-end" } as const;
const field = { display: "flex", flexDirection: "column", gap: "2px", minWidth: 0 } as const;

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
  const v = Number(s.trim());
  return s.trim() && Number.isFinite(v) ? v : null;
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
  const mark = num(span[0]) !== null && num(span[1]) !== null ? { f1: num(span[0])!, f2: num(span[1])! } : num(at) !== null ? { f1: num(at)! } : null;

  return (
    <section class="card" aria-label="Live response" style={{ display: "flex", flexDirection: "column", gap: "10px" }}>
      <div class="card-head" style={{ display: "flex", flexWrap: "wrap", gap: "8px", alignItems: "center" }}>
        <strong style={{ flex: "1 1 auto" }}>Live response</strong>
        {active ? (
          <button type="button" class="danger" onClick={() => void desk.stopLive()}>
            Stop live
          </button>
        ) : (
          <button type="button" class="primary" disabled={desk.running || !d.placement} onClick={() => void start()}>
            Start live
          </button>
        )}
      </div>
      {!active && !d.placement ? (
        <p class="note" aria-label="Live placement needed">
          {LIVE_PLACEMENT_FIRST}
        </p>
      ) : null}
      <fieldset style={{ border: 0, padding: 0, margin: 0, ...row }} disabled={active || desk.running}>
        <label style={field}>
          <span>Output</span>
          <select aria-label="Live output" value={String(d.output)} onChange={(e) => set({ output: Number((e.currentTarget as HTMLSelectElement).value) })}>
            {outputs.map((o) => (
              <option key={o} value={o}>
                {outputName(o)}
              </option>
            ))}
          </select>
        </label>
        <label style={field}>
          <span>Way</span>
          <select aria-label="Live way" value={d.role} onChange={(e) => set({ role: (e.currentTarget as HTMLSelectElement).value as MeasureRole })}>
            {ROLES.map((r) => (
              <option key={r.id} value={r.id}>
                {r.label}
              </option>
            ))}
          </select>
        </label>
        <label style={field}>
          <span>Placement</span>
          <select aria-label="Live placement" value={d.placement ?? ""} onChange={(e) => set({ placement: (e.currentTarget as HTMLSelectElement).value as MeasurePlacement })}>
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
        </label>
        {d.placement === "headphones" ? (
          <label style={field}>
            <span>Coupler</span>
            <select aria-label="Live coupler" value={d.coupler} onChange={(e) => set({ coupler: (e.currentTarget as HTMLSelectElement).value as MeasureCoupler })}>
              {COUPLERS.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.label}
                </option>
              ))}
            </select>
          </label>
        ) : null}
        <div role="radiogroup" aria-label="Live mode" style={field}>
          <span>Plays</span>
          <span style={{ display: "flex", gap: "10px" }}>
            <label>
              <input type="radio" name="measure-live-mode" checked={d.mode === "sweep"} onChange={() => set({ mode: "sweep" })} /> Sweeps
            </label>
            <label>
              <input type="radio" name="measure-live-mode" checked={d.mode === "pinkRta"} onChange={() => set({ mode: "pinkRta" })} /> Pink noise (RTA)
            </label>
          </span>
        </div>
        <label style={field}>
          <span>Smoothing</span>
          <select aria-label="Smoothing" value={String(d.smoothing)} onChange={(e) => set({ smoothing: Number((e.currentTarget as HTMLSelectElement).value) })}>
            {SMOOTHING.map((n) => (
              <option key={n} value={n}>
                {smoothingLabel(n)}
              </option>
            ))}
          </select>
        </label>
        <label style={field}>
          <span>Target file (REW or CSV)</span>
          <input type="file" aria-label="Import a target file" accept=".txt,.csv,.frd,text/plain" onChange={(e) => void importTarget(e.currentTarget as HTMLInputElement)} />
        </label>
        <span class="muted" style={field}>
          {desk.targetName ? `Target: ${desk.targetName}` : "Target: the placement's default (our own curve)"}
          {desk.targetName ? (
            <button type="button" onClick={() => desk.setDocs({ targetDocument: "", targetName: "" })}>
              Default target
            </button>
          ) : null}
        </span>
      </fieldset>
      {d.mode === "pinkRta" ? (
        <div style={row}>
          <label style={field}>
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
          {rta ? <span class="muted">{rta.frames} frames averaged</span> : null}
        </div>
      ) : null}
      <p aria-live="polite" data-seq={live?.seq ?? 0}>
        {liveText(live)}
      </p>
      {issue && !active ? (
        <p class={issue.key === "measure.stopped" ? "muted" : "error"} data-key={issue.key}>
          {issueText(issue, (k) => noticeRow(k)?.title ?? null, outputName)}
        </p>
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

      <div role="group" aria-label="Direct adjustments" style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
        <div style={row}>
          <label style={field}>
            <span>Peak at, Hz</span>
            <input type="text" inputMode="decimal" aria-label="Peak frequency" size={7} value={at} onInput={(e) => setAt((e.currentTarget as HTMLInputElement).value)} />
          </label>
          <button type="button" disabled={!active} onClick={() => setAt(String(Math.round(highestPoint(measured) ?? 1000)))} title="The highest trusted point from 100 Hz to 5 kHz">
            Find the highest peak
          </button>
          <button type="button" disabled={!active || !measured} onClick={cut}>
            Cut this peak
          </button>
          <label style={field}>
            <span>From, Hz</span>
            <input type="text" inputMode="decimal" aria-label="Range from" size={7} value={span[0]} onInput={(e) => setSpan([(e.currentTarget as HTMLInputElement).value, span[1]])} />
          </label>
          <label style={field}>
            <span>To, Hz</span>
            <input type="text" inputMode="decimal" aria-label="Range to" size={7} value={span[1]} onInput={(e) => setSpan([span[0], (e.currentTarget as HTMLInputElement).value])} />
          </label>
          <button type="button" disabled={!active || !measured} onClick={fit}>
            Fit this range
          </button>
        </div>
        {proposal ? (
          <div class="note" role="group" aria-label="Pending change">
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
                <p class="muted">
                  Error to the target: {proposal.rmsBeforeDb.toFixed(1)} dB RMS now, {proposal.rmsAfterDb.toFixed(1)} dB RMS predicted (dotted).
                </p>
                <div style={row}>
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
          <p class="note" role="status">
            {said}
          </p>
        ) : null}
        {mark ? <span class="muted">{mark.f2 ? `Range ${hz(mark.f1)} to ${hz(mark.f2)}` : `At ${hz(mark.f1)}`}</span> : null}
      </div>
    </section>
  );
}
