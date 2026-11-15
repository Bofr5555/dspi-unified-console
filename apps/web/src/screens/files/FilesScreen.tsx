// WP P1WB1 (the import part of WB6k): configuration files through the browser. Export
// downloads the device as a `.dspipreset`; Import Configuration… reads one and shows, before
// anything is written, what it would change (the core's preview, P1DF) with a choice per
// block and per channel's EQ; then Import, or Import with Rollback (a guarded apply, P1SA:
// Keep or Revert within the time). Over DSPi Link to a hub with the guard every import is
// guarded. Save as Template downloads a template; on a document opened offline, Save File
// downloads it as it now stands.

import { useEffect, useRef, useState } from "preact/hooks";
import { IMPORT_ALL_BLOCKS, IMPORT_EQ, type ConfigDiff } from "../../core/protocol";
import type { ScreenProps } from "../../ui/screens";
import { OutcomeLine, useDesk } from "../snapshots/Bars";
import { BLOCKS, DiffTable } from "../snapshots/DiffTable";
import { docMeta, download, fileNameFor, type SnapshotDesk } from "../snapshots/model";
// -- WP WB6K -- filter files, the saves to the device, the AutoEQ database
import { AutoEqDatabaseCard, DeviceSaveCard, FilterPicker, FiltersCard, type PendingFilters } from "./FilesCards";

export function FilesScreen(props: ScreenProps) {
  const desk = props.desk;
  if (!desk) return null;
  return <FilesView desk={desk} />;
}

interface Pending {
  text: string;
  name: string;
  kind: string;
  /** The preview with every block: which blocks hold changes. */
  all: ConfigDiff;
}

