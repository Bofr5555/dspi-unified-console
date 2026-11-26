// WP P1WB1: the Snapshots screen. Capture holds the device as a snapshot document; Open Snapshot…
// adds one from a file. -- WP UB14 -- (B14, D14) They are kept in this browser per DSPi (the
// desk's store), so a reload finds them; Capture no longer downloads, Export… in the row's menu
// does; a line under the heading says where they are kept. Each held snapshot can be recalled
// (with rollback when the user asks, always over DSPi Link to a hub with the guard), compared
// with the device or with another (A/B), exported or deleted. Read only: capture and export only.
//
// -- WP P1DCFIX -- Simple: the list with Recall and the A/B compare only; no capture, open,
// download, remove, rollback choice or rollback time (those are Advanced's and Installer's).

import { useId, useRef, useState } from "preact/hooks";
import { Popover } from "../../ui/Popover"; // -- WP INTD --
import type { ScreenProps } from "../../ui/screens";
import { OutcomeLine, useDesk } from "./Bars";
import { download, fileNameFor, type HeldDoc, type SnapshotDesk } from "./model";
import { DiffTable } from "./DiffTable";
import { abKeyHint } from "../../ui/keys";
import { HubSnapshots } from "./HubSnapshots"; // -- WP P2WB2 --
import { EmptyState, FormRow, Glyph, Segments, Switch } from "../../ui/design"; // -- WP WEB -- (-- WP WEB7 -- FormRow, Glyph, Segments, Switch)
// -- WP WEB7 -- Qt's Modern page: Import… and Capture… in the page's tools, the rows as ListRows with
// Recall on hover and the rest in the row's menu, Recall with rollback as a switch
import { PageTools } from "../../ui/shell/Parts";
import { ActionMenuButton } from "../../ui/menus/ActionMenu";
import { ownRow, type MenuItem } from "../../ui/menus/actions";
import { timeWords } from "../history/model";
import "./rows.css";
import "./snapshots.css"; // -- WP WEB4 --

export function SnapshotsScreen(props: ScreenProps) {
  const desk = props.desk;
  if (!desk) return null;
  return <SnapshotsView desk={desk} ask={props.ask} simple={props.mode === "simple"} />;
}

/** -- WP INTD -- Capture's popover (Qt's): Name, Note, Capture. */
function CaptureForm(props: { busy: boolean; onCapture: (name: string, note: string) => void; close: () => void }) {
  const [name, setName] = useState("");
  const [note, setNote] = useState("");
  return (
    <form
      class="snap-capture"
      onSubmit={(e) => {
        e.preventDefault();
        props.close();
        props.onCapture(name, note);
      }}
    >
      <label>
        Name
        <input name="snapshot-name" value={name} placeholder="Snapshot" onInput={(e) => setName((e.target as HTMLInputElement).value)} />
      </label>
      <label>
        Note
        <input name="snapshot-note" value={note} placeholder="Note (optional)" onInput={(e) => setNote((e.target as HTMLInputElement).value)} />
      </label>
      {/* -- WP WEB7 -- (REVE) the opener is the accent; the popover's own button is not a second one */}
      <button type="submit" disabled={props.busy}>
        Capture
      </button>
    </form>
  );
}

