// WP P1WB1: the Snapshots screen. Capture downloads the device as a snapshot document and
// holds it for this visit; Open Snapshot… adds one from a file (OD-P1b: on the web the
// browser's downloads are the store). Each held snapshot can be recalled (with rollback when
// the user asks, always over DSPi Link to a hub with the guard), compared with the device or
// with another (A/B), downloaded again or let go. Read only: capture and download only.
//
// -- WP P1DCFIX -- Simple: the list with Recall and the A/B compare only; no capture, open,
// download, remove, rollback choice or rollback time (those are Advanced's and Installer's).

import { useRef, useState } from "preact/hooks";
import type { ScreenProps } from "../../ui/screens";
import { OutcomeLine, useDesk } from "./Bars";
import { download, fileNameFor, type HeldDoc, type SnapshotDesk } from "./model";
import { DiffTable } from "./DiffTable";
import { abKeyHint } from "../../ui/keys";
import { HubSnapshots } from "./HubSnapshots"; // -- WP P2WB2 --

export function SnapshotsScreen(props: ScreenProps) {
  const desk = props.desk;
  if (!desk) return null;
  return <SnapshotsView desk={desk} ask={props.ask} simple={props.mode === "simple"} />;
}

function SnapshotsView(props: { desk: SnapshotDesk; ask: ScreenProps["ask"]; simple: boolean }) {
  const { desk, ask, simple } = props;
  const st = useDesk(desk);
  const session = desk.session;
  const ro = desk.readOnly;
  const [name, setName] = useState("");
  const [note, setNote] = useState("");
  const file = useRef<HTMLInputElement>(null);
  const busy = !!st.busy;
  const waiting = st.guard?.phase === "waiting";

  const capture = async () => {
    const preset = session.state.snapshot?.state.presetNames[session.state.snapshot.state.activePresetSlot] ?? "";
    const label = name.trim() || preset.trim() || "Snapshot";
    const doc = await desk.capture(label, note.trim());
    if (!doc) return;
    download(doc.text, fileNameFor(doc.name));
    setName("");
    setNote("");
  };

  const open = async (input: HTMLInputElement) => {
    const files = Array.from(input.files ?? []);
    input.value = "";
    for (const f of files) await desk.addFile(await f.text(), f.name);
  };

  const recall = async (d: HeldDoc, guarded: boolean) => {
    if (session.dirty) {
      const a = await ask({
        title: "Recall Snapshot",
        body: <p>The current settings have unsaved changes. Recalling “{d.name}” replaces them.</p>,
        buttons: [
          { label: "Recall", value: "recall" },
          { label: "Cancel", value: "cancel" },
        ],
      });
      if (a !== "recall") return;
    }
    await desk.recall(d.id, guarded);
  };

  return (
    <section class="snapshots" aria-label="Snapshots">
      <OutcomeLine desk={desk} outcome={st.outcome} />
      {simple ? null : (
      <div class="card">
        <h3>Capture</h3>
        <p class="note">
          A snapshot is the whole device: every setting, saved as a file. Capture downloads one; Open Snapshot… brings one
          back. The list below holds this visit's snapshots.
        </p>
        <form
          class="snap-capture"
          onSubmit={(e) => {
            e.preventDefault();
            void capture();
          }}
        >
          <label>
            Name
            <input name="snapshot-name" value={name} placeholder="Snapshot" onInput={(e) => setName((e.target as HTMLInputElement).value)} />
          </label>
          <label>
            Note
            <input name="snapshot-note" value={note} onInput={(e) => setNote((e.target as HTMLInputElement).value)} />
          </label>
          <div class="snap-actions">
            <button type="submit" disabled={busy}>
              Capture
            </button>
            <button type="button" disabled={busy} onClick={() => file.current?.click()}>
              Open Snapshot…
            </button>
            <input
              ref={file}
              type="file"
              hidden
              multiple
              accept=".dspipreset,.json,application/json"
              aria-label="Snapshot file"
              onChange={(e) => void open(e.target as HTMLInputElement)}
            />
          </div>
        </form>
      </div>
      )}
      <div class="card">
        <h3>Snapshots</h3>
        {st.guardDefault ? (
          <p class="note">Every recall here can be taken back: it is kept only when you press Keep within {st.timeoutS} s.</p>
        ) : null}
        {st.docs.length === 0 ? (
          <p class="note">{simple ? "No snapshots yet. Snapshots captured or opened in Advanced mode are listed here." : "No snapshots yet."}</p>
        ) : null}
        <ul class="snap-list" aria-label="Held snapshots">
          {st.docs.map((d) => (
            <li key={d.id} class="snap-row" aria-label={d.name}>
              <div class="snap-what">
                <strong>{d.name}</strong>
                {d.kind !== "snapshot" ? <span class="badge">{d.kind}</span> : null}
                <span class="note">
                  {d.savedUtc ? when(d.savedUtc) : d.fileName}
                  {d.note ? ` · ${d.note}` : ""}
                </span>
              </div>
              <div class="snap-actions">
                <button type="button" disabled={!!ro || busy || waiting} onClick={() => void recall(d, false)}>
                  Recall
                </button>
                {st.guardDefault || simple ? null : (
                  <button type="button" disabled={!!ro || busy || waiting} onClick={() => void recall(d, true)}>
                    Recall with Rollback
                  </button>
                )}
                <button type="button" disabled={!!ro || busy || waiting || st.ab.active} onClick={() => void desk.abStart(d.id, null)}>
                  Compare
                </button>
                {simple ? null : (
                  <>
                    <button type="button" onClick={() => download(d.text, d.fileName.endsWith(".dspipreset") ? d.fileName : fileNameFor(d.name))}>
                      Download
                    </button>
                    <button type="button" class="danger-text" aria-label={`Remove ${d.name} from the list`} onClick={() => desk.remove(d.id)}>
                      Remove
                    </button>
                  </>
                )}
              </div>
            </li>
          ))}
        </ul>
        {/* -- WP P1DCFIX -- the rollback time in Advanced and Installer, over DSPi Link too (5..300 s, 30 by default) */}
        {simple ? null : (
          <label class="snap-timeout">
            Rollback time (s)
            <input
              type="number"
              min={5}
              max={300}
              step={5}
              value={st.timeoutS}
              onChange={(e) => desk.setTimeoutS(Number((e.target as HTMLInputElement).value))}
            />
          </label>
        )}
      </div>
      {/* -- WP P2WB2 -- the hub's shared snapshots, when the hub keeps them */}
      <HubSnapshots desk={desk} ask={ask} simple={simple} />
      <Compare desk={desk} st={st} />
    </section>
  );
}

