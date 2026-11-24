// WP P5WAT: the Measure screen's auto-tune (PLAN-P5 section 1.4, T10, R5): the layout with its
// alignment groups and programme feeds, the measurement of it, the proposal with every write in
// words (what it was held to, what is withdrawn and why), the mandatory summed check, the guarded
// apply with its countdown and the verify that runs by itself after it, Keep (with the second
// question after a verify that did not pass) or Revert, and the A/B compare of before and after.
// Every rule and every number is the core's (tune-model.ts says which call when). Also the tabs
// this WP adds to the screen's slot (`WAT_TABS`).

import { useEffect, useState } from "preact/hooks";
import { noticeRow } from "../../ui/texts";
import type { MeasureTab, MeasureTabProps } from "./MeasureScreen";
import { DocsTab } from "./docs";
import { MeasureGraph } from "./graph";
import { COUPLERS, ROLES } from "./model";
import { RunPanel } from "./run";
import { download } from "./setup";
import { Glyph } from "../../ui/design"; // -- WP WEB4 (INTD: ui/) --
import { StepDots } from "./parts"; // -- WP WEB4 --
import {
  CONFIRM_MS,
  checkWords,
  feedWords,
  guardSeconds,
  issueWords,
  layoutSetup,
  proposalSeries,
  stageWords,
  templateWords,
  transitionWords,
  tuneDeskOf,
  verifyLines,
  type TuneDesk,
} from "./tune-model";

const row = { display: "flex", flexWrap: "wrap", gap: "8px 14px", alignItems: "center" } as const;
const col = { display: "flex", flexDirection: "column", gap: "8px", minWidth: 0 } as const;

/** Re-render on the desks' changes, and once a second while a countdown or the second question shows. */
function useTune(t: TuneDesk, props: MeasureTabProps): void {
  const [, bump] = useState(0);
  useEffect(() => {
    const a = t.watch(() => bump((n) => n + 1));
    const b = props.desk.watch(() => bump((n) => n + 1));
    return () => {
      a();
      b();
    };
  }, [t]);
  const ticking = t.applied || !!t.question;
  useEffect(() => {
    if (!ticking) return;
    const id = setInterval(() => bump((n) => n + 1), 250);
    return () => clearInterval(id);
  }, [ticking]);
}

