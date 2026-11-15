// WP P2WB2: the Snapshots screen's "On the hub" section, shown when the session's hub keeps snapshots
// (`hubCaps().hubSnapshots`). The hub's snapshots of the open DSPi as the core merges them
// (`snapshotListMerged`, each row's `where` the core's word; the hub's copy wins on a differing id).
//
// - Save to the hub: the device now (captured) or a held document (`hubSnapshotsPut`, Control and
//   Admin; over 61 440 bytes compact the core refuses it in its words and sends nothing).
// - Recall: the hub recalls it (WP P2WB3); where this session's recalls are guarded (DSPi Link to a
//   hub with the guard) the hub guards it and the core adopts the guard: the bar's Keep or Revert.
// - The list follows the hub's `snapshot.changed` (WP P2WB3), whichever console changed it.
// - Copy here: the hub's document held in this visit's list (download it from there).
// - Rename (Control, Admin), Delete (Admin): the hub refuses below in its words.

import { useEffect, useState } from "preact/hooks";
import type { HubCapsRecord, MergedSnapshotRecord, SnapshotMergedListRecord } from "../../core/hub-settings-protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { noticeRow } from "../../ui/texts";
import { useDesk } from "./Bars";
import { SAVE_DEVICE, hubFailure, rowNote, whereLabel } from "./hub";
import { fileNameFor, type SnapshotDesk } from "./model";