function FilesView(props: { desk: SnapshotDesk }) {
  const { desk } = props;
  const st = useDesk(desk);
  const session = desk.session;
  const snap = session.state.snapshot;
  const doc = session.document;
  const ro = desk.readOnly;
  const busy = !!st.busy;
  const activeName = snap ? (snap.state.presetNames[snap.state.activePresetSlot] ?? "").trim() : "";
  const [name, setName] = useState("");
  const [template, setTemplate] = useState("");
  const [pending, setPending] = useState<Pending | null>(null);
  const [readError, setReadError] = useState("");
  const [filters, setFilters] = useState<PendingFilters | null>(null); // -- WP WB6K --
  const file = useRef<HTMLInputElement>(null);

  const exportName = name.trim() || doc?.name || activeName || "Configuration";

  const choose = async (input: HTMLInputElement) => {
    const f = input.files?.[0];
    input.value = "";
    if (!f) return;
    const text = await f.text();
    try {
      const all = await desk.preview(text, IMPORT_ALL_BLOCKS);
      const m = docMeta(text, f.name);
      setReadError("");
      setPending({ text, name: m.name, kind: m.kind, all });
    } catch (e) {
      setPending(null);
      setReadError(`${f.name}: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  return (
    <section class="files" aria-label="Files">
      <OutcomeLine desk={desk} outcome={st.outcome} />
      {readError ? (
        <p class="error" role="alert">
          {readError}
        </p>
      ) : null}
      {pending ? (
        <ImportPreview desk={desk} pending={pending} onDone={() => setPending(null)} />
      ) : filters ? (
        <FilterPicker desk={desk} pending={filters} onDone={() => setFilters(null)} />
      ) : (
        <>
          <div class="card">
            <h3>Configuration file</h3>
            <p class="note">The whole configuration as a .dspipreset file, the format every DSPi console reads and writes.</p>
            <div class="files-row">
              <label>
                Name
                <input name="export-name" value={name} placeholder={exportName} onInput={(e) => setName((e.target as HTMLInputElement).value)} />
              </label>
              <button
                type="button"
                disabled={busy}
                onClick={() =>
                  void desk.exportDoc(exportName).then((t) => {
                    if (t) download(t, fileNameFor(exportName, "configuration"));
                  })
                }
              >
                Export Configuration
              </button>
              <button type="button" disabled={busy || !!ro} onClick={() => file.current?.click()}>
                Import Configuration…
              </button>
              <input
                ref={file}
                type="file"
                hidden
                accept=".dspipreset,.json,application/json"
                aria-label="Configuration file to import"
                onChange={(e) => void choose(e.target as HTMLInputElement)}
              />
            </div>
          </div>
          {doc ? (
            <div class="card">
              <h3>File</h3>
              <p class="note">
                Editing {doc.fileName || doc.name} with no device. Save File downloads it as it now stands.
              </p>
              <button
                type="button"
                disabled={busy}
                onClick={() =>
                  void desk.saveDocument().then((t) => {
                    if (t) download(t, doc.fileName || fileNameFor(doc.name, "configuration"));
                  })
                }
              >
                Save File
              </button>
            </div>
          ) : null}
          <div class="card">
            <h3>Template</h3>
            <p class="note">A template is a starting point for a new system: open it later with New from Template.</p>
            <div class="files-row">
              <label>
                Template name
                <input name="template-name" value={template} onInput={(e) => setTemplate((e.target as HTMLInputElement).value)} />
              </label>
              <button
                type="button"
                disabled={busy || !template.trim()}
                onClick={() =>
                  void desk.saveTemplate(template.trim()).then((t) => {
                    if (t) download(t, fileNameFor(template.trim(), "template"));
                  })
                }
              >
                Save as Template
              </button>
            </div>
          </div>
          {/* -- WP WB6K -- */}
          <FiltersCard desk={desk} onChosen={setFilters} />
          <DeviceSaveCard desk={desk} />
          <AutoEqDatabaseCard desk={desk} />
        </>
      )}
    </section>
  );
}

/** The import's preview: blocks to take (with how many values each changes), the EQ's channels, the rows. */
function ImportPreview(props: { desk: SnapshotDesk; pending: Pending; onDone: () => void }) {
  const { desk, pending } = props;
  const st = useDesk(desk);
  const snap = desk.session.state.snapshot;
  const counts = pending.all.blocks;
  const [blocks, setBlocks] = useState(() => BLOCKS.reduce((m, b) => ((counts[b.key] ?? 0) > 0 ? m | b.bit : m), 0));
  const channels = snap ? channelList(snap.state.channelNames, snap.state.numChannels) : [];
  const [eq, setEq] = useState(() => channels.reduce((m, c) => (m | (1 << c.wire)) >>> 0, 0));
  const [diff, setDiff] = useState<ConfigDiff | null>(null);
  const [error, setError] = useState("");
  const ro = !!desk.readOnly;
  const waiting = st.guard?.phase === "waiting";

  useEffect(() => {
    let alive = true;
    if (!blocks) {
      setDiff(null);
      return;
    }
    desk.preview(pending.text, blocks, eq || 0).then(
      (d) => alive && (setDiff(d), setError("")),
      (e: unknown) => alive && setError(e instanceof Error ? e.message : String(e)),
    );
    return () => {
      alive = false;
    };
  }, [desk, pending.text, blocks, eq]);

  const apply = async (guarded: boolean) => {
    if (await desk.importDoc(pending.text, pending.name, blocks, eq, guarded)) props.onDone();
  };
  const disabled = ro || !!st.busy || waiting || !blocks;
  const eqRows = counts.eq ?? 0;

  return (
    <div class="card import-preview" role="region" aria-label="Import preview">
      <h3>Import “{pending.name}”</h3>
      {pending.kind !== "preset" ? <p class="note">This file is a {pending.kind}.</p> : null}
      {pending.all.count === 0 ? <p class="note">It holds nothing that differs from the device.</p> : null}
      <fieldset class="import-blocks">
        <legend>What to import</legend>
        {BLOCKS.map((b) => (
          <label key={b.key} class="check">
            <input
              type="checkbox"
              name={`block-${b.key}`}
              checked={(blocks & b.bit) !== 0}
              onChange={(e) => setBlocks((m) => ((e.target as HTMLInputElement).checked ? m | b.bit : m & ~b.bit))}
            />
            {b.label} <span class="note">({counts[b.key] ?? 0})</span>
          </label>
        ))}
      </fieldset>
      {(blocks & IMPORT_EQ) !== 0 && eqRows > 0 && channels.length > 1 ? (
        <fieldset class="import-blocks">
          <legend>EQ of</legend>
          {channels.map((c) => (
            <label key={c.wire} class="check">
              <input
                type="checkbox"
                name={`eq-${c.wire}`}
                checked={(eq & (1 << c.wire)) !== 0}
                onChange={(e) => setEq((m) => ((e.target as HTMLInputElement).checked ? (m | (1 << c.wire)) >>> 0 : (m & ~(1 << c.wire)) >>> 0))}
              />
              {c.name}
            </label>
          ))}
        </fieldset>
      ) : null}
      {error ? (
        <p class="error" role="alert">
          {error}
        </p>
      ) : null}
      {!blocks ? <p class="note">Nothing is chosen.</p> : diff ? <DiffTable diff={diff} label="What the import changes" /> : <p class="note">Reading…</p>}
      {st.guardDefault ? <p class="note">On this hub the import can be taken back: it is kept only when you press Keep within {st.timeoutS} s.</p> : null}
      <div class="snap-actions">
        {st.guardDefault ? null : (
          <button type="button" disabled={disabled} onClick={() => void apply(false)}>
            Import
          </button>
        )}
        <button type="button" disabled={disabled} onClick={() => void apply(true)}>
          Import with Rollback
        </button>
        <button type="button" onClick={props.onDone}>
          Cancel
        </button>
      </div>
    </div>
  );
}

function channelList(names: string[], n: number): { wire: number; name: string }[] {
  return Array.from({ length: Math.min(n, 31) }, (_, i) => ({ wire: i, name: names[i] || `Channel ${i + 1}` }));
}
