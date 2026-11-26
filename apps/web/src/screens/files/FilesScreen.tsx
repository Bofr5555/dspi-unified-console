// WP P1WB1 (the import part of WB6k): configuration files through the browser. Export
// downloads the device as a `.dspipreset`; Import Configuration… reads one and shows, before
// anything is written, what it would change (the core's preview, P1DF) with a choice per
// block and per channel's EQ; then Import, or Import with Rollback (a guarded apply, P1SA:
// Keep or Revert within the time). Over DSPi Link to a hub with the guard every import is
// guarded. Save as Template downloads a template; on a document opened offline, Save File
// downloads it as it now stands.

import { useEffect, useRef, useState } from "preact/hooks";
import { IMPORT_ALL_BLOCKS, IMPORT_EQ, type ConfigDiff, type TemplateRecord } from "../../core/protocol";
import type { ScreenProps } from "../../ui/screens";
import { OutcomeLine, useDesk } from "../snapshots/Bars";
import { BLOCKS, DiffTable } from "../snapshots/DiffTable";
import { docMeta, download, fileNameFor, type SnapshotDesk } from "../snapshots/model";
// -- WP WB6K -- filter files, the saves to the device, the AutoEQ database
import { FilterPicker, FiltersCard, type PendingFilters } from "./FilesCards";
import { Dialog } from "../main/Dialog"; // -- WP WEB7 --
import { CONNECT } from "../../ui/shell/sections"; // -- WP WEB7 --
import "../snapshots/rows.css"; // -- WP WEB7 --
import "./files.css"; // -- WP WEB4 --

export function FilesScreen(props: ScreenProps) {
  const desk = props.desk;
  if (!desk) return null;
  return <FilesView desk={desk} go={props.go} />;
}

interface Pending {
  text: string;
  name: string;
  kind: string;
  /** The preview with every block: which blocks hold changes. */
  all: ConfigDiff;
}

/** -- WP WEB7 -- A name asked for before a download (Export Device Configuration…, Save as Template…). */
function NameDialog(props: { title: string; label: string; initial: string; ok: string; onOk: (name: string) => void; onCancel: () => void }) {
  const [name, setName] = useState(props.initial);
  const ok = () => name.trim() && props.onOk(name.trim());
  return (
    <Dialog
      title={props.title}
      onCancel={props.onCancel}
      buttons={[
        { label: props.ok, onClick: ok },
        { label: "Cancel", onClick: props.onCancel },
      ]}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          ok();
        }}
      >
        <label class="dialog-field">
          {props.label}
          <input name="name" value={name} autoFocus autoComplete="off" onInput={(e) => setName((e.target as HTMLInputElement).value)} />
        </label>
      </form>
    </Dialog>
  );
}

