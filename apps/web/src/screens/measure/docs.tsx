// WP P5WAT: the Measure screen's documents (PLAN-P5 T11, section 1.6): this page's documents (a
// run's result, files opened, calibrations and targets imported) and the hub's, in one list by
// kind; use one (a measurement in the results and the tune, a microphone in the setup, a target
// everywhere), rename it, note it, download it, export a measurement's output as REW text, file
// it on the hub, delete it. A hub document is renamed and noted by Control and Admin and deleted
// by Admin only (the hub refuses the others too). The microphone and target libraries: import a
// calibration or a target file, the built-in targets. The core reads and validates every document.

import { useEffect, useState } from "preact/hooks";
import { measureCall, type TargetInfoRecord } from "../../core/measure-protocol";
import type { MeasureTabProps } from "./MeasureScreen";
import { fileNameOf, micSummary } from "./model";
import { download, readTextFile } from "./setup";
import { KINDS, docOutputs, docsDeskOf, tuneDeskOf, type DocRow, type DocsDesk } from "./tune-model";

const row = { display: "flex", flexWrap: "wrap", gap: "6px 12px", alignItems: "center" } as const;
const col = { display: "flex", flexDirection: "column", gap: "10px", minWidth: 0 } as const;

function useDocs(d: DocsDesk, props: MeasureTabProps): void {
  const [, bump] = useState(0);
  useEffect(() => {
    const a = d.watch(() => bump((n) => n + 1));
    const b = props.desk.watch(() => bump((n) => n + 1));
    return () => {
      a();
      b();
    };
  }, [d]);
}

function when(ms: number): string {
  return ms > 0 ? new Date(ms).toLocaleString() : "";
}

export function DocsTab(props: MeasureTabProps) {
  const { desk, session } = props;
  const d = docsDeskOf(desk, () => !!session.readOnlyReason);
  useDocs(d, props);
  const [builtins, setBuiltins] = useState<TargetInfoRecord[]>([]);
  const [use, setUse] = useState<"speakers" | "headphones">("speakers");
  useEffect(() => {
    void d.refresh();
    void measureCall(desk.core, "targetBuiltins", undefined).then(setBuiltins, () => setBuiltins([]));
  }, [d, desk.hubMeasure, desk.result]);
  const rights = d.rights;
  const rows = d.rows;

  const pick = async (input: HTMLInputElement, what: "doc" | "mic" | "target") => {
    try {
      const f = await readTextFile(input);
      if (!f) return;
      if (what === "mic") await d.importMic(f.name, f.text);
      else if (what === "target") await d.importTarget(f.name, f.text, use);
      else await d.addText(f.text, f.name);
    } catch (e) {
      desk.fail(e);
    }
  };

  return (
    <section class="card" aria-label="Measurement documents" style={col}>
      <div class="card-head" style={row}>
        <strong style={{ flex: "1 1 auto" }}>Documents</strong>
        <button type="button" onClick={() => void d.refresh()}>
          Refresh
        </button>
      </div>
      <p class="muted">
        {desk.hubMeasure
          ? `The hub's documents and this page's, in one list. ${rights.remove ? "As the hub's Admin you may delete hub documents." : rights.write ? "Your role may rename, note and file hub documents; only the hub's Admin deletes them." : "Your role may read the hub's documents."}`
          : "No hub with measurement documents: these are this page's documents (download them to keep them)."}
        {d.hubNotice ? ` ${d.hubNotice}` : ""}
      </p>
      {d.error ? (
        <p class="error" role="alert" aria-label="Documents problem">
          {d.error}
        </p>
      ) : null}
      {d.said ? (
        <p class="note" role="status" aria-label="Documents outcome">
          {d.said}
        </p>
      ) : null}
      <p class="muted" aria-label="In use">
        Microphone: {desk.micDocument ? micSummary(desk.micDocument) || "a calibration" : "none (relative dB)"} · Target: {desk.targetName || "the placement's default"}
      </p>
      {KINDS.map((k) => {
        const list = rows.filter((r) => r.info.kind === k.id);
        return (
          <div key={k.id} style={col}>
            <strong>{k.label}</strong>
            {list.length ? (
              <ul aria-label={k.label} style={{ listStyle: "none", padding: 0, margin: 0, display: "flex", flexDirection: "column", gap: "8px" }}>
                {list.map((r) => (
                  <DocItem key={r.key} {...props} d={d} r={r} />
                ))}
              </ul>
            ) : (
              <p class="empty">None.</p>
            )}
          </div>
        );
      })}

      <div style={col} role="group" aria-label="Libraries">
        <strong>Add</strong>
        <div style={row}>
          <label>
            Open a document file…{" "}
            <input type="file" aria-label="Open a document file" accept=".dspipreset,.json,application/json" onChange={(e) => void pick(e.currentTarget as HTMLInputElement, "doc")} />
          </label>
          <label>
            Import a microphone calibration…{" "}
            <input type="file" aria-label="Import a microphone calibration" accept=".txt,.cal,.frd,.csv,text/plain" onChange={(e) => void pick(e.currentTarget as HTMLInputElement, "mic")} />
          </label>
        </div>
        <div style={row}>
          <label>
            Target for{" "}
            <select aria-label="Target use" value={use} onChange={(e) => setUse((e.currentTarget as HTMLSelectElement).value as typeof use)}>
              <option value="speakers">speakers</option>
              <option value="headphones">headphones</option>
            </select>
          </label>
          <label>
            Import a target…{" "}
            <input type="file" aria-label="Import a target" accept=".txt,.csv,.frd,text/plain" onChange={(e) => void pick(e.currentTarget as HTMLInputElement, "target")} />
          </label>
        </div>
        {builtins.length ? (
          <div>
            <span>Built-in targets (our own curves; each placement uses its default):</span>
            <ul class="change-list" aria-label="Built-in targets">
              {builtins.map((b) => (
                <li key={b.id}>
                  {b.name} ({b.targetUse}
                  {b.coupler ? `, ${b.coupler}` : ""}){b.params.length ? `: ${b.params.map((x) => `${x.name} ${x.value}`).join(", ")}` : ""}
                </li>
              ))}
            </ul>
            {desk.targetDocument ? (
              <button type="button" onClick={() => desk.setDocs({ targetDocument: "", targetName: "" })}>
                Use the placement's default target
              </button>
            ) : null}
          </div>
        ) : null}
      </div>
    </section>
  );
}