export function TuneTab(props: MeasureTabProps) {
  const { desk, outputName } = props;
  const t = tuneDeskOf(desk);
  useTune(t, props);
  useEffect(() => {
    void t.enter();
    return () => void t.leave();
  }, [t]);
  const st = t.state;
  const p = t.proposal;
  const src = t.source;
  const l = t.layout;
  const measuring = !!desk.state?.active;
  const busy = !!t.busy || t.running || measuring;

  const place = t.place;
  const measure = async () => {
    // WP P5FOLLOW: never without the person's placement (the button is disabled then too).
    if (!l || !place) return;
    t.useMeasurement(null);
    const on = { ...l, outputs: l.outputs.filter((o) => !t.off.has(o.output)) };
    await desk.startRun(layoutSetup(on, t.sweepMs, desk.micDocument || desk.draft?.micDocument || "", "Auto-tune layout", place));
  };

  // -- WP WEB4 -- DL2's Auto-tune: step dots over sections without numbers, the stop bar as an info notice
  const step = st && (t.applied || st.stage === "kept" || st.stage === "reverted" || st.stage === "revertIncomplete") ? 2 : p ? 1 : 0;
  return (
    <div class="measure-tune measure-column">
      <StepDots steps={["Layout", "Proposal", "Applied"]} current={step} label="Auto-tune steps" />
      {t.running && !desk.running ? (
        <div class="notice-row notice-row-info measure-stopbar" role="status" aria-label="Tuning">
          <Glyph name="volume" class="notice-icon" />
          <span class="notice-text">{stageWords(st)}</span>
          <button type="button" onClick={() => void t.stop()}>
            Stop
          </button>
        </div>
      ) : null}
      {t.error ? (
        <p class="error" role="alert" aria-label="Tune problem">
          {t.error}
        </p>
      ) : null}

      <section class="m-sec" aria-label="Tune layout" style={col}>
        <div class="ds-section-head">
          <h4 class="section-label">Layout</h4>
          <button type="button" disabled={busy || t.applied} onClick={() => void t.loadLayout()}>
            Read the layout again
          </button>
        </div>
        {l ? (
          <>
            <p class="m-caption" aria-label="Layout recognised">
              {templateWords(l.template)} Each output's way, speaker and alignment group come from its crossover and its programme input; correct them here. One walked run measures each group.
            </p>
            {l.notes.length ? (
              <ul class="change-list notes">
                {l.notes.map((n, i) => (
                  <li key={i}>{n}</li>
                ))}
              </ul>
            ) : null}
            <ul class="tune-layout m-rowlist" aria-label="Outputs">
              {l.outputs.map((o) => (
                <li key={o.output} data-output={o.output}>
                  <label class="m-check" style={{ minWidth: "9em" }}>
                    <input type="checkbox" aria-label={`Tune ${outputName(o.output)}`} checked={!t.off.has(o.output)} disabled={busy || t.applied} onChange={(e) => t.setOn(o.output, (e.currentTarget as HTMLInputElement).checked)} />{" "}
                    {outputName(o.output)}
                  </label>
                  <select aria-label={`Way of ${outputName(o.output)}`} value={o.role} disabled={busy || t.applied} onChange={(e) => t.editOutput(o.output, { role: (e.currentTarget as HTMLSelectElement).value as typeof o.role })}>
                    {ROLES.map((r) => (
                      <option key={r.id} value={r.id}>
                        {r.label}
                      </option>
                    ))}
                  </select>
                  <label class="m-inline">
                    Speaker{" "}
                    <input class="m-speaker" aria-label={`Speaker of ${outputName(o.output)}`} value={o.speaker} size={4} maxLength={16} disabled={busy || t.applied} onChange={(e) => t.editOutput(o.output, { speaker: (e.currentTarget as HTMLInputElement).value })} />
                  </label>
                  <label class="m-inline">
                    Group{" "}
                    <input
                      class="m-num"
                      aria-label={`Alignment group of ${outputName(o.output)}`}
                      type="number"
                      min={0}
                      max={15}
                      value={o.group}
                      disabled={busy || t.applied}
                      onChange={(e) => {
                        const g = Number((e.currentTarget as HTMLInputElement).value);
                        if (Number.isInteger(g) && g >= 0 && g <= 15) t.editOutput(o.output, { group: g });
                      }}
                    />
                  </label>
                  <span class="m-caption">{feedWords(o)}</span>
                </li>
              ))}
            </ul>
          </>
        ) : (
          <p class="empty">Reading the layout…</p>
        )}
      </section>

      <section class="m-sec" aria-label="Tune measurement" style={col}>
        <h4 class="section-label">Measurement</h4>
        {src ? (
          <p class="m-iconline m-ok" aria-label="Measurement to tune from" data-source="yes">
            <Glyph name="circle-check" class="m-line-icon" />
            <span>{`Tuning from ${src.from ? `${src.from}` : "the measurement"}.`}</span>
          </p>
        ) : (
          <div class="notice-row notice-row-info" role="status" aria-label="Measurement to tune from" data-source="none">
            <Glyph name="info" class="notice-icon" />
            <span class="notice-text">No measurement yet: measure the layout here (the preflight comes first), or open one on the Documents tab.</span>
          </div>
        )}
        <div style={row}>
          <label class="m-inline">
            Sweep length{" "}
            <input class="m-num" aria-label="Tune sweep length" type="number" min={1} max={15} step={0.5} value={t.sweepMs / 1000} disabled={busy} onChange={(e) => t.setSweepMs(Number((e.currentTarget as HTMLInputElement).value) * 1000)} />
          </label>
          <label class="m-inline">
            Measured{" "}
            <select aria-label="Tune placement" value={place?.placement ?? ""} disabled={busy || t.applied} onChange={(e) => t.setHeadphones((e.currentTarget as HTMLSelectElement).value === "headphones")}>
              {place ? null : (
                <option value="" disabled>
                  Choose where it is measured
                </option>
              )}
              <option value="listeningPosition">Speakers, at the listening position</option>
              <option value="headphones">Headphones on a coupler</option>
            </select>
          </label>
          <button type="button" disabled={busy || t.applied || !l || !place || props.blocked !== ""} onClick={() => void measure()}>
            Measure this layout
          </button>
        </div>
        <p class={place ? "m-caption" : "m-iconline m-note"} aria-label="Tune placement levels">
          {!place
            ? t.placeProblem
            : place.placement === "headphones"
              ? `Headphones: on the coupler, not on a head. The levels are the lower ones for headphones (${COUPLERS.find((c) => c.id === place.coupler)?.label ?? "the Setup tab's coupler"}).`
              : "Speakers: the levels are those for speakers. For headphones choose Headphones on a coupler first: they are measured much more quietly."}
        </p>
        {desk.preflight && !desk.preflight.clear ? (
          <ul class="change-list" aria-label="Tune preflight">
            {desk.preflight.blockers.map((b, i) => (
              <li key={i} data-key={b.key}>
                {issueWords(b)}
              </li>
            ))}
          </ul>
        ) : null}
        <RunPanel {...props} inTune />
      </section>

      <section class="m-sec" aria-label="Tune options" style={col}>
        <h4 class="section-label">Proposal</h4>
        <div role="group" aria-label="What the tune may change" class="m-checks">
          <label>
            <input type="checkbox" checked={t.options.keepCrossover} disabled={busy || t.applied} onChange={(e) => t.setOptions({ keepCrossover: (e.currentTarget as HTMLInputElement).checked })} /> Keep the crossover, align only
          </label>
          <label>
            <input type="checkbox" checked={t.options.keepDelays} disabled={busy || t.applied} onChange={(e) => t.setOptions({ keepDelays: (e.currentTarget as HTMLInputElement).checked })} /> Keep the delays
          </label>
          <label>
            <input type="checkbox" checked={t.options.keepLevels} disabled={busy || t.applied} onChange={(e) => t.setOptions({ keepLevels: (e.currentTarget as HTMLInputElement).checked })} /> Keep the levels
          </label>
          <label>
            <input type="checkbox" checked={t.options.keepEq} disabled={busy || t.applied} onChange={(e) => t.setOptions({ keepEq: (e.currentTarget as HTMLInputElement).checked })} /> No EQ fit
          </label>
        </div>
        <p class="m-caption">Target: {desk.targetName ? desk.targetName : "the placement's default (our own curve)"}. Choose another on the Documents tab.</p>
        <div>
          <button type="button" class={src && !p ? "primary" : undefined} disabled={busy || t.applied || !src || !l} onClick={() => void t.propose()}>
            Propose a Tune
          </button>
        </div>
      </section>

      {p ? <ProposalCard {...props} t={t} /> : null}
      {st && (t.applied || st.stage === "kept" || st.stage === "reverted" || st.stage === "revertIncomplete") ? <GuardCard {...props} t={t} /> : null /* -- WP REVERTW -- */}
      {t.said ? (
        <p class="m-iconline m-note" role="status" aria-label="Tune outcome">
          {t.said}
        </p>
      ) : null}
    </div>
  );
}