// -- WP WEB7 -- Qt's Library › Files (LibraryFiles.qml): Filters, Device configuration, Files without a
// device, Templates; each its caps label, one muted sentence and its buttons in actions.json's words.
// The saves of the master volume and the output configuration are Save ▾'s rows (as Qt's), the AutoEQ
// database is Library › AutoEQ's ▾. A file or a template opens as its own session, which the page's
// Devices › Open a DSPi does: Open File… and a template's Open go there.
function FilesView(props: { desk: SnapshotDesk; go?: ScreenProps["go"] }) {
  const { desk } = props;
  const st = useDesk(desk);
  const session = desk.session;
  const snap = session.state.snapshot;
  const doc = session.document;
  const ro = desk.readOnly;
  const busy = !!st.busy;
  const activeName = snap ? (snap.state.presetNames[snap.state.activePresetSlot] ?? "").trim() : "";
  const [asking, setAsking] = useState<"export" | "template" | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);
  const [readError, setReadError] = useState("");
  const [filters, setFilters] = useState<PendingFilters | null>(null); // -- WP WB6K --
  const [templates, setTemplates] = useState<TemplateRecord[] | null>(null);
  const file = useRef<HTMLInputElement>(null);

  const exportName = doc?.name || activeName || "Configuration";

  useEffect(() => {
    let alive = true;
    session.core.call("templateList", undefined).then(
      (l) => alive && setTemplates(l),
      () => alive && setTemplates([]),
    );
    return () => {
      alive = false;
    };
  }, [session]);

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
  const toDevices = props.go ? () => props.go!(CONNECT) : undefined;

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
          <FiltersCard desk={desk} onChosen={setFilters} />
          <div class="card" role="region" aria-label="Device configuration">
            <h3>Device configuration</h3>
            <p class="note">Every setting of the device. An import shows what it changes first.</p>
            <div class="files-row">
              <button type="button" disabled={busy || !!ro} onClick={() => file.current?.click()}>
                Import Device Configuration…
              </button>
              <input
                ref={file}
                type="file"
                hidden
                accept=".dspipreset,.json,application/json"
                aria-label="Configuration file to import"
                onChange={(e) => void choose(e.target as HTMLInputElement)}
              />
              <button type="button" disabled={busy} onClick={() => setAsking("export")}>
                Export Device Configuration…
              </button>
            </div>
          </div>
          <div class="card" role="region" aria-label="Files without a device">
            <h3>Files without a device</h3>
            <p class="note">
              {doc
                ? `Open now: ${doc.fileName || doc.name || "not saved yet"}`
                : "Edit a configuration on every page with no device, then save it or import it to a DSPi."}
            </p>
            <div class="files-row">
              <button type="button" disabled={!toDevices} title="Open a file on the Devices page: it opens as its own tab" onClick={toDevices}>
                Open File…
              </button>
              <button
                type="button"
                disabled={busy || !doc}
                onClick={() =>
                  void desk.saveDocument().then((t) => {
                    if (t && doc) download(t, doc.fileName || fileNameFor(doc.name, "configuration"));
                  })
                }
              >
                Save File
              </button>
              <button type="button" disabled={busy} onClick={() => setAsking("template")}>
                Save as Template…
              </button>
            </div>
          </div>
          <div class="card" role="region" aria-label="Templates">
            <h3>Templates</h3>
            <p class="note">A template opens as a new file with no device.</p>
            {templates === null ? null : templates.length ? (
              <ul class="lib-rows files-templates" aria-label="Templates">
                {templates.map((tpl) => (
                  <li key={tpl.id} class="lib-row" aria-label={tpl.name}>
                    <span class="lib-text">
                      <span class="lib-title">{tpl.name}</span>
                      <span class="lib-sub">
                        {tpl.platform}, {tpl.inputs} in, {tpl.outputs} out · {tpl.builtin ? "shipped with the app" : "saved here"}
                      </span>
                      {tpl.status ? <span class="lib-sub">{tpl.status}</span> : null}
                    </span>
                    <span class="lib-actions">
                      <button type="button" aria-label={`Open template ${tpl.name}`} disabled={!toDevices} title="New from Template… on the Devices page" onClick={toDevices}>
                        Open
                      </button>
                    </span>
                  </li>
                ))}
              </ul>
            ) : (
              <p class="note">No templates</p>
            )}
          </div>
        </>
      )}
      {asking === "export" ? (
        <NameDialog
          title="Export Device Configuration"
          label="Name"
          initial={exportName}
          ok="Export"
          onCancel={() => setAsking(null)}
          onOk={(name) => {
            setAsking(null);
            void desk.exportDoc(name).then((t) => {
              if (t) download(t, fileNameFor(name, "configuration"));
            });
          }}
        />
      ) : asking === "template" ? (
        <NameDialog
          title="Save as Template"
          label="Template name"
          initial=""
          ok="Save"
          onCancel={() => setAsking(null)}
          onOk={(name) => {
            setAsking(null);
            void desk.saveTemplate(name).then((t) => {
              if (t) download(t, fileNameFor(name, "template"));
            });
          }}
        />
      ) : null}
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
