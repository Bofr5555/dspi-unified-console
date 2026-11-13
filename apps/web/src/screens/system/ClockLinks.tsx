// WP SW3: the Installer mode's "Clock and links" panel (SYS2 section 7). Per
// unit the link delay (`d_local`) against its limits, the hop offset (`hops_offset`), the
// latency after the link (measured or configured) and the lip sync of its
// room; the slot matrix (the sender's link channels to each unit's 8 matrix
// inputs); the alignment (`system.align`) with each unit's result; the link
// stats and the trim scan. Everything goes through the hub's `system.*`; the
// model and the alignment are admin-only, so a Control or Viewer role sees the
// panel read only (docs/UI-MODES.md section 4).

import { useState } from "preact/hooks";
import type { Ask } from "../../ui/screens";
import type { SystemFeed } from "./feed";
import { useFeed } from "./SystemScreen";
import {
  classWord,
  framesMs,
  mayWrite,
  parseWhole,
  rateOf,
  resultTone,
  resultWord,
  SILENT,
  senderChannels,
  slotWord,
  sourceOf,
  subjectName,
  writeWhat,
  UNIT_INPUTS,
  unitSlots,
  withLinkDelay,
  withMeasuredLatency,
  withRoomLipSync,
  withSlot,
  writeNote,
  type SystemModel,
  type WriteResult,
} from "./model";

export function ClockLinks(props: { feed: SystemFeed; ask: Ask }) {
  const st = useFeed(props.feed);
  const model = st.model!;
  const admin = mayWrite(st.role, "model") && !props.feed.readOnly;
  // Edits to the model are staged here until Save sends the whole model.
  // -- fixup sw3x -- with the revision the edits started from: Save sends it, and
  // the hub refuses the model if someone changed the installation meanwhile.
  const [draft, setDraft] = useState<{ model: SystemModel; base: number } | null>(null);
  const [conflict, setConflict] = useState(false);
  const shown = draft?.model ?? model;
  const edit = (m: SystemModel) => {
    setDraft({ model: m, base: draft?.base ?? st.revision });
    setConflict(false);
  };
  const stale = !!draft && (conflict || draft.base !== st.revision);
  const save = async () => {
    if (!draft || stale) return;
    const r = await props.feed.setModel(draft.model, draft.base);
    if (r === "ok") setDraft(null);
    if (r === "conflict") setConflict(true);
  };

  return (
    <section class="sys-clock" aria-label="Clock and links">
      <h3>Clock and links</h3>
      {admin ? null : <p class="sys-note">{writeNote(st.role, "model")}: this panel is read only.</p>}
      <UnitsTable feed={props.feed} model={shown} admin={admin} onEdit={edit} />
      <SlotMatrix model={shown} admin={admin} onEdit={edit} />
      {draft && stale ? (
        <div class="sys-save sys-stale" role="alert" aria-label="Installation changed on the hub">
          <span>
            The installation changed on the hub while you edited it. Saving now would undo that change, so it is not offered:
            reload the hub's installation and make your changes again.
          </span>
          <button
            type="button"
            onClick={() => {
              setDraft(null);
              setConflict(false);
              void props.feed.reloadModel();
            }}
          >
            Reload
          </button>
        </div>
      ) : draft ? (
        <div class="sys-save" role="group" aria-label="Unsaved installation">
          <span>The installation has unsaved changes. Save sends them to the hub; Align writes them to the units.</span>
          <button type="button" onClick={() => void save()}>
            Save
          </button>
          <button type="button" onClick={() => setDraft(null)}>
            Revert
          </button>
        </div>
      ) : null}
      <AlignPanel feed={props.feed} ask={props.ask} admin={admin && !draft} mayAlign={admin} />
      <StatsTable feed={props.feed} />
    </section>
  );
}

/** A number field committed on Enter or blur; invalid text shows the field as such and is not sent. */
function NumberCell(props: { label: string; value: number | null | undefined; lo: number; hi: number; admin: boolean; placeholder?: string; onCommit: (v: number | null) => void }) {
  const [text, setText] = useState<string | null>(null);
  const shown = text ?? (props.value == null ? "" : String(props.value));
  const parsed = parseWhole(shown, props.lo, props.hi);
  if (!props.admin) return <span>{props.value == null ? (props.placeholder ?? "–") : props.value}</span>;
  const commit = () => {
    if (text === null) return;
    if (parsed !== undefined) props.onCommit(parsed);
    setText(null);
  };
  return (
    <input
      class="sys-num"
      aria-label={props.label}
      aria-invalid={parsed === undefined}
      inputMode="numeric"
      placeholder={props.placeholder}
      value={shown}
      onInput={(e) => setText((e.target as HTMLInputElement).value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
        if (e.key === "Escape") setText(null);
      }}
    />
  );
}