function ProposalCard(props: MeasureTabProps & { t: TuneDesk }) {
  const { t, outputName } = props;
  const p = t.proposal!;
  const busy = !!t.busy || t.running || !!props.desk.state?.active;
  const pending = p.sumCheck === "pending";
  const series = proposalSeries(p);
  return (
    <section class="m-sec" aria-label="Tune proposal" style={col} data-proposal={p.id}>
      <h4 class="section-label">The proposal{p.measurementName ? `: ${p.measurementName}` : ""}</h4>
      <p class={`m-iconline ${p.timingReliable ? "m-ok" : "m-bad"}`} aria-label="Proposal timing" data-reliable={String(p.timingReliable)}>
        <Glyph name={p.timingReliable ? "circle-check" : "warning"} class="m-line-icon" />
        <span>{p.timingReliable ? "The measurement's timing is reliable: delay, polarity and crossover changes may be offered." : "The measurement's timing is unreliable: no delay, polarity or crossover change is offered from it."}</span>
      </p>
      {series.length ? <MeasureGraph label="Tune curves" series={series} unit={series.some((s) => s.curve.spl) ? "dB SPL" : "dB (relative)"} /> : null}
      <div>
        <strong>Changes</strong> <span class="m-caption">(nothing is written until Apply)</span>
        {p.writes.length ? (
          <ul class="change-list" aria-label="Changes">
            {p.writes.map((w, i) => (
              <li key={i} data-kind={w.kind} data-timing={String(w.timing)}>
                {w.words}
              </li>
            ))}
          </ul>
        ) : (
          <p class="m-caption" aria-label="Changes">
            No change: the speaker is as close to its target as the tune can bring it.
          </p>
        )}
      </div>
      {p.transitions.length ? (
        <ul class="change-list" aria-label="Transitions">
          {p.transitions.map((x, i) => (
            <li key={i}>{transitionWords(x, outputName)}</li>
          ))}
        </ul>
      ) : null}
      {p.held.length ? (
        <div>
          <strong>Held to</strong>
          <ul class="change-list" aria-label="Held to">
            {p.held.map((h, i) => (
              <li key={i}>{h}</li>
            ))}
          </ul>
        </div>
      ) : null}
      {p.withdrawn.length ? (
        <div>
          <strong>Not offered</strong>
          <ul class="change-list" aria-label="Withdrawn">
            {p.withdrawn.map((w, i) => (
              <li key={i} data-key={w.key}>
                {issueWords(w)}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {p.warnings.length ? (
        <ul class="change-list notes" aria-label="Warnings">
          {p.warnings.map((w, i) => (
            <li key={i} data-key={w.key}>
              {issueWords(w)}
            </li>
          ))}
        </ul>
      ) : null}
      {p.notes.length ? (
        <ul class="change-list notes" aria-label="Notes">
          {p.notes.map((n, i) => (
            <li key={i}>{n}</li>
          ))}
        </ul>
      ) : null}
      <p aria-label="Summed check" data-check={p.sumCheck} class={`m-iconline ${p.sumCheck === "failed" ? "m-bad" : p.sumCheck === "passed" ? "m-ok" : "m-note"}`}>
        <Glyph name={p.sumCheck === "failed" ? "error" : p.sumCheck === "passed" ? "circle-check" : "info"} class="m-line-icon" />
        <span>{checkWords(p.sumCheck)}</span>
      </p>
      <div style={row}>
        {pending ? (
          <button type="button" class="primary" disabled={busy || t.applied || props.blocked !== ""} onClick={() => void t.sumCheck()}>
            Run the summed check
          </button>
        ) : null}
        <button type="button" class={pending ? "" : "primary"} disabled={busy || t.applied || pending || !p.writes.length || props.blocked !== ""} onClick={() => void t.apply()}>
          Apply and verify
        </button>
      </div>
      <p class="m-caption">
        Apply files a "before" snapshot, writes every change as one undo step inside a guard, then verifies at once: Keep or Revert before the guard ends, or the settings from before come back.
      </p>
    </section>
  );
}

function GuardCard(props: MeasureTabProps & { t: TuneDesk }) {
  const { t, outputName } = props;
  const st = t.state!;
  const g = st.guard;
  const left = guardSeconds(g);
  const v = st.verify;
  const q = t.question;
  const ready = t.confirmReady();
  const wait = q ? Math.max(0, Math.ceil((q.since + CONFIRM_MS - Date.now()) / 1000)) : 0;
  const kept = t.kept;
  const row2 = noticeRow("measure.tune.keep_unverified");
  return (
    <section class="m-sec" aria-label="Tune guard" style={col} data-stage={st.stage}>
      <h4 class="section-label">Verify, then Keep or Revert</h4>
      {/* -- WP REVERTW -- a revert not fully successful is an alert, not a status line */}
      <p aria-label="Tune stage" aria-live="polite" role={st.stage === "revertIncomplete" ? "alert" : undefined} class={st.stage === "revertIncomplete" ? "error" : undefined}>
        {stageWords(st)}
      </p>
      {t.applied && g.phase === "waiting" ? (
        <>
          <p aria-label="Guard countdown" data-left={left}>
            {left} s left to Keep or Revert; then the settings from before come back by themselves.
          </p>
          <progress max={1} value={g.timeoutMs > 0 ? Math.max(0, Math.min(1, g.remainingMs / g.timeoutMs)) : 0} aria-label="Time left" />
        </>
      ) : null}
      {st.running ? (
        <div role="progressbar" aria-label="Verify progress" aria-valuemin={0} aria-valuemax={st.runs || 1} aria-valuenow={st.run}>
          <span class="m-caption">
            Run {Math.max(1, st.run)} of {Math.max(1, st.runs)}
          </span>
        </div>
      ) : null}
      {v && st.stage === "verified" ? (
        <div aria-label="Verify" data-passed={String(v.passed)} class={`m-verify ${v.passed ? "m-ok" : "m-bad"}`}>
          <strong>{v.passed ? "The verify confirmed the tune." : "The verify did not confirm the tune."}</strong>
          <ul class="change-list">
            {verifyLines(v, outputName).map((x, i) => (
              <li key={i}>{x}</li>
            ))}
          </ul>
        </div>
      ) : null}
      {st.issue && !q && !st.running && st.stage !== "kept" ? (
        <p class={st.stage === "revertIncomplete" ? "error" : "m-iconline m-note"} data-key={st.issue.key}>
          {issueWords(st.issue)}
        </p>
      ) : null}
      {q ? (
        <div role="alertdialog" aria-label="Keep anyway?" class="notice-row notice-row-warning m-question" style={col}>
          <strong>{q.why === "unverified" ? (row2?.title ?? "The verify did not confirm the tune") : "The tune is not verified"}</strong>
          <span>{q.text}</span>
          {q.why === "unverified" && row2 ? <span class="m-caption">{`${row2.reason} ${row2.action}`}</span> : null}
          <div style={row}>
            <button type="button" class="danger" disabled={!ready || !!t.busy} onClick={() => void t.confirmKeep()}>
              {ready ? "Keep anyway" : `Keep anyway (in ${wait} s)`}
            </button>
            <button type="button" onClick={() => void t.revert()}>
              Revert
            </button>
            <button type="button" onClick={() => t.cancelKeep()}>
              Not now
            </button>
          </div>
        </div>
      ) : null}
      <div style={row}>
        {t.keepOffered && !q ? (
          <button type="button" class="primary" disabled={!!t.busy} onClick={() => void t.keep()}>
            Keep
          </button>
        ) : null}
        {t.applied ? (
          <button type="button" onClick={() => void t.revert()}>
            Revert
          </button>
        ) : null}
        {t.applied && !t.running && g.phase === "waiting" ? (
          <button type="button" disabled={!!t.busy || props.blocked !== ""} onClick={() => void t.verify()}>
            Verify again
          </button>
        ) : null}
        {t.running ? (
          <button type="button" onClick={() => void t.stop()}>
            Stop the verify
          </button>
        ) : null}
      </div>
      {st.stage === "kept" && kept ? (
        <div style={col}>
          {kept.notes.length ? (
            <ul class="change-list notes">
              {kept.notes.map((n, i) => (
                <li key={i}>{n}</li>
              ))}
            </ul>
          ) : null}
          <div style={row}>
            {kept.snapshotBefore ? (
              <button type="button" onClick={() => download(kept.snapshotBefore, "Before auto-tune.dspipreset")}>
                Download "before"
              </button>
            ) : null}
            {kept.snapshotAfter ? (
              <button type="button" onClick={() => download(kept.snapshotAfter, "After auto-tune.dspipreset")}>
                Download "after"
              </button>
            ) : null}
            {!t.ab ? (
              <button type="button" onClick={() => void t.abStart()}>
                Compare A/B
              </button>
            ) : null}
          </div>
        </div>
      ) : null}
      {t.ab?.active ? (
        <div role="group" aria-label="Tune A/B" style={row} data-side={t.ab.side ?? ""}>
          <span>
            Now {t.ab.side === "b" ? "B: before the tune" : "A: the tune"}
          </span>
          <button type="button" onClick={() => void t.abToggle()}>
            Switch to {t.ab.side === "b" ? "A" : "B"}
          </button>
          <button type="button" onClick={() => void t.abEnd("a")}>
            End on the tune
          </button>
          <button type="button" onClick={() => void t.abEnd("b")}>
            End on before
          </button>
        </div>
      ) : null}
    </section>
  );
}

/** The tabs this WP adds to the Measure screen's slot: the auto-tune, and the documents (a Viewer's too). */
export const WAT_TABS: MeasureTab[] = [
  { id: "tune", label: "Auto-tune", View: TuneTab },
  { id: "docs", label: "Documents", View: DocsTab, passive: true },
];
