// WP P3WGR: a group's action bar (PLAN-P3 F4, F8): the volume with an offset per member, shown
// as the value each member will get after the clamp to the registry's range (the hub clamps
// to each device's own range and says so in its row); mute and unmute; a preset with a
// member's own slot where it differs; and Read, the members' volume now (`hubGroupParamGet`,
// every role). Writes are Control and Admin (`hubGroupParamSet`, `hubGroupPresetLoad`), one
// member after another at the hub, each judged on its own. Every answer's rows stay on the
// page, one line per device, until dismissed: a partial failure is shown, never hidden.

import { useEffect, useState } from "preact/hooks";
import type { HubGroupRecord, ParamInfo } from "../../core/protocol";
import type { Session } from "../../ui/session";
import type { HubView } from "./feed";
import {
  MUTE_PATH,
  PRESET_SLOTS,
  VOLUME_PATH,
  dbFrom,
  dbText,
  failureWords,
  failures,
  memberRows,
  offsetList,
  offsetPreview,
  paramOk,
  presetOk,
  resultLines,
  slotText,
  targetOf,
  type ResultSet,
} from "./groups-model";

/** Why this session may not write to a group ("" when it may). */
export function writeOff(role: string, readOnly: string): string {
  if (readOnly) return readOnly;
  if (role === "viewer") return "A Viewer cannot change the group's DSPis";
  return "";
}