function UnitsTable(props: { feed: SystemFeed; model: SystemModel; admin: boolean; onEdit: (m: SystemModel) => void }) {
  const st = useFeed(props.feed);
  const { model } = props;
  const a = st.status?.alignment;
  const rate = a?.rateHz || model.rateHz || 48000;
  return (
    <>
      <div class="sys-row">
        <span>
          <abbr title="D: the delay every unit's link delay is worked out from">System delay</abbr>: <strong>{a ? framesMs(a.dFrames, rate) : "–"}</strong>{" "}
          {a?.dAuto ? "(the smallest every unit takes)" : ""}
        </span>
        <label class="sys-inline">
          Fixed system delay, frames
          <NumberCell
            label="Fixed system delay in frames"
            value={model.dFrames}
            lo={0}
            hi={65535}
            admin={props.admin}
            placeholder="auto"
            onCommit={(v) => props.onEdit(withLinkDelay(model, v))}
          />
        </label>
        <span>Rate {rate / 1000} kHz</span>
      </div>
      <div class="sys-scroll">
        <table class="sys-table" aria-label="Units on the audio link">
          <thead>
            <tr>
              <th>Unit</th>
              <th>Hops</th>
              <th>Path delay</th>
              <th>
                <abbr title="d_local (EXT_LINK_PHASE): frames from a link frame's arrival to its playing, ms beside">Link delay</abbr>
              </th>
              <th>
                <abbr title="d_local_min..d_local_max (EXT_LINK_INFO), in frames">Link delay range</abbr>
              </th>
              <th>
                <abbr title="hops_offset (EXT_LINK_PHASE): frames added for the boxes between the sender and this unit">Hop offset</abbr>
              </th>
              <th>Latency, µs</th>
              <th>Measured, µs</th>
              <th>Lip sync</th>
              <th>Effective</th>
            </tr>
          </thead>
          <tbody>
            {model.units.map((u) => {
              const plan = a?.units.find((p) => p.unit === u.id);
              const o = st.status?.observed[u.serial];
              const read = o?.phase;
              const issues = a?.issues.filter((i) => i.subject === u.id) ?? [];
              return (
                <tr key={u.id} aria-label={`Clock of ${u.name || u.id}`} data-writable={plan?.writable ?? false}>
                  <th scope="row">{u.name || u.id}</th>
                  <td>{plan?.hops ?? o?.hops ?? "–"}</td>
                  <td>{plan ? `${plan.pathDelayFrames} frames` : "–"}</td>
                  <td title={read ? `Read from the unit: ${read[1]}` : undefined}>
                    {plan ? framesMs(plan.dLocal, rate) : "–"}
                    {read && plan && read[1] !== plan.dLocal ? <span class="sys-diff"> (unit: {read[1]})</span> : null}
                  </td>
                  <td>{o?.dLocalMin != null ? `${o.dLocalMin}..${o.dLocalMax}` : "unknown"}</td>
                  <td>
                    {plan ? plan.hopsOffset : "–"}
                    {read && plan && read[2] !== plan.hopsOffset ? <span class="sys-diff"> (unit: {read[2]})</span> : null}
                  </td>
                  <td>{u.latencyUs ?? 0}</td>
                  <td>
                    <NumberCell
                      label={`Measured latency of ${u.name || u.id}`}
                      value={u.measuredLatencyUs}
                      lo={-100000}
                      hi={100000}
                      admin={props.admin}
                      onCommit={(v) => props.onEdit(withMeasuredLatency(model, u.id, v))}
                    />
                  </td>
                  <td>{plan ? framesMs(plan.lipSyncFrames, rate) : "–"}</td>
                  <td>
                    {plan ? `${plan.effective} frames` : "–"}
                    {issues.map((i) => (
                      <div class="sys-issue" key={i.code} data-code={i.code}>
                        {i.message}
                      </div>
                    ))}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <div class="sys-scroll">
        <table class="sys-table" aria-label="Lip sync by room">
          <thead>
            <tr>
              <th>Room</th>
              <th>Lip sync, µs</th>
            </tr>
          </thead>
          <tbody>
            {model.rooms.map((r) => (
              <tr key={r.id}>
                <th scope="row">{r.name || r.id}</th>
                <td>
                  <NumberCell
                    label={`Lip sync of ${r.name || r.id}`}
                    value={r.lipSyncUs}
                    lo={0}
                    hi={1000000}
                    admin={props.admin}
                    placeholder={`${model.lipSyncUs ?? 0} (installation)`}
                    onCommit={(v) => props.onEdit(withRoomLipSync(model, r.id, v))}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {a?.issues.filter((i) => !i.subject || !model.units.some((u) => u.id === i.subject)).map((i) => (
        <p class="sys-issue" key={`${i.code}-${i.subject ?? ""}`} data-code={i.code}>
          {i.message}
        </p>
      ))}
    </>
  );
}

/** The sender's link channels (rows) to each unit's matrix inputs (columns). One channel per input, or silent. */
function SlotMatrix(props: { model: SystemModel; admin: boolean; onEdit: (m: SystemModel) => void }) {
  const { model } = props;
  const channels = senderChannels(model);
  return (
    <>
      <h4>Link slots</h4>
      <div class="sys-scroll">
        <table class="sys-matrix" aria-label="Link slots">
          <thead>
            <tr>
              <th rowSpan={2}>Link channel</th>
              {model.units.map((u) => (
                <th key={u.id} colSpan={UNIT_INPUTS} class="sys-matrix-unit">
                  {u.name || u.id}
                </th>
              ))}
            </tr>
            <tr>
              {model.units.flatMap((u) =>
                Array.from({ length: UNIT_INPUTS }, (_, i) => (
                  <th key={`${u.id}-${i}`} class={i === 0 ? "sys-matrix-first" : undefined}>
                    {i + 1}
                  </th>
                )),
              )}
            </tr>
          </thead>
          <tbody>
            {channels.map((ch) => (
              <tr key={ch}>
                <th scope="row">
                  {slotWord(ch)}
                  {sourceOf(model, ch) ? <span class="sys-muted"> · {sourceOf(model, ch)!.name}</span> : null}
                </th>
                {model.units.flatMap((u) => {
                  const slots = unitSlots(u);
                  return slots.map((c, i) => {
                    const on = c === ch;
                    const label = `${u.name || u.id} IN${i + 1} from ${slotWord(ch)}`;
                    return (
                      <td key={`${u.id}-${i}`} class={i === 0 ? "sys-matrix-first" : undefined}>
                        <button
                          type="button"
                          class={`sys-cell${on ? " on" : ""}`}
                          aria-label={label}
                          aria-pressed={on}
                          disabled={!props.admin}
                          onClick={() => props.onEdit(withSlot(model, u.id, i, on ? SILENT : ch))}
                        />
                      </td>
                    );
                  });
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}

/** Each write of an alignment or a scene, grouped by element. */
export function WritesByElement(props: { writes: WriteResult[]; names: (element: string) => string; label: string; what?: (w: WriteResult) => string }) {
  const by = new Map<string, WriteResult[]>();
  for (const w of props.writes) by.set(w.element, [...(by.get(w.element) ?? []), w]);
  return (
    <ul class="sys-results" aria-label={props.label}>
      {[...by].map(([el, ws]) => (
        <li key={el} aria-label={`Result for ${props.names(el)}`}>
          <strong>{props.names(el)}</strong>
          {ws.map((w, i) => (
            <span key={i} class={`sys-result sys-${resultTone(w.result)}`} data-result={w.result}>
              {props.what ? props.what(w) : writeWhat(w.what)}: {resultWord(w.result)}
            </span>
          ))}
        </li>
      ))}
    </ul>
  );
}

function AlignPanel(props: { feed: SystemFeed; ask: Ask; admin: boolean; mayAlign: boolean }) {
  const st = useFeed(props.feed);
  const al = st.align;
  const names = (id: string) => subjectName(st.model, id);
  const apply = async () => {
    const go = await props.ask({
      title: "Align the Units",
      body: <p>Each unit whose link delay changes re-anchors: 2 to 6 ms of silence. Speaker delays are written too.</p>,
      buttons: [
        { label: "Align", value: "align" },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (go === "align") await props.feed.align(true);
  };
  return (
    <section class="sys-align" aria-label="Alignment">
      <h4>Alignment</h4>
      <div class="sys-row">
        {/* The hub takes `system.align` from Admin only, even to compute: the status carries the computation for everyone. */}
        <button type="button" disabled={!props.mayAlign || al?.running} onClick={() => void props.feed.align(false)} title={props.mayAlign ? undefined : writeNote(st.role, "align")}>
          Compute
        </button>
        <button type="button" disabled={!props.admin || al?.running} onClick={() => void apply()} title={props.admin ? undefined : writeNote(st.role, "align")}>
          Align
        </button>
        {al?.running ? <span class="sys-muted">Working…</span> : null}
      </div>
      {al?.error ? (
        <p class="error" role="alert">
          {al.error}
        </p>
      ) : null}
      {al?.report ? (
        <>
          <p class="sys-note" aria-label="Alignment result">
            {al.report.applied ? "Written" : "Computed, not written"}: system delay {framesMs(al.report.alignment.dFrames, al.report.alignment.rateHz || 48000)}
            {al.report.alignment.issues.length ? `, ${al.report.alignment.issues.length} issue(s)` : ""}
          </p>
          <ul class="sys-results" aria-label="Computed per unit">
            {al.report.alignment.units.map((u) => (
              <li key={u.unit} aria-label={`Plan for ${names(u.unit)}`}>
                <strong>{names(u.unit)}</strong>
                <span title="d_local">Link delay {framesMs(u.dLocal, al.report!.alignment.rateHz || 48000)}</span>
                <span title="hops_offset">Hop offset {u.hopsOffset}</span>
                <span class={`sys-result sys-${u.writable ? "ok" : "warn"}`}>{u.writable ? "Writable" : "Not written"}</span>
              </li>
            ))}
          </ul>
          {al.report.applied ? <WritesByElement writes={al.report.writes} names={names} label="Alignment writes" /> : null}
        </>
      ) : null}
    </section>
  );
}

function StatsTable(props: { feed: SystemFeed }) {
  const st = useFeed(props.feed);
  const model = st.model!;
  const rows = [
    ...model.fanOuts.filter((f) => f.serial).map((f) => ({ id: f.id, name: f.name || f.id, serial: f.serial! })),
    ...model.units.map((u) => ({ id: u.id, name: u.name || u.id, serial: u.serial })),
  ];
  return (
    <>
      <h4>Link stats and trim scan</h4>
      <div class="sys-scroll">
        <table class="sys-table" aria-label="Link stats">
          <thead>
            <tr>
              <th>Element</th>
              <th>State</th>
              <th>Class</th>
              <th>Rate</th>
              <th>Locks</th>
              <th>Losses</th>
              <th>CRC-8</th>
              <th>CRC-16</th>
              <th>Bit slips</th>
              <th>Superframe</th>
              <th>Phase error</th>
              <th>VCXO</th>
              <th>Trim scan</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => {
              const o = st.status?.observed[r.serial];
              const s = o?.stats;
              return (
                <tr key={r.id} aria-label={`Link stats of ${r.name}`}>
                  <th scope="row">{r.name}</th>
                  <td>{o?.online ? (o.state ?? "–") : "offline"}</td>
                  <td>{s ? classWord(s.lineClass) : "–"}</td>
                  <td>{rateOf(o?.rateCode)}</td>
                  <td>{s?.lockCount ?? "–"}</td>
                  <td>{o?.lossCount ?? "–"}</td>
                  <td>{s?.crc8Errors ?? "–"}</td>
                  <td>{o?.crc16Errors ?? "–"}</td>
                  <td>{s?.bitSlips ?? "–"}</td>
                  <td>{s?.superframeErrors ?? "–"}</td>
                  <td>{o?.phaseErrorFrames != null ? `${o.phaseErrorFrames.toFixed(2)} frames` : "–"}</td>
                  <td>{s ? `${s.vcxoPpm.toFixed(1)} ppm` : "–"}</td>
                  <td>
                    <TrimCell scan={o?.trimScan ?? null} />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </>
  );
}

/** The trim scan: 19 steps (-9..+9), the clean ones marked and the one in use outlined. */
function TrimCell(props: { scan: import("./model").TrimScan | null }) {
  const t = props.scan;
  if (!t) return <span>–</span>;
  if (t.state !== "done") return <span>{t.state === "no_clean_trim" ? "No clean trim" : t.state === "running" ? "Running" : "Not run"}</span>;
  return (
    <span class="sys-trim" aria-label={`Trim scan: clean ${t.trimLo} to ${t.trimHi}, using ${t.trimUsed}`}>
      {Array.from({ length: 19 }, (_, i) => (
        <span key={i} class={`sys-trim-step${(t.cleanMask >> i) & 1 ? " clean" : ""}${i - 9 === t.trimUsed ? " used" : ""}`} />
      ))}
      <span class="sys-muted">
        {" "}
        {t.trimLo}..{t.trimHi}, {t.trimUsed}
      </span>
    </span>
  );
}
