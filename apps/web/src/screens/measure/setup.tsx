// WP P5WMS: the measurement's setup (PLAN-P5 T7, T8, section 1.4 steps 1 and 2): the microphone
// and its calibration file, the placement and the coupler, the outputs with their ways, speakers
// and alignment groups, the sweep; then the core's preflight in words. Nothing is played here.

import { useEffect, useRef } from "preact/hooks";
import { Glyph, NoticeRow } from "../../ui/design"; // -- WP WEB4 --
import { measureCall } from "../../core/measure-protocol";
import { noticeRow } from "../../ui/texts";
import type { MeasureTabProps } from "./MeasureScreen";
import { COUPLERS, PLACEMENTS, TWO_PASSES_HINT, countRange, draftProblem, initialDraft, layoutKey, micSummary, preflightLines, setupOf, withLayout, withOutputs, type MeasureDraft, type OutputDraft } from "./model";
import { Stepper } from "./parts"; // -- WP WEB7 --
// -- WP ROLEW -- the rows from the speaker layout, the banner
import { bannerOf, baseOf } from "../settings/speakers/model";
import { useSpeakerLayout } from "../settings/speakers/use-layout";
import { LayoutSummary } from "../settings/speakers/LayoutSummary"; // -- WP UB7 --

/** A file's text, at most `max` bytes (the core bounds it again). */
export async function readTextFile(input: HTMLInputElement, max = 1 << 20): Promise<{ name: string; text: string } | null> {
  const f = input.files?.[0];
  input.value = "";
  if (!f) return null;
  if (f.size > max) throw new Error(`${f.name} is larger than ${Math.round(max / 1024)} KiB`);
  return { name: f.name, text: await f.text() };
}

