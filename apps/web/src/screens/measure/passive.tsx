// WP P5WMS: the passive microphone RTA (PLAN-P5 section 1.5, T5; P5CURV's calls): what a Viewer or
// a read-only connection gets. The microphone's blocks go from the capture worklet to the Worker,
// which pushes them into `measureRtaPassivePush`; the core's frames (its RTA, the held copy and
// the compare) are drawn here. Nothing is played and nothing is written to the DSPi. It is offered
// only while no measurement or live view runs on this core, so it never shows the test signal.

import { useEffect, useState } from "preact/hooks";
import type { MeasureRtaAveraging } from "../../core/measure-protocol";
import type { MeasureTabProps } from "./MeasureScreen";
import { MeasureGraph, type Series } from "./graph";
import { AVERAGING } from "./model";
import { NoticeRow } from "../../ui/design"; // -- WP WEB --

export function PassiveTab(props: MeasureTabProps) {
  const { desk } = props;
  const [averaging, setAveraging] = useState<MeasureRtaAveraging>("slow");
  // The tab's own: closing it stops the RTA and gives the microphone back.
  useEffect(
    () => () => {
      void desk.stopPassive();
    },
    [desk],
  );
  const f = desk.passive;
  const rta = f?.rta ?? null;
  const series: Series[] = [];
  if (rta?.current) series.push({ label: "Microphone RTA", kind: "measured", curve: rta.current });
  if (rta?.held) series.push({ label: "Held", kind: "held", curve: rta.held });
  const blocker = desk.passiveBlocker;
  const mic = desk.mic;
  return (
    // -- WP WEB4 -- as DL2's Live: no card and no second title; Listen the one primary, Stop secondary
    <section class="measure-live" aria-label="Passive microphone RTA">
      <div class="m-line">
        {f ? (
          <button type="button" onClick={() => void desk.stopPassive()}>
            Stop listening
          </button>
        ) : (
          <button type="button" class="primary" disabled={!!blocker || desk.running} onClick={() => void desk.startPassive(averaging)}>
            Listen
          </button>
        )}
      </div>
      {props.blocked ? <NoticeRow kind="info">{props.blocked}</NoticeRow> : null}
      <p class="m-caption">The microphone listens to what is playing; nothing is played and nothing on the DSPi changes.</p>
      {/* -- WP WEB -- the condition that holds Listen back, as a notice row (DESIGN-LANGUAGE §14) */}
      {blocker ? <NoticeRow kind="warning">{blocker}</NoticeRow> : null}
      <div class="m-line">
        <label class="m-inline">
          <span>Microphone</span>
          <select
            aria-label="Microphone"
            disabled={!!f}
            value={mic.kind === "browser" ? `mic:${mic.deviceId}` : "mic:"}
            onChange={(e) => desk.setMic({ kind: "browser", deviceId: (e.currentTarget as HTMLSelectElement).value.slice(4) })}
          >
            <option value="mic:">The browser's default microphone</option>
            {desk.mics.map((m) => (
              <option key={m.deviceId} value={`mic:${m.deviceId}`}>
                {m.labelled ? m.label : `Microphone ${m.deviceId.slice(0, 6)} (named after the permission)`}
              </option>
            ))}
          </select>
        </label>
        <label class="m-inline">
          <span>Averaging</span>
          <select
            aria-label="Averaging"
            value={averaging}
            onChange={(e) => {
              const a = (e.currentTarget as HTMLSelectElement).value as MeasureRtaAveraging;
              setAveraging(a);
              void desk.passiveAveraging(a);
            }}
          >
            {AVERAGING.map((a) => (
              <option key={a.id} value={a.id}>
                {a.label}
              </option>
            ))}
          </select>
        </label>
        <button type="button" disabled={!rta?.current} onClick={() => void desk.passiveHold(true)}>
          Hold
        </button>
        <button type="button" disabled={!rta?.held} onClick={() => void desk.passiveHold(false)}>
          Clear hold
        </button>
        <span class="m-caption" aria-label="Frames averaged" data-frames={f?.state.frames ?? 0}>
          {f ? `${f.state.frames} frames averaged` : "Not listening."}
        </span>
      </div>
      <MeasureGraph label="Passive microphone RTA" series={series} unit={f?.state.spl ? "dB SPL" : "dBFS"} />
      {rta?.compare ? <MeasureGraph label="Compared with the held spectrum" series={[{ label: "Now minus held", kind: "compare", curve: rta.compare }]} unit="dB" /> : null}
    </section>
  );
}
