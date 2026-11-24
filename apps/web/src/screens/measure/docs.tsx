// WP P5WAT: the Measure screen's documents (PLAN-P5 T11, section 1.6): this page's documents (a
// run's result, files opened, calibrations and targets imported) and the hub's, in one list by
// kind; use one (a measurement in the results and the tune, a microphone in the setup, a target
// everywhere), rename it, note it, download it, export a measurement's output as REW text, file
// it on the hub, delete it. A hub document is renamed and noted by Control and Admin and deleted
// by Admin only (the hub refuses the others too). The microphone and target libraries: import a
// calibration or a target file, the built-in targets. The core reads and validates every document.

import { useEffect, useRef, useState } from "preact/hooks";
import { measureCall, type TargetInfoRecord } from "../../core/measure-protocol";
import type { MeasureTabProps } from "./MeasureScreen";
import { fileNameOf, micSummary } from "./model";
import { download, readTextFile } from "./setup";
import { KINDS, docOutputs, docsDeskOf, tuneDeskOf, type DocRow, type DocsDesk } from "./tune-model";
import { EmptyState, Glyph } from "../../ui/design"; // -- WP WEB -- (Glyph: WEB4, INTD)

const row = { display: "flex", flexWrap: "wrap", gap: "6px 12px", alignItems: "center" } as const;

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
  const micField = useRef<HTMLInputElement>(null); // -- WP WEB --

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

  // -- WP WEB4 -- DL2's Documents: no card; the tools in one row at the top, the empty state alone when
  // there is nothing, the kinds as sections of striped rows, the built-in targets as rows (their names)
  const docField = useRef<HTMLInputElement>(null);
  const targetField = useRef<HTMLInputElement>(null);
  return (
    <section class="measure-docs measure-column" aria-label="Measurement documents">
      <div class="m-toolbar">
        <label class="m-inline">
          Targets for{" "}
          <select aria-label="Target use" value={use} onChange={(e) => setUse((e.currentTarget as HTMLSelectElement).value as typeof use)}>
            <option value="speakers">speakers</option>
            <option value="headphones">headphones</option>
          </select>
        </label>
        <button type="button" onClick={() => targetField.current?.click()}>
          Import Target…
        </button>
        {rows.length ? (
          <button type="button" onClick={() => micField.current?.click()}>
            Import Calibration…
          </button>
        ) : null}
        <button type="button" onClick={() => docField.current?.click()}>
          Open…
        </button>
        <button type="button" onClick={() => void d.refresh()}>
          Refresh
        </button>
        <input ref={docField} type="file" class="visually-hidden" tabIndex={-1} aria-label="Open a document file" accept=".dspipreset,.json,application/json" onChange={(e) => void pick(e.currentTarget as HTMLInputElement, "doc")} />
        <input ref={micField} type="file" class="visually-hidden" tabIndex={-1} aria-label="Import a microphone calibration" accept=".txt,.cal,.frd,.csv,text/plain" onChange={(e) => void pick(e.currentTarget as HTMLInputElement, "mic")} />
        <input ref={targetField} type="file" class="visually-hidden" tabIndex={-1} aria-label="Import a target" accept=".txt,.csv,.frd,text/plain" onChange={(e) => void pick(e.currentTarget as HTMLInputElement, "target")} />
      </div>
      {d.error ? (
        <p class="error" role="alert" aria-label="Documents problem">
          {d.error}
        </p>
      ) : null}
      {d.said ? (
        <p class="m-iconline m-ok" role="status" aria-label="Documents outcome">
          <Glyph name="circle-check" class="m-line-icon" />
          <span>{d.said}</span>
        </p>
      ) : null}
      {/* the hub's rights in words, also before its first document (who may file, rename, delete) */}
      {rows.length || desk.hubMeasure ? (
        <p class="m-caption">
          {desk.hubMeasure
            ? `The hub's documents and this page's, in one list. ${rights.remove ? "As the hub's Admin you may delete hub documents." : rights.write ? "Your role may rename, note and file hub documents; only the hub's Admin deletes them." : "Your role may read the hub's documents."}`
            : "No hub with measurement documents: these are this page's documents (download them to keep them)."}
          {d.hubNotice ? ` ${d.hubNotice}` : ""}
        </p>
      ) : null}
      {/* -- WP WEB -- DL2's empty state while there is no document of any kind */}
      {rows.length === 0 ? (
        <EmptyState
          icon="folder-open"
          title="No documents yet"
          text="Measurements you save from Results, microphone calibrations and targets are kept here."
          action={{ label: "Import Calibration…", run: () => micField.current?.click() }}
        />
      ) : (
        <p class="m-caption" aria-label="In use">
          Microphone: {desk.micDocument ? micSummary(desk.micDocument) || "a calibration" : "none (relative dB)"} · Target: {desk.targetName || "the placement's default"}
        </p>
      )}
      {rows.length === 0 ? null : KINDS.map((k) => {
        const list = rows.filter((r) => r.info.kind === k.id);
        return (
          <div key={k.id} class="m-sec">
            <h4 class="section-label">{k.label}</h4>
            {list.length ? (
              <ul aria-label={k.label} class="m-rowlist m-docs">
                {list.map((r) => (
                  <DocItem key={r.key} {...props} d={d} r={r} />
                ))}
              </ul>
            ) : (
              <p class="m-caption">None.</p>
            )}
          </div>
        );
      })}

      {rows.length && builtins.length ? (
        <div class="m-sec" role="group" aria-label="Libraries">
          <div class="ds-section-head">
            <h4 class="section-label">Built-in targets</h4>
            {desk.targetDocument ? (
              <button type="button" onClick={() => desk.setDocs({ targetDocument: "", targetName: "" })}>
                Use the placement's default target
              </button>
            ) : null}
          </div>
          <p class="m-caption">Our own curves; each placement uses its default.</p>
          <ul class="m-rowlist" aria-label="Built-in targets">
            {builtins.map((b) => (
              <li key={b.id} title={b.params.map((x) => `${x.name} ${x.value}`).join(", ")}>
                <span>{b.name}</span>
                <span class="m-caption">
                  {b.targetUse}
                  {b.coupler ? ` · ${b.coupler}` : ""}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
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
    <li class="doc-row" data-where={r.where} data-kind={r.info.kind} aria-label={label}>
      <div style={row}>
        <strong>{label}</strong>
        <span class="m-caption">
          {hub ? "on the hub" : `this page${r.from ? `, ${r.from}` : ""}`}
          {r.info.serial ? ` · ${r.info.serial}` : ""}
          {r.info.savedMs ? ` · ${when(r.info.savedMs)}` : ""} · {Math.max(1, Math.round(r.info.bytes / 1024))} KiB
        </span>
      </div>
      {r.info.note ? <span class="m-caption" aria-label="Note">{r.info.note}</span> : null}
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
      <div style={row} class="m-rowactions">
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
          <button type="button" class="danger-text" onClick={() => void d.remove(r)}>
            Delete
          </button>
        ) : (
          <span class="m-caption" aria-label="Delete">
            Only the hub's Admin deletes it.
          </span>
        )}
      </div>
      {r.info.kind === "measurement" && outputs.length ? (
        <div style={row} class="m-rowactions">
          <label class="m-inline">
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
