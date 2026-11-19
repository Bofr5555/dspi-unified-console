// WP P5WMS: a measurement's results (PLAN-P5 T1, T8, T11): each output's measured response as
// the document holds it, untrusted points faint, the spread across positions or reseats as a
// band, the timing verdict in words ("timing unreliable" with its reasons), the placement and
// the microphone; the document saved to the hub (or downloaded as text without one), a file
// opened, and REW text exported per output.

import { useState } from "preact/hooks";
import { measureCall } from "../../core/measure-protocol";
import type { MeasureTabProps } from "./MeasureScreen";
import { MeasureGraph, type Series } from "./graph";
import { fileNameOf, resultView } from "./model";
import { download, readTextFile } from "./setup";

export function ResultsTab(props: MeasureTabProps) {
  const { desk, outputName, session } = props;
  const [hidden, setHidden] = useState<Set<number>>(new Set());
  const [said, setSaid] = useState("");
  const [rewOut, setRewOut] = useState<number | null>(null);
  const view = desk.result ? resultView(desk.result) : null;
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
  const opener = (
    <label>
      Open a measurement file… <input type="file" aria-label="Open a measurement file" accept=".dspipreset,.json,application/json" onChange={(e) => void open(e.currentTarget as HTMLInputElement)} />
    </label>
  );
  if (!desk.result || !view)
    return (
      <section class="card" aria-label="Measurement results">
        <p class="empty">No measurement yet: run one on the Measure tab, or open a measurement file.</p>
        {opener}
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
  const series: Series[] = view.outputs
    .filter((o) => o.curve && !hidden.has(o.output))
    .map((o) => ({ label: outputName(o.output), kind: "measured", curve: o.curve!, spreadDb: o.curve!.spreadDb, colour: view.outputs.indexOf(o) }));
  const spl = view.outputs.some((o) => o.curve?.spl);
  return (
    <section class="card" aria-label="Measurement results" style={{ display: "flex", flexDirection: "column", gap: "10px" }}>
      <div class="card-head">
        <strong>{view.name || "Measurement"}</strong>
      </div>
      <p class="muted">
        {view.placement}
        {view.positions > 1 ? ` · ${view.positions} positions averaged (the band is their spread)` : ""} · {view.mic}
        {typeof view.levelDb === "number" ? ` · played at ${view.levelDb.toFixed(0)} dBFS` : ""}
        {view.serial ? ` · ${view.serial}` : ""}
      </p>
      <p
        aria-label="Timing"
        data-reliable={view.timingReliable === null ? "none" : String(view.timingReliable)}
        class={view.timingReliable === false ? "error" : "note"}
      >
        {view.timingReliable === null
          ? "No timing: this measurement is magnitude only."
          : view.timingReliable
            ? `Timing reliable${view.maxResidualUs !== null ? ` (largest cycle residual ${view.maxResidualUs.toFixed(1)} µs)` : ""}: delays, polarity and crossovers may be tuned from it.`
            : `Timing unreliable: the magnitudes are kept, but no delay, polarity or crossover change is offered from it.${view.reasons.length ? ` ${view.reasons.join("; ")}.` : ""}`}
      </p>
      <div role="group" aria-label="Curves shown" style={{ display: "flex", flexWrap: "wrap", gap: "6px 14px" }}>
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
      <MeasureGraph label="Measured responses" series={series} unit={spl ? "dB SPL" : "dB (relative)"} />
      <p class="muted">The curves as measured (1/96 octave); the live view shows them smoothed, with the target and a change's predicted curve.</p>
      <div style={{ display: "flex", flexWrap: "wrap", gap: "8px", alignItems: "center" }}>
        {mayFile ? (
          <button type="button" class="primary" onClick={() => void save()}>
            Save to the hub
          </button>
        ) : null}
        <button type="button" onClick={() => download(text, fileNameOf(view.name, "measurement"))}>
          Download
        </button>
        <label>
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
        <p class="note" role="status">
          {said}
        </p>
      ) : null}
      {!desk.hubMeasure ? <p class="muted">No hub with measurement documents: download the document to keep it.</p> : null}
    </section>
  );
}