function SnapshotsView(props: { desk: SnapshotDesk; ask: ScreenProps["ask"]; simple: boolean }) {
  const { desk, ask, simple } = props;
  const st = useDesk(desk);
  const session = desk.session;
  const ro = desk.readOnly;
  const file = useRef<HTMLInputElement>(null);
  const busy = !!st.busy;
  const waiting = st.guard?.phase === "waiting";
  const timeoutId = useId(); // -- WP WEB4 --
  const [selected, setSelected] = useState<number | null>(null);

  const capture = async (name: string, note: string) => {
    const preset = session.state.snapshot?.state.presetNames[session.state.snapshot.state.activePresetSlot] ?? "";
    const label = name.trim() || preset.trim() || "Snapshot";
    await desk.capture(label, note.trim()); // -- WP UB14 -- kept here; Export… downloads
  };

  const open = async (input: HTMLInputElement) => {
    const files = Array.from(input.files ?? []);
    input.value = "";
    for (const f of files) await desk.addFile(await f.text(), f.name);
  };

  const recall = async (d: HeldDoc) => {
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
    // -- WP WEB7 -- the switch (Qt's) says whether a recall is guarded; over DSPi Link it always is
    await desk.recall(d.id, st.rollback && !simple);
  };

  const remove = async (d: HeldDoc) => {
    const a = await ask({
      title: "Delete Snapshot",
      body: <p>Delete the snapshot “{d.name}”? It cannot be brought back.</p>,
      buttons: [
        { label: "Delete", value: "delete", danger: true },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (a === "delete") desk.remove(d.id);
  };

  /** The row's menu: what Qt's sheet offers for the selected snapshot (Simple: compare only). */
  const rowMenu = (d: HeldDoc): MenuItem[] => {
    const items: MenuItem[] = [
      ownRow("snap.compare", "Compare with Current", () => void desk.abStart(d.id, null), {
        disabled: !!ro || busy || waiting || st.ab.active,
        reason: ro || (st.ab.active ? "A compare runs" : ""),
      }),
    ];
    if (simple) return items;
    return [
      ...items,
      ownRow("snap.export", "Export…", () => download(d.text, d.fileName.endsWith(".dspipreset") ? d.fileName : fileNameFor(d.name))),
      "separator",
      ownRow("snap.delete", "Delete…", () => void remove(d), { danger: true }),
    ];
  };

  const capturePopover = (primary: boolean) => (
    <Popover
      label="Capture Snapshot"
      button={
        <>
          <PlusIcon />
          Capture…
        </>
      }
      buttonLabel="Capture…"
      buttonClass={`tool-icon-button${primary ? " primary" : ""}`}
      disabled={busy}
      align="right"
      title="Keep the device's settings as they are now"
    >
      {(close) => <CaptureForm busy={busy} close={close} onCapture={(n, no) => void capture(n, no)} />}
    </Popover>
  );

  // -- WP WEB4 -- DL3a's Snapshots; -- WP WEB7 -- as Qt Modern's page: the A/B compare first (its row
  // always there), the snapshots as ListRows (an empty state with Capture… when there are none), the
  // recall's rollback switch and time, the hub's snapshots last.
  return (
    <section class="snapshots" aria-label="Snapshots">
      {simple ? null : (
        <PageTools label="Snapshots tools">
          <button type="button" class="tool-icon-button" disabled={busy} title="Keep a configuration file as a snapshot of this device" onClick={() => file.current?.click()}>
            <Glyph name="file-up" />
            Import…
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
          {st.docs.length ? capturePopover(true) : null}
        </PageTools>
      )}
      <OutcomeLine desk={desk} outcome={st.outcome} />
      <Compare desk={desk} st={st} />
      <div class="snap-sec" role="group" aria-label="Snapshots list">
        <div class="ds-section-head">
          <h4 class="section-label">Snapshots</h4>
        </div>
        {/* -- WP UB14 -- where they are kept (Simple has no Export…) */}
        {!simple || st.docs.length ? <p class="snap-caption">{keptWords(st.kept, simple)}</p> : null}
        {st.guardDefault ? <p class="snap-caption">Every recall here can be taken back: it is kept only when you press Keep within {st.timeoutS} s.</p> : null}
        {/* -- WP WEB -- DL3a's empty state (-- WP WEB7 -- with Capture… as Qt's, the page's one accent) */}
        {st.docs.length === 0 ? (
          <EmptyState
            icon="save"
            title="No snapshots yet"
            text={simple ? "Snapshots are captured in Advanced mode." : "A snapshot keeps every setting of the device as it is now, to recall or compare later."}
          >
            {simple ? null : capturePopover(true)}
          </EmptyState>
        ) : null}
        <ul class="lib-rows snap-rows" aria-label="Held snapshots" hidden={st.docs.length === 0}>
          {st.docs.map((d) => (
            <li
              key={d.id}
              class={`lib-row${selected === d.id ? " selected" : ""}`}
              aria-label={d.name}
              onClick={(e) => (e.target as HTMLElement).closest("button, .menu") || setSelected(d.id)}
              onDblClick={(e) => (e.target as HTMLElement).closest("button, .menu") || ro || busy || waiting || void recall(d)}
            >
              <span class="lib-text">
                <span class="lib-title">
                  {d.name}
                  {d.kind !== "snapshot" ? <span class="badge">{d.kind}</span> : null}
                </span>
                {d.note ? <span class="lib-sub">{d.note}</span> : null}
              </span>
              <span class="lib-value">{d.savedUtc ? when(d.savedUtc) : d.fileName}</span>
              <span class="lib-actions">
                <button type="button" disabled={!!ro || busy || waiting} title="Put the device back to this snapshot" onClick={() => void recall(d)}>
                  Recall
                </button>
                <ActionMenuButton label={`More, ${d.name}`} items={() => rowMenu(d)}>
                  <span class="visually-hidden">More, {d.name}</span>
                </ActionMenuButton>
              </span>
            </li>
          ))}
        </ul>
      </div>
      {/* -- WP P1DCFIX -- the rollback time in Advanced and Installer, over DSPi Link too (5..300 s, 30 by default) */}
      {/* -- WP WEB7 -- with Qt's switch "Recall with rollback" over it, and Qt's stepper */}
      {simple ? null : (
        <div class="snap-sec" role="group" aria-label="Recall">
          <h4 class="section-label">Recall</h4>
          <FormRow title="Recall with rollback" caption={st.guardDefault ? "Always over DSPi Link: Keep or Revert after each recall" : "Keep or Revert after each recall"}>
            <Switch label="Recall with rollback" on={st.guardDefault || st.rollback} disabled={st.guardDefault} toggle={() => desk.setRollback(!st.rollback)} />
          </FormRow>
          <FormRow title={<label for={timeoutId}>Rollback after</label>}>
            <span class="snap-stepper">
              <button type="button" aria-label="Rollback after: less" disabled={st.timeoutS <= 5} onClick={() => desk.setTimeoutS(st.timeoutS - 5)}>
                −
              </button>
              <input
                id={timeoutId}
                class="snap-seconds"
                type="text"
                inputMode="numeric"
                aria-label="Rollback after, seconds"
                value={String(st.timeoutS)}
                onChange={(e) => {
                  const el = e.target as HTMLInputElement;
                  const v = Number(el.value.trim());
                  if (Number.isFinite(v) && el.value.trim() !== "") desk.setTimeoutS(v);
                  el.value = String(desk.state.timeoutS);
                }}
              />
              <span class="snap-unit">s</span>
              <button type="button" aria-label="Rollback after: more" disabled={st.timeoutS >= 300} onClick={() => desk.setTimeoutS(st.timeoutS + 5)}>
                +
              </button>
            </span>
          </FormRow>
        </div>
      )}
      {/* -- WP P2WB2 -- the hub's shared snapshots, when the hub keeps them */}
      <HubSnapshots desk={desk} ask={ask} simple={simple} />
    </section>
  );
}

function PlusIcon() {
  return (
    <svg class="shell-icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <path d="M5 12h14M12 5v14" />
    </svg>
  );
}

/**
 * A/B: Qt's row always there (A: Current settings or a snapshot, B: a snapshot or "No snapshot yet",
 * Start); while a compare runs, which side is on, Toggle A/B, Keep A and Keep B, the differences.
 */
function Compare(props: { desk: SnapshotDesk; st: ReturnType<typeof useDesk> }) {
  const { desk, st } = props;
  const [a, setA] = useState("now");
  const [b, setB] = useState("");
  const ro = !!desk.readOnly;
  const waiting = st.guard?.phase === "waiting";
  const none = st.docs.length === 0;
  const bId = b && st.docs.some((d) => String(d.id) === b) ? Number(b) : (st.docs[0]?.id ?? 0);
  const same = a !== "now" && Number(a) === bId;
  const off = st.busy !== "" || waiting || ro;
  return (
    <div class="snap-sec" role="region" aria-label="A/B compare">
      <h4 class="section-label">A/B Compare</h4>
      {st.ab.active ? (
        <>
          <div class="snap-ab-run">
            <Segments
              label="On the device now"
              options={[
                { value: "a", text: sideName("A", st.ab.aName) },
                { value: "b", text: sideName("B", st.ab.bName) },
              ]}
              value={st.ab.side === "b" ? "b" : "a"}
              choose={(v) => v !== st.ab.side && void desk.abToggle()}
              disabled={off}
            />
            <button type="button" disabled={off} aria-keyshortcuts={abKeyHint().aria} title={waiting ? "Finish the safe apply first: Keep or Revert." : `Recall the other side (${abKeyHint().text})`} onClick={() => void desk.abToggle()}>
              Toggle A/B
            </button>
            <button type="button" disabled={off} onClick={() => void desk.abKeep("a")}>
              Keep A
            </button>
            <button type="button" disabled={off} onClick={() => void desk.abKeep("b")}>
              Keep B
            </button>
            <button type="button" disabled={st.busy !== ""} onClick={() => void desk.abDiff()}>
              Show Differences
            </button>
          </div>
          {waiting ? <p class="snap-caption">Finish the safe apply first: Keep or Revert.</p> : null}
          {st.abDiff ? <DiffTable diff={st.abDiff} label={`From A to B`} /> : null}
        </>
      ) : (
        <form
          class="snap-compare"
          onSubmit={(e) => {
            e.preventDefault();
            if (!none && !same) void desk.abStart(bId, a === "now" ? null : Number(a));
          }}
        >
          <label>
            <span class="snap-side">A</span>
            <select name="ab-a" aria-label="A side" value={a} onChange={(e) => setA((e.target as HTMLSelectElement).value)}>
              <option value="now">Current settings</option>
              {st.docs.map((d) => (
                <option key={d.id} value={String(d.id)}>
                  {d.name}
                </option>
              ))}
            </select>
          </label>
          <label>
            <span class="snap-side">B</span>
            <select name="ab-b" aria-label="B side" value={none ? "" : String(bId)} disabled={none} onChange={(e) => setB((e.target as HTMLSelectElement).value)}>
              {none ? <option value="">No snapshot yet</option> : null}
              {st.docs.map((d) => (
                <option key={d.id} value={String(d.id)}>
                  {d.name}
                </option>
              ))}
            </select>
          </label>
          <button type="submit" disabled={ro || st.busy !== "" || waiting || none || same}>
            Start
          </button>
          <p class="snap-caption">Recall two states in turn and hear the difference; Keep ends the compare on one of them.</p>
        </form>
      )}
    </div>
  );
}

/** -- WP UB14 -- where the held snapshots are, in a line under the heading. */
export function keptWords(kept: "browser" | "page", simple: boolean): string {
  const where = kept === "browser" ? "Kept in this browser for this DSPi." : "Kept until you close this page: this browser keeps no site data here.";
  return simple ? where : `${where} Export… saves one as a file.`;
}

/** -- WP WEB7 -- a compare's side as Qt names it: "B: Evening", or just "A" when that is its name. */
function sideName(side: string, name: string): string {
  return name && name !== side ? `${side}: ${name}` : side;
}

/** -- WP WEB7 -- when a snapshot was saved: the time today, the date and time before (Qt's value column). */
function when(utc: string): string {
  const t = Date.parse(utc);
  return Number.isNaN(t) ? utc : timeWords(t, Date.now());
}