export function HubSnapshots(props: { desk: SnapshotDesk; ask: ScreenProps["ask"]; simple: boolean }) {
  const { desk, ask, simple } = props;
  const session = desk.session;
  const st = useDesk(desk);
  const sst = useSessionState(session);
  const phase = sst.link?.phase ?? "";
  const serial = sst.snapshot?.device.serial || session.deviceId || "";
  const [caps, setCaps] = useState<HubCapsRecord | null>(null);
  const [list, setList] = useState<SnapshotMergedListRecord | null>(null);
  const [problem, setProblem] = useState("");
  const [done, setDone] = useState("");
  const [source, setSource] = useState(SAVE_DEVICE);
  const [busy, setBusy] = useState(false);
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  const read = async () => {
    const c = await session.core.call("hubCaps", undefined).catch(() => null);
    setCaps(c);
    if (!c?.hubSnapshots) return;
    try {
      setList(await session.core.call("snapshotListMerged", serial ? { serial } : {}));
    } catch (e) {
      setProblem(hubFailure(e));
    }
  };
  useEffect(() => {
    void read();
  }, [session, phase, serial]);
  // -- WP P2WB3 -- the hub's `snapshot.changed` (any console's put, rename, note, delete): this DSPi's list again
  useEffect(
    () =>
      session.watchHub((events) => {
        if (events.some((e) => e.kind === "snapshot.changed" && (!serial || !e.serial || e.serial === serial))) void read();
      }),
    [session, serial],
  );
  if (!caps?.hubSnapshots) return null;

  const act = async (f: () => Promise<string>) => {
    setBusy(true);
    try {
      setDone(await f());
      setProblem("");
    } catch (e) {
      setDone("");
      setProblem(hubFailure(e));
    } finally {
      setBusy(false);
      await read();
    }
  };
  const save = () =>
    act(async () => {
      let text: string;
      if (source === SAVE_DEVICE) {
        const preset = sst.snapshot?.state.presetNames[sst.snapshot.state.activePresetSlot] ?? "";
        text = (await session.core.call("snapshotCapture", { name: preset.trim() || "Snapshot", note: "" })).text;
      } else {
        const d = desk.doc(Number(source));
        if (!d) return "";
        text = d.text;
      }
      const r = await session.core.call("hubSnapshotsPut", { ...(serial ? { serial } : {}), text });
      return r.created ? `Saved “${r.snapshot.name}” to the hub.` : `The hub already holds “${r.snapshot.name}”.`;
    });
  const recall = async (r: MergedSnapshotRecord) => {
    if (session.dirty) {
      const a = await ask({
        title: "Recall Snapshot",
        body: <p>The current settings have unsaved changes. Recalling “{r.snapshot.name}” replaces them.</p>,
        buttons: [
          { label: "Recall", value: "recall" },
          { label: "Cancel", value: "cancel" },
        ],
      });
      if (a !== "recall") return;
    }
    setProblem("");
    setDone("");
    // -- WP P2WB3 -- recalled on the hub; guarded, the hub's own guard (the core adopts it: the bar's Keep or Revert)
    const id = r.snapshot.id;
    await desk.recallOnHub(
      async (guard, timeoutMs) => {
        const x = await session.core.call("hubSnapshotsRecall", { ...(serial ? { serial } : {}), id, ...(guard ? { guard, timeoutMs } : {}) });
        await session.refresh();
        return x;
      },
      async () => (await session.core.call("hubSnapshotsGet", { id })).document,
    );
  };
  const copyHere = (r: MergedSnapshotRecord) =>
    act(async () => {
      const doc = await session.core.call("hubSnapshotsGet", { id: r.snapshot.id });
      const held = await desk.addFile(doc.document, fileNameFor(doc.snapshot.name || r.snapshot.name));
      return held ? `“${held.name}” is in this visit's list: download it from there.` : "";
    });
  const rename = (r: MergedSnapshotRecord, name: string) =>
    act(async () => {
      setRenaming(null);
      const x = await session.core.call("hubSnapshotsRename", { id: r.snapshot.id, name: name.trim() });
      return `Renamed to “${x.name}” on the hub.`;
    });
  const remove = async (r: MergedSnapshotRecord) => {
    const a = await ask({
      title: "Delete from the Hub",
      body: <p>Delete “{r.snapshot.name}” from the hub? Every console on the hub loses it.</p>,
      buttons: [
        { label: "Delete", value: "delete", danger: true },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (a !== "delete") return;
    await act(async () => {
      await session.core.call("hubSnapshotsDelete", { id: r.snapshot.id });
      return `Deleted “${r.snapshot.name}” from the hub.`;
    });
  };
  const ro = !!desk.readOnly;
  const waiting = st.guard?.phase === "waiting";
  const off = busy || !!st.busy || waiting;
  const why = list && !list.hubListed && list.hubNoticeKey ? noticeRow(list.hubNoticeKey) : null;
  const rows = list?.entries ?? [];
  return (
    <div class="card hub-snapshots" role="region" aria-label="On the hub">
      <h3>On the hub</h3>
      <p class="note">Snapshots kept on the hub are shared by every console on it. Where an id is on both sides with different content, the hub's copy is the one shown.</p>
      {why ? (
        <p class="note warn" role="note" data-key={why.key}>
          {why.title}: {why.reason}
        </p>
      ) : null}
      {problem ? (
        <p class="error" role="alert">
          {problem}
        </p>
      ) : null}
      {done ? (
        <p class="note hub-done" role="status">
          {done}
        </p>
      ) : null}
      {rows.length === 0 ? <p class="note">No snapshots on the hub for this DSPi.</p> : null}
      <ul class="snap-list" aria-label="Snapshots on the hub">
        {rows.map((r) => (
          <li key={`${r.snapshot.id}-${r.where}-${r.localCopy}`} class="snap-row" aria-label={r.snapshot.name} data-where={r.where}>
            <div class="snap-what">
              <strong>{r.snapshot.name}</strong> <span class="badge">{whereLabel(r.where)}</span>
              <span class="note"> {rowNote(r)}</span>
            </div>
            <div class="snap-actions">
              <button type="button" disabled={ro || off} onClick={() => void recall(r)}>
                Recall
              </button>
              {simple ? null : (
                <>
                  <button type="button" disabled={off} onClick={() => void copyHere(r)}>
                    Copy Here
                  </button>
                  <button type="button" disabled={ro || off} aria-label={`Rename ${r.snapshot.name} on the hub`} onClick={() => setRenaming({ id: r.snapshot.id, name: r.snapshot.name })}>
                    Rename…
                  </button>
                  <button type="button" class="danger-text" disabled={ro || off} aria-label={`Delete ${r.snapshot.name} from the hub`} onClick={() => void remove(r)}>
                    Delete
                  </button>
                </>
              )}
            </div>
            {renaming && renaming.id === r.snapshot.id && !r.localCopy ? (
              <form
                class="snap-capture"
                onSubmit={(e) => {
                  e.preventDefault();
                  if (renaming.name.trim()) void rename(r, renaming.name);
                }}
              >
                <label>
                  New name
                  <input value={renaming.name} maxLength={64} onInput={(e) => setRenaming({ id: r.snapshot.id, name: (e.target as HTMLInputElement).value })} />
                </label>
                <div class="snap-actions">
                  <button type="submit" disabled={off || !renaming.name.trim()}>
                    Rename
                  </button>
                  <button type="button" onClick={() => setRenaming(null)}>
                    Cancel
                  </button>
                </div>
              </form>
            ) : null}
          </li>
        ))}
      </ul>
      {simple ? null : (
        <form
          class="snap-compare"
          onSubmit={(e) => {
            e.preventDefault();
            void save();
          }}
        >
          <label>
            Save to the hub
            <select name="hub-save-source" aria-label="What to save to the hub" value={source} onChange={(e) => setSource((e.target as HTMLSelectElement).value)}>
              <option value={SAVE_DEVICE}>The DSPi now</option>
              {st.docs.map((d) => (
                <option key={d.id} value={String(d.id)}>
                  {d.name}
                </option>
              ))}
            </select>
          </label>
          <button type="submit" disabled={ro || off}>
            Save to the Hub
          </button>
        </form>
      )}
    </div>
  );
}
