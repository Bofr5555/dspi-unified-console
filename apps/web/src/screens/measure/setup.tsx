// WP P5WMS: the measurement's setup (PLAN-P5 T7, T8, section 1.4 steps 1 and 2): the microphone
// and its calibration file, the placement and the coupler, the outputs with their ways, speakers
// and alignment groups, the sweep; then the core's preflight in words. Nothing is played here.

import { useEffect, useRef, useState } from "preact/hooks";
import { Glyph, NoticeRow } from "../../ui/design"; // -- WP WEB4 --
import { measureCall } from "../../core/measure-protocol";
import { noticeRow } from "../../ui/texts";
import type { MeasureTabProps } from "./MeasureScreen";
import { COUPLERS, PLACEMENTS, ROLES, TWO_PASSES_HINT, countRange, draftProblem, initialDraft, layoutKey, micSummary, preflightLines, setupOf, withLayout, withOutputs, type MeasureDraft, type OutputDraft } from "./model";
// -- WP ROLEW -- the rows from the speaker layout, an edit written back, the banner
import { SAVED_TO_LAYOUT, bannerOf, baseOf, withRow } from "../settings/speakers/model";
import { useSpeakerLayout } from "../settings/speakers/use-layout";

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
  const [saved, setSaved] = useState<{ error: boolean; text: string } | null>(null);
  useEffect(() => {
    const cur = withOutputs(desk.draft ?? initialDraft(outputs), outputs);
    const next = layout.loaded && !desk.running ? withLayout(cur, base) : cur;
    if (next !== desk.draft) desk.setDraft(next);
  }, [outputs.join(","), layout.loaded, layoutKey(base)]);
  const set = (patch: Partial<MeasureDraft>) => desk.setDraft({ ...d, ...patch });
  const setOut = (o: number, patch: Partial<OutputDraft>) => set({ outputs: d.outputs.map((x) => (x.output === o ? { ...x, ...patch } : x)) });
  /** An edit of a row's way, speaker or group, kept in the speaker layout too (confirmed as it was). */
  const writeBack = async (o: number, patch: Partial<OutputDraft>) => {
    const row = d.outputs.find((x) => x.output === o);
    if (!row || session.readOnlyReason) return;
    const r = { ...row, ...patch };
    // -- WP WEBM3 -- (RB C3) on the layout as last kept, not as rendered
    const why = await layout.update((v) => ({ layout: withRow(v.stored ?? v.proposal, o, { role: r.role, speaker: r.speaker.trim(), group: r.group }), confirmed: !!v.stored?.confirmedByUser }));
    setSaved(why ? { error: true, text: `Not saved to your speaker layout: ${why}` } : { error: false, text: SAVED_TO_LAYOUT });
  };
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

  // -- WP WEB4 -- DL2's Measure page: sections (label, no box) in the 760 px column, the calibration
  // as a line with Import Calibration…, the placement as chips, the outputs striped, the blocking
  // condition as a warning notice above Check and the one primary, Measure.
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
            value={micValue}
            onChange={(e) => {
              const v = (e.currentTarget as HTMLSelectElement).value;
              desk.setMic(v === "sim" ? { kind: "sim" } : { kind: "browser", deviceId: v.slice(4) });
            }}
          >
            {desk.sim?.available ? <option value="sim">{desk.sim.label} (the demo's room)</option> : null}
            <option value="mic:">The browser's default microphone</option>
            {desk.mics.map((m) => (
              <option key={m.deviceId} value={`mic:${m.deviceId}`}>
                {m.labelled ? m.label : `Microphone ${m.deviceId.slice(0, 6)} (named after the permission)`}
              </option>
            ))}
          </select>
        </div>
        {mic.kind === "browser" ? <p class="m-caption">The browser asks for the microphone when the measurement starts; nothing is played by this page.</p> : null}
        <div class="m-line m-spread">
          {d.micDocument ? (
            <span aria-label="Calibration">{micSummary(d.micDocument)}</span>
          ) : (
            <span>No calibration: levels are relative (dB), with no SPL cap.</span>
          )}
          <span class="m-line">
            {d.micDocument ? (
              <button type="button" onClick={() => (desk.setDocs({ micDocument: "" }), set({ micDocument: "" }))}>
                No calibration
              </button>
            ) : null}
            <button type="button" onClick={() => calField.current?.click()}>
              Import Calibration…
            </button>
          </span>
          <input ref={calField} type="file" class="visually-hidden" tabIndex={-1} aria-label="Import a calibration file" accept=".txt,.cal,.csv,.frd,text/plain" onChange={(e) => void importCal(e.currentTarget as HTMLInputElement)} />
        </div>
      </fieldset>

      <fieldset class="m-sec" disabled={running}>
        <legend class="section-label">Placement</legend>
        <div role="radiogroup" aria-label="Placement" class="chips">
          {PLACEMENTS.map((p) => (
            <button
              type="button"
              role="radio"
              class="chip"
              key={p.id}
              title={p.detail}
              aria-checked={d.placement === p.id}
              onClick={() => set({ placement: p.id, count: p.id === "headphones" ? 3 : d.count < 3 ? 3 : d.count })}
            >
              {p.label}
            </button>
          ))}
        </div>
        <span class="m-caption" aria-label="Placement detail">
          {PLACEMENTS.find((p) => p.id === d.placement)?.detail ?? "Where is the microphone? Speakers or headphones on a coupler: the levels depend on it, and headphones are measured much more quietly."}
        </span>
        {range || d.placement === "headphones" ? (
          <div class="m-line">
            {range ? (
              <label class="m-field">
                <span>{d.placement === "headphones" ? "Reseats" : "Positions"}</span>
                <input type="number" class="m-num" min={range[0]} max={range[1]} step={1} value={d.count} onInput={(e) => set({ count: Number((e.currentTarget as HTMLInputElement).value) })} />
              </label>
            ) : null}
            {d.placement === "headphones" ? (
              <label class="m-field">
                <span>Coupler</span>
                <select value={d.coupler} onChange={(e) => set({ coupler: (e.currentTarget as HTMLSelectElement).value as MeasureDraft["coupler"] })}>
                  {COUPLERS.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.label}
                    </option>
                  ))}
                </select>
              </label>
            ) : null}
          </div>
        ) : null}
      </fieldset>

      <fieldset class="m-sec" disabled={running}>
        <legend class="section-label">Outputs</legend>
        <p class="m-caption">The outputs measured together in one walked run are an alignment group: their delays and polarity are compared within the run only.</p>
        <div class="m-scroll">
        <table class="measure-outputs m-rows" aria-label="Outputs to measure">
          <thead>
            <tr>
              <th scope="col">Output</th>
              <th scope="col">Way</th>
              <th scope="col">Speaker</th>
              <th scope="col" title="The outputs of one group are measured in one run and aligned to each other">Group</th>
            </tr>
          </thead>
          <tbody>
            {d.outputs.map((o) => (
              <tr key={o.output} class={o.on ? undefined : "off"}>
                <td>
                  <label class="m-check">
                    <input type="checkbox" aria-label={`Measure ${outputName(o.output)}`} checked={o.on} onChange={(e) => setOut(o.output, { on: (e.currentTarget as HTMLInputElement).checked })} /> {outputName(o.output)}
                  </label>
                </td>
                <td>
                  <select aria-label={`Way of ${outputName(o.output)}`} value={o.role} onChange={(e) => {
                      const role = (e.currentTarget as HTMLSelectElement).value as OutputDraft["role"];
                      setOut(o.output, { role });
                      void writeBack(o.output, { role });
                    }}
                  >
                    {ROLES.map((r) => (
                      <option key={r.id} value={r.id}>
                        {r.label}
                      </option>
                    ))}
                  </select>
                </td>
                <td>
                  <input type="text" class="m-speaker" aria-label={`Speaker of ${outputName(o.output)}`} size={4} maxLength={16} value={o.speaker} placeholder="Not set" onInput={(e) => setOut(o.output, { speaker: (e.currentTarget as HTMLInputElement).value })} onChange={(e) => void writeBack(o.output, { speaker: (e.currentTarget as HTMLInputElement).value })} />
                </td>
                <td>
                  <input type="number" class="m-num" aria-label={`Group of ${outputName(o.output)}`} min={0} max={15} step={1} value={o.group} onInput={(e) => setOut(o.output, { group: Math.max(0, Math.min(15, Math.trunc(Number((e.currentTarget as HTMLInputElement).value) || 0))) })} onChange={(e) => void writeBack(o.output, { group: Math.max(0, Math.min(15, Math.trunc(Number((e.currentTarget as HTMLInputElement).value) || 0))) })} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        </div>
        <p class="m-caption">A mid or a tweeter is measured only through an active high-pass; the core checks it (the tweeter rule).</p>
        {saved ? (
          <p class={saved.error ? "error" : "m-caption"} role="status" aria-label="Speaker layout saved">
            {saved.text}
          </p>
        ) : null}
      </fieldset>

      <fieldset class="m-sec" disabled={running}>
        <legend class="section-label">Sweep</legend>
        <div class="m-line">
          <label class="m-inline">
            <span>Sweep length</span>
            <input type="number" class="m-num" aria-label="Sweep length" min={1} max={15} step={0.5} value={d.sweepMs / 1000} onInput={(e) => set({ sweepMs: Math.round(Number((e.currentTarget as HTMLInputElement).value) * 1000) })} />
            <span class="m-unit">s</span>
          </label>
          <label class="m-inline">
            <span>Passes</span>
            <input type="number" class="m-num" aria-label="Passes" min={2} max={8} step={1} value={d.passes} onInput={(e) => set({ passes: Number((e.currentTarget as HTMLInputElement).value) })} />
          </label>
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
          {desk.preflight?.clear ? (
            <li role="listitem" class="m-ok">
              <Glyph name="check" class="m-line-icon" />
              Ready: nothing blocks the measurement.
            </li>
          ) : null}
          {lines.map((l, i) => (
            <li key={i} role="listitem" class={l.kind === "blocker" ? "m-bad" : "m-note"} data-key={l.key}>
              <Glyph name={l.kind === "blocker" ? "error" : "info"} class="m-line-icon" />
              <span>
                {l.kind === "blocker" ? "Blocks: " : "Note: "}
                {l.text}
              </span>
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
