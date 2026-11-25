// WP P5WMS: a measurement's results (PLAN-P5 T1, T8, T11): each output's measured response as
// the document holds it, untrusted points faint, the spread across positions or reseats as a
// band, the timing verdict in words ("timing unreliable" with its reasons), the placement and
// the microphone; the document saved to the hub (or downloaded as text without one), a file
// opened, and REW text exported per output. The curves drawn are the core's (`measureDocCurves`):
// smoothed as chosen, with the target levelled to each output when asked; no maths here (R1).
// The document's own curves (decoded) are drawn only while the core's answer is not in.

import { useEffect, useRef, useState } from "preact/hooks";
import { measureCall, type MeasureDocCurvesRecord } from "../../core/measure-protocol";
import type { MeasureTabProps } from "./MeasureScreen";
import { MeasureGraph, type Series } from "./graph";
import { SMOOTHING, fileNameOf, resultView, smoothingLabel, displayName } from "./model";
import { download, readTextFile } from "./setup";
import { EmptyState, Glyph } from "../../ui/design"; // -- WP WEB -- (Glyph: WEB4, INTD)
import { PageTools } from "../../ui/shell/Parts"; // -- WP WEB4 --

export function ResultsTab(props: MeasureTabProps) {
  const { desk, outputName, session } = props;
  const [hidden, setHidden] = useState<Set<number>>(new Set());
  const [said, setSaid] = useState("");
  const [rewOut, setRewOut] = useState<number | null>(null);
  const [smoothing, setSmoothing] = useState(6);
  const [withTarget, setWithTarget] = useState(true);
  /** The core's curves, with what they were asked for. */
  const [curves, setCurves] = useState<{ text: string; targetDocument: string; smoothing: number; withTarget: boolean; c: MeasureDocCurvesRecord } | null>(null);
  const view = desk.result ? resultView(desk.result) : null;
  const targetDocument = desk.targetDocument;
  useEffect(() => {
    const text = desk.result;
    if (!text || !view) return;
    let alive = true;
    void measureCall(desk.core, "measureDocCurves", { text, options: { smoothing, target: withTarget, targetDocument } }).then(
      (c) => alive && setCurves({ text, targetDocument, smoothing, withTarget, c }),
      (e) => alive && (setCurves(null), desk.fail(e)),
    );
    return () => {
      alive = false;
    };
  }, [desk.result, smoothing, withTarget, targetDocument]);
  const fileField = useRef<HTMLInputElement>(null); // -- WP WEB --
  const open = async (input: HTMLInputElement) => {
    try {
      const f = await readTextFile(input);
      if (!f) return;
      if (!resultView(f.text)) throw new Error(`${f.name} is not a measurement document`);
      await measureCall(desk.core, "measureDocInfo", { text: f.text });
      desk.setResult(f.text, f.name);
      setSaid("");
    } catch (e) {
      desk.fail(e);
    }
  };
  // -- WP WEB4 -- one file field, opened by the empty state's action or the page's tool
  const opener = (
    <input
      ref={fileField}
      type="file"
      class="visually-hidden"
      tabIndex={-1}
      aria-label="Open a measurement file"
      accept=".dspipreset,.json,application/json"
      onChange={(e) => void open(e.currentTarget as HTMLInputElement)}
    />
  );
  // -- WP WEB -- DL2's empty state; its action opens the file field
  if (!desk.result || !view)
    return (
      <section aria-label="Measurement results">
        <EmptyState
          icon="chart-line"
          title="No measurement yet"
          text="Measure on the Measure tab, or open a measurement file."
          action={{ label: "Open a Measurement…", run: () => fileField.current?.click() }}
        >
          {opener}
        </EmptyState>
      </section>
    );

  const text = desk.result;
  const mayFile = desk.hubMeasure && !session.readOnlyReason && desk.hubRole !== "viewer";
  const save = async () => {
    try {
      const r = await measureCall(desk.core, "hubMeasureDocPut", { text });
      setSaid(r.created ? `Saved on the hub as "${r.doc.name}".` : `The hub already holds "${r.doc.name}".`);
    } catch (e) {
      desk.fail(e);
    }
  };
  const exportRew = async (output: number) => {
    try {
      const r = await measureCall(desk.core, "measurementExportRew", { text, output });
      download(r.text, `${fileNameOf(view.name, "measurement").replace(/\.dspipreset$/, "")} - ${outputName(output)}.txt`, "text/plain");
    } catch (e) {
      desk.fail(e);
    }
  };
  const core =
    curves && curves.text === text && curves.targetDocument === targetDocument && curves.smoothing === smoothing && curves.withTarget === withTarget ? curves.c : null;
  const series: Series[] = [];
  if (core) {
    core.outputs.forEach((o, i) => {
      if (hidden.has(o.output)) return;
      series.push({ label: outputName(o.output), kind: "measured", curve: o.measured, spreadDb: o.spreadDb.length ? o.spreadDb : undefined, colour: i });
      if (o.target) series.push({ label: `Target for ${outputName(o.output)}`, kind: "target", curve: o.target });
    });
  } else
    for (const o of view.outputs)
      if (o.curve && !hidden.has(o.output)) series.push({ label: outputName(o.output), kind: "measured", curve: o.curve, spreadDb: o.curve.spreadDb, colour: view.outputs.indexOf(o) });
  const spl = core ? core.outputs.some((o) => o.measured.spl) : view.outputs.some((o) => o.curve?.spl);
  return (
    // -- WP WEB4 -- DL2's Results: no card; the name and what was measured as the page's first line, the
    // timing with an icon, the curves' chips and the graph's options over the graph frame, Open… in the page's tools
    <section class="measure-results" aria-label="Measurement results">
      <PageTools label="Results tools">
        <button type="button" onClick={() => fileField.current?.click()}>
          Open a Measurement…
        </button>
      </PageTools>
      <div class="m-head">
        <strong class="m-title">{view.name ? displayName(view.name) : "Measurement"}</strong>
        <span class="m-caption">
          {view.placement}
          {view.positions > 1 ? ` · ${view.positions} positions averaged (the band is their spread)` : ""} · {view.mic}
          {typeof view.levelDb === "number" ? ` · played at ${view.levelDb.toFixed(0)} dBFS` : ""}
          {view.serial ? ` · ${view.serial}` : ""}
        </span>
      </div>
      <p
        aria-label="Timing"
        data-reliable={view.timingReliable === null ? "none" : String(view.timingReliable)}
        class={`m-iconline ${view.timingReliable === false ? "m-bad" : view.timingReliable ? "m-ok" : "m-note"}`}
      >
        <Glyph name={view.timingReliable === false ? "warning" : view.timingReliable ? "circle-check" : "info"} class="m-line-icon" />
        <span>
        {view.timingReliable === null
          ? "No timing: this measurement is magnitude only."
          : view.timingReliable
            ? `Timing reliable${view.maxResidualUs !== null ? ` (largest cycle residual ${view.maxResidualUs.toFixed(1)} µs)` : ""}: delays, polarity and crossovers may be tuned from it.`
            : `Timing unreliable: the magnitudes are kept, but no delay, polarity or crossover change is offered from it.${view.reasons.length ? ` ${view.reasons.join("; ")}.` : ""}`}
        </span>
      </p>
      <div role="group" aria-label="Curves shown" class="m-checks">
        {view.outputs.map((o) => (
          <label key={o.output}>
            <input
              type="checkbox"
              checked={!hidden.has(o.output)}
              onChange={() => {
                const n = new Set(hidden);
                if (n.has(o.output)) n.delete(o.output);
                else n.add(o.output);
                setHidden(n);
              }}
            />{" "}
            {outputName(o.output)}
            {o.speaker ? ` (${o.speaker}, ${o.role})` : ` (${o.role})`}
            {o.arrivalMs !== null ? ` · arrival ${o.arrivalMs.toFixed(3)} ms` : ""}
            {o.curve ? "" : " · no curve"}
          </label>
        ))}
      </div>
      <div role="group" aria-label="Graph" class="m-line">
        <label class="m-inline">
          Smoothing{" "}
          <select aria-label="Results smoothing" value={String(smoothing)} onChange={(e) => setSmoothing(Number((e.currentTarget as HTMLSelectElement).value))}>
            {SMOOTHING.map((n) => (
              <option key={n} value={n}>
                {smoothingLabel(n)}
              </option>
            ))}
          </select>
        </label>
        <label class="m-check">
          <input type="checkbox" aria-label="Show the target" checked={withTarget} onChange={(e) => setWithTarget((e.currentTarget as HTMLInputElement).checked)} />{" "}
          Target {desk.targetName ? `(${desk.targetName})` : "(the placement's default, our own curve)"}, levelled to each output
        </label>
      </div>
      <MeasureGraph label="Measured responses" series={series} unit={spl ? "dB SPL" : "dB (relative)"} />
      <p class="m-caption" aria-label="Curves drawn" data-smoothing={core ? String(core.smoothing) : "none"}>
        {core ? `Smoothed by the core: ${smoothingLabel(core.smoothing)}.` : "The curves as stored (1/96 octave)."} The live view adds a change's predicted curve.
      </p>
      <div class="m-actions">
        {mayFile ? (
          <button type="button" class="primary" onClick={() => void save()}>
            Save to the hub
          </button>
        ) : null}
        <button type="button" onClick={() => download(text, fileNameOf(view.name, "measurement"))}>
          Download
        </button>
        <label class="m-inline">
          REW text of{" "}
          <select aria-label="Export output" value={String(rewOut ?? view.outputs[0]?.output ?? 0)} onChange={(e) => setRewOut(Number((e.currentTarget as HTMLSelectElement).value))}>
            {view.outputs.map((o) => (
              <option key={o.output} value={o.output}>
                {outputName(o.output)}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          onClick={() => void exportRew(rewOut ?? view.outputs[0]?.output ?? 0)}
        >
          Export REW text
        </button>
        {opener}
      </div>
      {said ? (
        <p class="m-caption" role="status">
          {said}
        </p>
      ) : null}
      {!desk.hubMeasure ? <p class="m-caption">No hub with measurement documents: download the document to keep it.</p> : null}
    </section>
  );
}
