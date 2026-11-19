// WP P5WMS: a run as it goes (PLAN-P5 sections 1.1 to 1.3, S3 to S5): the phase in words, its
// progress, the ramp's levels, the position to move to, why it stopped, the timing verdict. Stop
// is always here while it runs (and in the screen's bar above every tab).

import { noticeRow } from "../../ui/texts";
import type { MeasureTabProps } from "./MeasureScreen";
import { issueText, phaseText, progressPct, timingWords } from "./model";

export function RunPanel(props: MeasureTabProps) {
  const { desk, outputName } = props;
  const st = desk.state;
  if (!st || st.phase === "idle" || desk.live?.active) return null;
  const pct = progressPct(st);
  const timing = timingWords(st.timing);
  const stopped = st.phase === "aborted" || st.phase === "failed";
  return (
    <section class="card" aria-label="Measurement run" style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
      <div class="card-head" style={{ display: "flex", flexWrap: "wrap", gap: "8px", alignItems: "center" }}>
        <strong style={{ flex: "1 1 auto" }}>Run</strong>
        {st.active ? (
          <button type="button" class="danger" onClick={() => void desk.stopAll()}>
            Stop measuring
          </button>
        ) : null}
      </div>
      <p aria-live="polite" data-phase={st.phase}>
        {phaseText(st, outputName)}
      </p>
      {st.active ? (
        <div role="progressbar" aria-label="Progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct} style={{ height: "6px", background: "var(--line)", borderRadius: "3px" }}>
          <div style={{ width: `${pct}%`, height: "100%", background: "var(--accent)", borderRadius: "3px" }} />
        </div>
      ) : null}
      {st.rampLevels.length ? (
        <p class="muted">
          Levels found: {st.rampLevels.map((l) => `${outputName(l.output)} ${l.levelDb.toFixed(0)} dBFS`).join(", ")}
          {typeof st.runLevelDb === "number" ? `; the run plays at ${st.runLevelDb.toFixed(0)} dBFS (the quietest of them).` : "."}
        </p>
      ) : null}
      {st.phase === "waitPosition" ? (
        <div>
          <button type="button" class="primary" onClick={() => void desk.nextPosition()}>
            Next position
          </button>
        </div>
      ) : null}
      {st.issue && (stopped || st.phase === "waitPosition") ? (
        <p class={stopped ? "error" : "note"} role={stopped ? "alert" : undefined} data-key={st.issue.key}>
          {issueText(st.issue, (k) => noticeRow(k)?.title ?? null, outputName)}
          {noticeRow(st.issue.key)?.action ? ` ${noticeRow(st.issue.key)!.action}` : ""}
        </p>
      ) : null}
      {timing ? (
        <p class={timing.reliable ? "note" : "error"} aria-label="Timing" data-reliable={String(timing.reliable)}>
          {timing.text}
        </p>
      ) : null}
      {st.phase === "done" ? (
        <div>
          <button type="button" class="primary" onClick={() => props.show("results")}>
            See the results
          </button>
        </div>
      ) : null}
    </section>
  );
}