function DocItem(props: MeasureTabProps & { d: DocsDesk; r: DocRow }) {
  const { d, r, desk, outputName } = props;
  const [edit, setEdit] = useState<{ field: "name" | "note"; value: string } | null>(null);
  const [rewOut, setRewOut] = useState<number | null>(null);
  const rights = d.rights;
  const hub = r.where === "hub";
  const mayEdit = hub ? rights.write : true;
  const mayDelete = hub ? rights.remove : true;
  const outputs = r.text !== undefined ? docOutputs(r.text) : [];
  const label = r.info.name || "(no name)";

  const save = async () => {
    if (!edit) return;
    const ok = edit.field === "name" ? await d.rename(r, edit.value.trim()) : await d.note(r, edit.value);
    if (ok) setEdit(null);
  };
  const dl = async () => {
    const text = await d.textOf(r);
    if (text !== null) download(text, fileNameOf(r.info.name, r.info.kind));
  };
  const rew = async () => {
    const o = rewOut ?? outputs[0] ?? 0;
    const text = await d.exportRew(r, o);
    if (text !== null) download(text, `${fileNameOf(r.info.name, "measurement").replace(/\.dspipreset$/, "")} - ${outputName(o)}.txt`, "text/plain");
  };
  const tune = async () => {
    const text = await d.textOf(r);
    if (text === null) return;
    tuneDeskOf(desk).useMeasurement(text, `"${label}"`);
    props.show("tune");
  };
  const results = async () => {
    if ((await d.use(r)) !== null) props.show("results");
  };

  return (
    <li class="doc-row" data-where={r.where} data-kind={r.info.kind} aria-label={label} style={{ ...col, gap: "4px", borderTop: "1px solid var(--line)", paddingTop: "6px" }}>
      <div style={row}>
        <strong>{label}</strong>
        <span class="muted">
          {hub ? "on the hub" : `this page${r.from ? `, ${r.from}` : ""}`}
          {r.info.serial ? ` · ${r.info.serial}` : ""}
          {r.info.savedMs ? ` · ${when(r.info.savedMs)}` : ""} · {Math.max(1, Math.round(r.info.bytes / 1024))} KiB
        </span>
      </div>
      {r.info.note ? <span class="muted" aria-label="Note">{r.info.note}</span> : null}
      {edit ? (
        <div style={row}>
          <input aria-label={edit.field === "name" ? "New name" : "Note"} value={edit.value} maxLength={edit.field === "name" ? 256 : 4000} onInput={(e) => setEdit({ ...edit, value: (e.currentTarget as HTMLInputElement).value })} />
          <button type="button" class="primary" disabled={edit.field === "name" && !edit.value.trim()} onClick={() => void save()}>
            Save
          </button>
          <button type="button" onClick={() => setEdit(null)}>
            Cancel
          </button>
        </div>
      ) : null}
      <div style={row}>
        {r.info.kind === "measurement" ? (
          <>
            <button type="button" onClick={() => void results()}>
              Show in Results
            </button>
            {props.blocked ? null : (
              <button type="button" onClick={() => void tune()}>
                Tune from this
              </button>
            )}
          </>
        ) : (
          <button type="button" onClick={() => void d.use(r)}>
            Use
          </button>
        )}
        <button type="button" onClick={() => void dl()}>
          Download
        </button>
        {mayEdit ? (
          <>
            <button type="button" onClick={() => setEdit({ field: "name", value: r.info.name })}>
              Rename
            </button>
            <button type="button" onClick={() => setEdit({ field: "note", value: r.info.note })}>
              Note
            </button>
          </>
        ) : null}
        {!hub && rights.write ? (
          <button type="button" onClick={() => void d.saveToHub(r)}>
            Save to the hub
          </button>
        ) : null}
        {mayDelete ? (
          <button type="button" class="danger" onClick={() => void d.remove(r)}>
            Delete
          </button>
        ) : (
          <span class="muted" aria-label="Delete">
            Only the hub's Admin deletes it.
          </span>
        )}
      </div>
      {r.info.kind === "measurement" && outputs.length ? (
        <div style={row}>
          <label>
            REW text of{" "}
            <select aria-label={`REW output of ${label}`} value={String(rewOut ?? outputs[0])} onChange={(e) => setRewOut(Number((e.currentTarget as HTMLSelectElement).value))}>
              {outputs.map((o) => (
                <option key={o} value={o}>
                  {outputName(o)}
                </option>
              ))}
            </select>
          </label>
          <button type="button" onClick={() => void rew()}>
            Export REW text
          </button>
        </div>
      ) : null}
    </li>
  );
}