/** A request's rows, one line per device, kept until dismissed. */
export function ResultsPanel(p: { result: ResultSet; label: string; onDismiss: () => void }) {
  const r = p.result;
  const bad = failures(r.lines);
  return (
    <div class={bad ? "hub-results partial" : "hub-results"} role="status" aria-label={p.label}>
      <div class="hub-results-head">
        <strong>{r.title}</strong>
        <span class={bad ? "hub-count bad" : "hub-count"}>{bad ? `${bad} of ${r.lines.length} did not` : r.lines.length ? `All ${r.lines.length} done` : "No DSPis"}</span>
        <button type="button" onClick={p.onDismiss}>
          Dismiss
        </button>
      </div>
      {r.note ? <p class="note">{r.note}</p> : null}
      <ul class="hub-result-lines">
        {r.lines.map((l) => (
          <li key={l.serial} class={`hub-result ${l.ok ? "ok" : l.tone}`} aria-label={l.name} data-serial={l.serial} data-key={l.key || undefined}>
            <span class="hub-name">{l.name}</span>
            {l.name !== l.serial ? <span class="hub-serial">{l.serial}</span> : null}
            <span class="hub-result-text">{l.text}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

export function GroupActions(p: { session: Session; view: HubView; group: HubGroupRecord; name: (serial: string) => string; onDone: () => void }) {
  const { session, view, group } = p;
  const off = writeOff(view.role, session.readOnlyReason);
  const [range, setRange] = useState<{ min?: number; max?: number }>({});
  const [volume, setVolume] = useState("-20");
  const [offsets, setOffsets] = useState<Record<string, string>>({});
  const [slot, setSlot] = useState(0);
  const [slots, setSlots] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ResultSet | null>(null);
  useEffect(() => {
    session.core.call("describe", { path: VOLUME_PATH }).then(
      (d: ParamInfo) => setRange({ min: d.min, max: d.max }),
      () => setRange({}),
    );
  }, [session]);
  // Another group: its own offsets and slots.
  useEffect(() => {
    setOffsets({});
    setSlots({});
  }, [group.id]);

  const members = memberRows(group, view.devices);
  const base = dbFrom(volume);
  const nums: Record<string, number> = {};
  for (const [s, t] of Object.entries(offsets)) {
    const n = dbFrom(t);
    if (n !== null) nums[s] = n;
  }
  const preview = base === null ? [] : offsetPreview(base, group.members, nums, range);
  const badOffset = Object.values(offsets).some((t) => t.trim() !== "" && dbFrom(t) === null);
  const outOfRange = base !== null && ((range.min !== undefined && base < range.min) || (range.max !== undefined && base > range.max));

  const run = async (title: string, f: () => Promise<ResultSet["lines"]>, note = "") => {
    setBusy(true);
    try {
      setResult({ title, lines: await f(), note });
    } catch (e) {
      const w = failureWords(e);
      setResult({ title, lines: [], note: w.text });
    } finally {
      setBusy(false);
      p.onDone();
    }
  };
  const setVol = () =>
    run(`Volume ${dbText(base!)} on ${group.name}`, async () => {
      const r = await session.core.call("hubGroupParamSet", { target: targetOf(group), path: VOLUME_PATH, value: { float: base! }, offsets: offsetList(group.members, nums) });
      return resultLines(r.results, p.name, paramOk(VOLUME_PATH));
    });
  const mute = (on: boolean) =>
    run(`${on ? "Mute" : "Unmute"} ${group.name}`, async () => {
      const r = await session.core.call("hubGroupParamSet", { target: targetOf(group), path: MUTE_PATH, value: { bool: on } });
      return resultLines(r.results, p.name, paramOk(MUTE_PATH));
    });
  const load = () =>
    run(`${slotText(slot)} on ${group.name}`, async () => {
      const map = group.members.filter((s) => slots[s] !== undefined && slots[s] !== "" && Number(slots[s]) !== slot).map((serial) => ({ serial, slot: Number(slots[serial]) }));
      const rows = await session.core.call("hubGroupPresetLoad", { target: targetOf(group), slot, ...(map.length ? { map } : {}) });
      return resultLines(rows, p.name, presetOk);
    });
  const read = () =>
    run(`Volume now on ${group.name}`, async () => {
      const r = await session.core.call("hubGroupParamGet", { target: targetOf(group), path: VOLUME_PATH });
      return resultLines(r.results, p.name, paramOk(VOLUME_PATH));
    });
  const none = !group.members.length;

  return (
    <div class="hub-group-actions" aria-label="Group actions">
      <h3>Act on the group</h3>
      {off ? <p class="note">{off}.</p> : null}
      <div class="hub-group-volume">
        <label>
          Volume (dB)
          <input inputMode="decimal" value={volume} disabled={!!off} onInput={(e) => setVolume((e.target as HTMLInputElement).value)} aria-invalid={base === null || outOfRange} />
        </label>
        {range.min !== undefined && range.max !== undefined ? <span class="note">{`${range.min} to ${range.max} dB`}</span> : null}
      </div>
      {members.length ? (
        <table class="hub-table hub-offsets" aria-label="Offsets">
          <thead>
            <tr>
              <th scope="col">DSPi</th>
              <th scope="col">Offset (dB)</th>
              <th scope="col">Will get</th>
              <th scope="col">Preset</th>
            </tr>
          </thead>
          <tbody>
            {members.map((m) => {
              const pv = preview.find((x) => x.serial === m.serial);
              return (
                <tr key={m.serial} aria-label={m.name} data-serial={m.serial} class={m.online ? undefined : "offline"}>
                  <td data-label="DSPi">
                    <span class="hub-name">{m.name}</span>
                    {m.name !== m.serial ? <span class="hub-serial">{m.serial}</span> : null}
                    {m.online ? null : <span class="hub-state offline">Offline</span>}
                  </td>
                  <td data-label="Offset (dB)">
                    <input
                      class="hub-offset"
                      inputMode="decimal"
                      placeholder="0"
                      disabled={!!off}
                      aria-label={`Offset of ${m.name}`}
                      value={offsets[m.serial] ?? ""}
                      onInput={(e) => setOffsets({ ...offsets, [m.serial]: (e.target as HTMLInputElement).value })}
                    />
                  </td>
                  <td data-label="Will get" class="hub-will-get">
                    {pv ? `${dbText(pv.value)}${pv.clamped ? " (clamped)" : ""}` : "–"}
                  </td>
                  <td data-label="Preset">
                    <select aria-label={`Preset of ${m.name}`} disabled={!!off} value={slots[m.serial] ?? ""} onChange={(e) => setSlots({ ...slots, [m.serial]: (e.target as HTMLSelectElement).value })}>
                      <option value="">As the group</option>
                      {Array.from({ length: PRESET_SLOTS }, (_, i) => (
                        <option key={i} value={String(i)}>
                          {slotText(i)}
                        </option>
                      ))}
                    </select>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      ) : null}
      <div class="hub-action-row">
        <button type="button" disabled={busy || !!off || none || base === null || outOfRange || badOffset} title={off || undefined} onClick={() => void setVol()}>
          Set Volume
        </button>
        <button type="button" disabled={busy || !!off || none} title={off || undefined} onClick={() => void mute(true)}>
          Mute
        </button>
        <button type="button" disabled={busy || !!off || none} title={off || undefined} onClick={() => void mute(false)}>
          Unmute
        </button>
        <label class="hub-inline">
          Preset
          <select aria-label="The group's preset" disabled={!!off} value={String(slot)} onChange={(e) => setSlot(Number((e.target as HTMLSelectElement).value))}>
            {Array.from({ length: PRESET_SLOTS }, (_, i) => (
              <option key={i} value={String(i)}>
                {slotText(i)}
              </option>
            ))}
          </select>
        </label>
        <button type="button" disabled={busy || !!off || none} title={off || undefined} onClick={() => void load()}>
          Load Preset
        </button>
        <button type="button" disabled={busy || none} onClick={() => void read()}>
          Read Volume
        </button>
      </div>
      {result ? <ResultsPanel result={result} label="Group action result" onDismiss={() => setResult(null)} /> : null}
    </div>
  );
}