/** Saves `text` as a download (the documents without a hub). */
export function download(text: string, fileName: string, type = "application/json"): void {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(new Blob([text], { type }));
  a.download = fileName;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

export function SetupPanel(props: MeasureTabProps) {
  const { desk, outputs, outputName, session } = props;
  const d = desk.draft ?? initialDraft(outputs);
  // -- WP ROLEW --
  const layout = useSpeakerLayout(session);
  const base = baseOf(layout.view);
  useEffect(() => {
    const cur = withOutputs(desk.draft ?? initialDraft(outputs), outputs);
    const next = layout.loaded && !desk.running ? withLayout(cur, base) : cur;
    if (next !== desk.draft) desk.setDraft(next);
  }, [outputs.join(","), layout.loaded, layoutKey(base)]);
  const set = (patch: Partial<MeasureDraft>) => desk.setDraft({ ...d, ...patch });
  const setOut = (o: number, patch: Partial<OutputDraft>) => set({ outputs: d.outputs.map((x) => (x.output === o ? { ...x, ...patch } : x)) });
  const banner = layout.loaded ? bannerOf(layout.view) : null;
  const running = desk.running;
  const problem = draftProblem(d);
  const range = countRange(d.placement);
  const mic = desk.mic;
  const micValue = mic.kind === "sim" ? "sim" : `mic:${mic.deviceId}`;
  const lines = desk.preflight ? preflightLines(desk.preflight, (k) => noticeRow(k)?.title ?? null, outputName) : [];

  const importCal = async (input: HTMLInputElement) => {
    try {
      const f = await readTextFile(input);
      if (!f) return;
      const { text } = await measureCall(desk.core, "micImport", { name: f.name.replace(/\.[^.]*$/, ""), fileName: f.name, text: f.text });
      desk.setDocs({ micDocument: text });
      set({ micDocument: text });
    } catch (e) {
      desk.fail(e);
    }
  };
  const preflight = () => void desk.runPreflight(setupOf(d, 48_000));
  // The microphone first (Firefox opens audio only close to the click), then the core's preflight.
  const start = () => void desk.startRun(setupOf(d, 48_000));

  // -- WP WEB4 -- DL2's Measure page: sections (label, no box) in the 760 px column; the blocking
  // condition as a warning notice above Check and the one primary, Measure.
  // -- WP WEB7 -- as Qt's MeasureSetupTab: the microphone combo with Refresh and its caption, the
  // placement as a combo ("Choose the placement" until one is chosen), the outputs as rows (the
  // core's label, Way, Speaker, a Group stepper; no header row) with "Propose from the Device", the
  // sweep's steppers, the preflight's lines in Qt's words.
  const calField = useRef<HTMLInputElement>(null);
  return (
    <section class="measure-setup measure-column" aria-label="Measurement setup">
      {banner ? (
        <div class={`notice-row notice-row-${banner.kind === "stated" || banner.kind === "imported" ? "info" : "warning"}`} role="status" aria-label="Speaker layout" data-kind={banner.kind}>
          <Glyph name={banner.kind === "stated" || banner.kind === "imported" ? "info" : "warning"} class="notice-icon" />
          <span class="notice-text">{banner.text}</span>
        </div>
      ) : null}
      {layout.view.problem ? (
        <p class="error" role="alert" aria-label="Speaker layout problem">
          {layout.view.problem}
        </p>
      ) : null}

      <fieldset class="m-sec" disabled={running}>
        <legend class="section-label">Microphone</legend>
        <div class="m-line">
          <select
            aria-label="Microphone"
            class="m-combo m-combo-wide"
            value={micValue}
            onChange={(e) => {
              const v = (e.currentTarget as HTMLSelectElement).value;
              desk.setMic(v === "sim" ? { kind: "sim" } : { kind: "browser", deviceId: v.slice(4) });
            }}
          >
            {desk.sim?.available ? <option value="sim">{desk.sim.label}</option> : null}
            <option value="mic:">The browser's default microphone</option>
            {desk.mics.map((m) => (
              <option key={m.deviceId} value={`mic:${m.deviceId}`}>
                {m.labelled ? m.label : `Microphone ${m.deviceId.slice(0, 6)} (named after the permission)`}
              </option>
            ))}
          </select>
          <button type="button" onClick={() => void desk.refreshMics()}>
            Refresh
          </button>
        </div>
        {mic.kind === "sim" ? (
          <p class="m-caption">
            The demo DSPi's simulated room: nothing is played or captured, the microphone is arithmetic in the core.
          </p>
        ) : (
          <p class="m-caption">
            The browser asks for the microphone when the measurement starts; nothing is played by this page.
          </p>
        )}
        <div class="m-line m-spread">
          {d.micDocument ? <span aria-label="Calibration">{micSummary(d.micDocument)}</span> : <span aria-label="Calibration">Uncalibrated: levels are relative dB.</span>}
          <span class="m-line">
            <button type="button" onClick={() => calField.current?.click()}>
              Import Calibration…
            </button>
            {d.micDocument ? (
              <button type="button" onClick={() => (desk.setDocs({ micDocument: "" }), set({ micDocument: "" }))}>
                Uncalibrated
              </button>
            ) : null}
          </span>
          <input ref={calField} type="file" class="visually-hidden" tabIndex={-1} aria-label="Import a calibration file" accept=".txt,.cal,.csv,.frd,text/plain" onChange={(e) => void importCal(e.currentTarget as HTMLInputElement)} />
        </div>
      </fieldset>

      <fieldset class="m-sec" disabled={running}>
        <legend class="section-label">Placement</legend>
        <div class="m-line">
          <select
            aria-label="Placement"
            class="m-combo"
            value={d.placement ?? ""}
            onChange={(e) => {
              const id = (e.currentTarget as HTMLSelectElement).value as MeasureDraft["placement"];
              if (id) set({ placement: id, count: id === "headphones" ? 3 : d.count < 3 ? 3 : d.count });
            }}
          >
            {d.placement ? null : (
              <option value="" disabled>
                Choose the placement
              </option>
            )}
            {PLACEMENTS.map((p) => (
              <option key={p.id} value={p.id}>
                {p.label}
              </option>
            ))}
          </select>
          {range ? <Stepper label={d.placement === "headphones" ? "Reseats" : "Positions"} value={d.count} min={range[0]} max={range[1]} onValue={(v) => set({ count: v })} /> : null}
          {d.placement === "headphones" ? (
            <select aria-label="Coupler" class="m-combo m-combo-coupler" value={d.coupler} onChange={(e) => set({ coupler: (e.currentTarget as HTMLSelectElement).value as MeasureDraft["coupler"] })}>
              {COUPLERS.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.label}
                </option>
              ))}
            </select>
          ) : null}
        </div>
        <span class="m-caption" aria-label="Placement detail">
          {PLACEMENTS.find((p) => p.id === d.placement)?.detail ?? "Where is the microphone? Speakers or headphones on a coupler: the levels depend on it, and headphones are measured much more quietly."}
        </span>
      </fieldset>

      <fieldset class="m-sec" disabled={running}>
        <legend class="section-label">Outputs</legend>
        {/* -- WP UB7 -- (UXP B7) the speaker layout in one line, edited in Settings › Speakers; this run's outputs are the tick boxes */}
        <LayoutSummary view={layout.view} loaded={layout.loaded} onEdit={props.openSpeakers} />
        <p class="m-caption">Tick the outputs to measure in this run. Their roles, and which of them are measured together, come from the speaker layout.</p>
        <ul class="measure-outputs m-checks" aria-label="Outputs to measure">
          {d.outputs.map((o) => (
            <li key={o.output} class={o.on ? undefined : "off"} data-output={o.output}>
              <label class="m-check m-outname">
                <input type="checkbox" aria-label={`Measure ${outputName(o.output)}`} checked={o.on} onChange={(e) => setOut(o.output, { on: (e.currentTarget as HTMLInputElement).checked })} /> {outputName(o.output)}
              </label>
            </li>
          ))}
        </ul>
        <p class="m-caption">A mid driver or a tweeter is measured only through an active high-pass; the core checks it (the tweeter rule).</p>
      </fieldset>

      <fieldset class="m-sec" disabled={running}>
        <legend class="section-label">Sweep</legend>
        <div class="m-line">
          <span>Sweep length</span>
          <Stepper label="Sweep length" value={d.sweepMs / 1000} min={1} max={15} step={0.5} decimals={1} unit="s" class="m-stepper-wide" onValue={(v) => set({ sweepMs: Math.round(v * 1000) })} />
          <span>Passes</span>
          <Stepper label="Passes" value={d.passes} min={2} max={8} onValue={(v) => set({ passes: v })} />
          <label class="m-inline m-grow">
            <span>Name</span>
            <input type="text" aria-label="Measurement name" maxLength={256} value={d.name} placeholder="By the date" onInput={(e) => set({ name: (e.currentTarget as HTMLInputElement).value })} />
          </label>
        </div>
        {d.passes === 2 ? (
          <span class="m-caption" aria-label="Passes hint">
            {TWO_PASSES_HINT}
          </span>
        ) : null}
        <p class="m-caption">The level is the core's: it starts quiet and rises in steps until the microphone hears the output well, never past the ceiling.</p>
      </fieldset>

      {lines.length || desk.preflight ? (
        <ul role="list" aria-label="Preflight" class="m-lines">
          <li role="listitem" class={desk.preflight?.clear ? "m-ok" : "m-bad"}>
            <Glyph name={desk.preflight?.clear ? "circle-check" : "error"} class="m-line-icon" />
            {desk.preflight?.clear ? "Clear to measure." : "Not clear to measure:"}
          </li>
          {lines.map((l, i) => (
            <li key={i} role="listitem" class={l.kind === "blocker" ? "m-bad" : "m-note"} data-key={l.key} data-kind={l.kind}>
              <Glyph name={l.kind === "blocker" ? "error" : "warning"} class="m-line-icon" />
              <span>{l.text}</span>
            </li>
          ))}
        </ul>
      ) : null}
      {problem ? <NoticeRow kind="warning">{problem}</NoticeRow> : null}
      <div class="m-actions">
        <button type="button" disabled={running || !!problem} onClick={preflight}>
          Check
        </button>
        <button type="button" class="primary" disabled={running || !!problem} onClick={start}>
          Measure
        </button>
      </div>
    </section>
  );
}