/** A/B: start a compare of two held snapshots (or the device and one), its differences. */
function Compare(props: { desk: SnapshotDesk; st: ReturnType<typeof useDesk> }) {
  const { desk, st } = props;
  const [a, setA] = useState("now");
  const [b, setB] = useState("");
  const ro = !!desk.readOnly;
  const waiting = st.guard?.phase === "waiting";
  const bId = b ? Number(b) : (st.docs[0]?.id ?? 0);
  return (
    <div class="card" role="region" aria-label="A/B compare">
      <h3>A/B compare</h3>
      {st.ab.active ? (
        <>
          <p class="note">
            On the device now: <strong>{st.ab.side === "b" ? `B (${st.ab.bName})` : `A (${st.ab.aName})`}</strong>. Switch with the
            A/B button above the screens or {abKeyHint().text}; edits on a side are kept until you switch back.
          </p>
          <div class="snap-actions">
            <button type="button" disabled={st.busy !== "" || waiting || ro} onClick={() => void desk.abToggle()}>
              Switch to {st.ab.side === "b" ? "A" : "B"}
            </button>
            <button type="button" disabled={st.busy !== ""} onClick={() => void desk.abDiff()}>
              Show Differences
            </button>
          </div>
          {st.abDiff ? <DiffTable diff={st.abDiff} label={`From A to B`} /> : null}
        </>
      ) : st.docs.length === 0 ? (
        <p class="note">Capture or open a snapshot to compare it with the device.</p>
      ) : (
        <form
          class="snap-compare"
          onSubmit={(e) => {
            e.preventDefault();
            void desk.abStart(bId, a === "now" ? null : Number(a));
          }}
        >
          <label>
            A
            <select name="ab-a" value={a} onChange={(e) => setA((e.target as HTMLSelectElement).value)}>
              <option value="now">The device now</option>
              {st.docs.map((d) => (
                <option key={d.id} value={String(d.id)}>
                  {d.name}
                </option>
              ))}
            </select>
          </label>
          <label>
            B
            <select name="ab-b" value={String(bId)} onChange={(e) => setB((e.target as HTMLSelectElement).value)}>
              {st.docs.map((d) => (
                <option key={d.id} value={String(d.id)}>
                  {d.name}
                </option>
              ))}
            </select>
          </label>
          <button type="submit" disabled={ro || st.busy !== "" || waiting}>
            Start Compare
          </button>
        </form>
      )}
    </div>
  );
}

function when(utc: string): string {
  const t = Date.parse(utc);
  return Number.isNaN(t) ? utc : new Date(t).toLocaleString();
}
