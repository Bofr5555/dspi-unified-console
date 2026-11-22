// WP P5WMS: the measurement's setup (PLAN-P5 T7, T8, section 1.4 steps 1 and 2): the microphone
// and its calibration file, the placement and the coupler, the outputs with their ways, speakers
// and alignment groups, the sweep; then the core's preflight in words. Nothing is played here.

import { useEffect, useState } from "preact/hooks";
import { measureCall } from "../../core/measure-protocol";
import { noticeRow } from "../../ui/texts";
import type { MeasureTabProps } from "./MeasureScreen";
import { COUPLERS, PLACEMENTS, ROLES, TWO_PASSES_HINT, countRange, draftProblem, initialDraft, layoutKey, micSummary, preflightLines, setupOf, withLayout, withOutputs, type MeasureDraft, type OutputDraft } from "./model";
// -- WP ROLEW -- the rows from the speaker layout, an edit written back, the banner
import { SAVED_TO_LAYOUT, bannerOf, baseOf, withRow } from "../settings/speakers/model";
import { useSpeakerLayout } from "../settings/speakers/use-layout";

const field = { display: "flex", flexDirection: "column", gap: "2px", minWidth: 0 } as const;
const row = { display: "flex", flexWrap: "wrap", gap: "8px 16px", alignItems: "flex-end" } as const;

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

  return (
    <section class="card" aria-label="Measurement setup" style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
      <div class="card-head">
        <strong>Setup</strong>
      </div>
      {banner ? (
        <div class="banner" role="status" aria-label="Speaker layout" data-kind={banner.kind} style={{ flexWrap: "wrap", margin: 0 }}>
          <span style={{ flex: "1 1 14em", minWidth: 0 }}>{banner.text}</span>
        </div>
      ) : null}
      {layout.view.problem ? (
        <p class="error" role="alert" aria-label="Speaker layout problem">
          {layout.view.problem}
        </p>
      ) : null}

      <fieldset style={{ border: 0, padding: 0, margin: 0, ...row }} disabled={running}>
        <label style={field}>
          <span>Microphone</span>
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
        </label>
        <label style={field}>
          <span>Calibration file (UMIK, Dayton, REW or CSV)</span>
          <input type="file" aria-label="Import a calibration file" accept=".txt,.cal,.csv,.frd,text/plain" onChange={(e) => void importCal(e.currentTarget as HTMLInputElement)} />
        </label>
        {d.micDocument ? (
          <span style={field}>
            <span aria-label="Calibration">{micSummary(d.micDocument)}</span>
            <button type="button" onClick={() => (desk.setDocs({ micDocument: "" }), set({ micDocument: "" }))}>
              No calibration
            </button>
          </span>
        ) : (
          <span class="muted">No calibration: levels are relative (dB), with no SPL cap.</span>
        )}
      </fieldset>
      {mic.kind === "browser" ? <p class="muted">The browser asks for the microphone when the measurement starts; nothing is played by this page.</p> : null}

      <fieldset style={{ border: 0, padding: 0, margin: 0, display: "flex", flexDirection: "column", gap: "6px" }} disabled={running}>
        <legend>Placement</legend>
        <div role="radiogroup" aria-label="Placement" style={row}>
          {PLACEMENTS.map((p) => (
            <label key={p.id} title={p.detail}>
              <input type="radio" name="measure-placement" checked={d.placement === p.id} onChange={() => set({ placement: p.id, count: p.id === "headphones" ? 3 : d.count < 3 ? 3 : d.count })} /> {p.label}
            </label>
          ))}
        </div>
        <span class="muted" aria-label="Placement detail">
          {PLACEMENTS.find((p) => p.id === d.placement)?.detail ?? "Where is the microphone? Speakers or headphones on a coupler: the levels depend on it, and headphones are measured much more quietly."}
        </span>
        <div style={row}>
          {range ? (
            <label style={field}>
              <span>{d.placement === "headphones" ? "Reseats" : "Positions"}</span>
              <input type="number" min={range[0]} max={range[1]} step={1} value={d.count} onInput={(e) => set({ count: Number((e.currentTarget as HTMLInputElement).value) })} />
            </label>
          ) : null}
          {d.placement === "headphones" ? (
            <label style={field}>
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
          <label style={field}>
            <span>Sweep, s</span>
            <input type="number" aria-label="Sweep length" min={1} max={15} step={0.5} value={d.sweepMs / 1000} onInput={(e) => set({ sweepMs: Math.round(Number((e.currentTarget as HTMLInputElement).value) * 1000) })} />
          </label>
          <label style={field}>
            <span>Passes</span>
            <input type="number" aria-label="Passes" min={2} max={8} step={1} value={d.passes} onInput={(e) => set({ passes: Number((e.currentTarget as HTMLInputElement).value) })} />
            {d.passes === 2 ? (
              <span class="muted" aria-label="Passes hint">
                {TWO_PASSES_HINT}
              </span>
            ) : null}
          </label>
          <label style={{ ...field, flex: "1 1 10em" }}>
            <span>Name</span>
            <input type="text" aria-label="Measurement name" maxLength={256} value={d.name} placeholder="By the date" onInput={(e) => set({ name: (e.currentTarget as HTMLInputElement).value })} />
          </label>
        </div>
      </fieldset>

      <fieldset style={{ border: 0, padding: 0, margin: 0, minWidth: 0 }} disabled={running}>
        <legend>Outputs</legend>
        <div style={{ overflowX: "auto" }}>
          <table class="measure-outputs" aria-label="Outputs to measure" style={{ borderCollapse: "collapse", width: "100%" }}>
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
                <tr key={o.output}>
                  <td>
                    <label>
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
                    <input type="text" aria-label={`Speaker of ${outputName(o.output)}`} size={4} maxLength={16} value={o.speaker} onInput={(e) => setOut(o.output, { speaker: (e.currentTarget as HTMLInputElement).value })} onChange={(e) => void writeBack(o.output, { speaker: (e.currentTarget as HTMLInputElement).value })} />
                  </td>
                  <td>
                    <input type="number" aria-label={`Group of ${outputName(o.output)}`} min={0} max={15} step={1} style={{ width: "4em" }} value={o.group} onInput={(e) => setOut(o.output, { group: Math.max(0, Math.min(15, Math.trunc(Number((e.currentTarget as HTMLInputElement).value) || 0))) })} onChange={(e) => void writeBack(o.output, { group: Math.max(0, Math.min(15, Math.trunc(Number((e.currentTarget as HTMLInputElement).value) || 0))) })} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p class="muted">A mid or a tweeter is measured only through an active high-pass; the core checks it (the tweeter rule).</p>
        {saved ? (
          <p class={saved.error ? "error" : "note"} role="status" aria-label="Speaker layout saved">
            {saved.text}
          </p>
        ) : null}
      </fieldset>

      {lines.length || desk.preflight ? (
        <div role="list" aria-label="Preflight">
          {desk.preflight?.clear ? (
            <p role="listitem" class="note">
              Ready: nothing blocks the measurement.
            </p>
          ) : null}
          {lines.map((l, i) => (
            <p key={i} role="listitem" class={l.kind === "blocker" ? "error" : "muted"} data-key={l.key}>
              {l.kind === "blocker" ? "Blocks: " : "Note: "}
              {l.text}
            </p>
          ))}
        </div>
      ) : null}
      {problem ? <p class="muted">{problem}</p> : null}
      <div style={row}>
        <button type="button" disabled={running || !!problem} onClick={preflight}>
          Check
        </button>
        <button type="button" class="primary" disabled={running || !!problem} onClick={start}>
          Start measuring
        </button>
      </div>
    </section>
  );
}
