// WP P5WMS: a run as it goes (PLAN-P5 sections 1.1 to 1.3, S3 to S5): the phase in words, its
// progress, the ramp's levels, the position to move to, why it stopped, the timing verdict. Stop
// is always here while it runs (and in the screen's bar above every tab).

import { noticeRow } from "../../ui/texts";
import type { MeasureTabProps } from "./MeasureScreen";
import { issueText, phaseText, progressPct, timingWords } from "./model";
import { Icon } from "./parts"; // -- WP WEB4 --

/** -- WP WEB4 -- `inTune`: inside the auto-tune, where "See the results" is not the page's one primary. */
export function RunPanel(props: MeasureTabProps & { inTune?: boolean }) {
  const { desk, outputName } = props;
  const st = desk.state;
  if (!st || st.phase === "idle" || desk.live?.active) return null;
  const pct = progressPct(st);
  const timing = timingWords(st.timing);
  const stopped = st.phase === "aborted" || st.phase === "failed";
  return (
    // -- WP WEB4 -- a section of the page (DL2): no card; Stop is secondary (the danger fill only in a question)
    <section class="m-sec measure-run measure-column" aria-label="Measurement run">
      <div class="ds-section-head">
        <h4 class="section-label">Run</h4>
        {st.active ? (
          <button type="button" onClick={() => void desk.stopAll()}>
            Stop measuring
          </button>
        ) : null}
      </div>
      <p aria-live="polite" data-phase={st.phase}>
        {phaseText(st, outputName)}
      </p>
      {st.active ? (
        <div role="progressbar" class="m-progress" aria-label="Progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}>
          <div style={{ width: `${pct}%` }} />
        </div>
      ) : null}
      {st.rampLevels.length ? (
        <p class="m-caption">
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
        <p class={`m-iconline ${timing.reliable ? "m-ok" : "m-bad"}`} aria-label="Timing" data-reliable={String(timing.reliable)}>
          <Icon name={timing.reliable ? "circle-check" : "warning"} class="m-line-icon" />
          <span>{timing.text}</span>
        </p>
      ) : null}
      {st.phase === "done" ? (
        <div>
          <button type="button" class={props.inTune ? undefined : "primary"} onClick={() => props.show("results")}>
            See the results
          </button>
        </div>
      ) : null}
    </section>
  );
}
