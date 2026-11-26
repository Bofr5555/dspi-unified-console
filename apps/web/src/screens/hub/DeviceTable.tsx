// -- WP UB6 -- (UXP B6, W10) The hub page's Devices part: the device table and the health board
// as one table (they listed the same DSPis twice). A row: the name, location, state, temperature
// (coloured from the hub's alarm rule), the alarms that are on about it (a press opens the Alarms
// part), the firmware, and Identify and Open; its details on expand: CPU, clip, the stream
// counters when the hub gives them, uptime, hangs, the last restart and change, its tags and note,
// and an Admin's Edit… and Forget…. Sort by the name, state, temperature or alarms; filter by tag
// or group. Moved here from HubScreen's device table and P3WHB's HealthBoard (same feed, same
// rules: nothing coloured from a threshold of its own).

import { useEffect, useId, useState } from "preact/hooks";
import type { HubGroupRecord } from "../../core/protocol";
import type { ScreenProps } from "../../ui/screens";
import { identifyRefused, refuseIdentify, type HubFeed, type HubView, type Said } from "./feed";
import {
  alarmTone,
  allTags,
  boardColumns,
  boardRows,
  cellTone,
  clipText,
  filterRows,
  nextSort,
  sortRows,
  thresholdsFrom,
  type BoardRow,
  type Sort,
  type SortKey,
  type Thresholds,
  type Tone,
  type ToneColumn,
} from "./health-model";
import { alarmCountText, deviceRows, durationText, rowActions, shortTime, tagsFrom, type HubRow } from "./model";

export function DeviceTable(p: {
  feed: HubFeed;
  view: HubView;
  props: ScreenProps;
  groups: HubGroupRecord[];
  /** Opens the Alarms part (a row's alarm count). */
  onAlarms: (() => void) | null;
  /** The Identify sentence (UWN): `who` and the hub's answer joined. */
  identifyFailure: (who: string, said: Said) => string;
}) {
  const { feed, view, props } = p;
  const session = props.session;
  const open = feed.openSerial();
  const all = boardRows(view.devices, view.health, view.alarms, open);
  const inv = new Map(deviceRows(view.devices, view.health, open).map((r) => [r.serial, r]));
  const [sort, setSort] = useState<Sort>({ key: "name", desc: false });
  const [filter, setFilter] = useState("");
  const [groupId, setGroupId] = useState("");
  const inGroup = p.groups.find((g) => g.id === groupId) ?? null;
  const rows = sortRows(filterRows(all, filter), sort).filter((r) => !inGroup || inGroup.members.includes(r.serial));
  const counters = boardColumns(all).counters;
  const t = thresholdsFrom(view.rules);
  const tagList = `hub-tags-${useId()}`;
  const [shown, setShown] = useState<ReadonlySet<string>>(new Set());
  const [said, setSaid] = useState<Said | null>(null);
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState<{ serial: string; location: string; note: string; tags: string } | null>(null);
  const [, refused] = useState(0);
  const run = async (f: () => Promise<Said>) => {
    setBusy(true);
    setSaid(await f());
    setBusy(false);
  };
  useEffect(() => {
    if (!said) return;
    const tm = setTimeout(() => setSaid(null), 10_000);
    return () => clearTimeout(tm);
  }, [said]);
  const readOnly = session.readOnlyReason;
  const opts = { role: view.role, identify: !!view.caps?.identify, inventory: !!view.caps?.inventory, readOnly, canOpen: !!session.openDevice };
  const toggle = (serial: string) =>
    setShown((s) => {
      const n = new Set(s);
      if (!n.delete(serial)) n.add(serial);
      return n;
    });
  // -- WP UWN -- (UXP A16) Identify's failure in one sentence naming the action and the device; a DSPi that cannot blink
  // is not offered Identify again (UB6: the Fleet and this page share what they learnt)
  const identify = (r: BoardRow) =>
    run(async () => {
      const r1 = await feed.identify(r.serial);
      if (!r1.bad) return r1;
      const who = all.some((o) => o.serial !== r.serial && o.name === r.name) ? `${r.name} (${r.serial})` : r.name;
      const text = p.identifyFailure(who, r1);
      if (r1.notice?.key === "unsupported") {
        refuseIdentify(r.serial, text);
        refused((n) => n + 1);
      }
      return { text, bad: true, notice: r1.notice ? { ...r1.notice, reason: "" } : null };
    });
  const forget = async (r: HubRow) => {
    const a = await props.ask({
      title: "Forget the DSPi",
      body: (
        <>
          <p>
            Forget {r.name} ({r.serial}) on the hub? Its record and its history on the hub go. It is listed again the next time it is attached.
          </p>
          <p>Its snapshots on the hub stay unless you delete them too.</p>
        </>
      ),
      buttons: [
        { label: "Forget", value: "forget", danger: true },
        { label: "Forget and Delete Its Snapshots", value: "with-data", danger: true },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (a !== "forget" && a !== "with-data") return;
    if (editing?.serial === r.serial) setEditing(null);
    await run(() => feed.forget(r.serial, a === "with-data"));
  };
  const head = (key: SortKey, label: string) => (
    <th scope="col" aria-sort={sort.key === key ? (sort.desc ? "descending" : "ascending") : "none"}>
      <button type="button" class="hub-sort" onClick={() => setSort(nextSort(sort, key))}>
        {label}
        {sort.key === key ? (sort.desc ? " ▼" : " ▲") : ""}
      </button>
    </th>
  );
  const COLS = 7;
  return (
    <section class="card hub-devices" aria-label="Devices on the hub">
      <div class="card-head">
        <h2 class="visually-hidden">Devices on the hub</h2>
        {view.caps?.health ? (
          <label class="hub-health-filter">
            Filter by tag
            <input type="search" value={filter} list={tagList} onInput={(e) => setFilter((e.target as HTMLInputElement).value)} />
            <datalist id={tagList}>
              {allTags(all).map((g) => (
                <option key={g} value={g} />
              ))}
            </datalist>
          </label>
        ) : null}
        {p.groups.length ? (
          <label class="hub-health-filter">
            Group
            <select value={inGroup ? inGroup.id : ""} onChange={(e) => setGroupId((e.target as HTMLSelectElement).value)}>
              <option value="">Every DSPi</option>
              {p.groups.map((g) => (
                <option key={g.id} value={g.id}>
                  {g.name}
                </option>
              ))}
            </select>
          </label>
        ) : null}
        <span class="hub-head-gap" />
        {view.role ? <span class="hub-role-tag">{roleWord(view.role)}</span> : null}
      </div>
      {view.error ? (
        <p class="error" role="alert">
          {view.error}
        </p>
      ) : null}
      {said ? (
        <p class={said.bad ? "error" : "note"} role="status" aria-label="Action result" data-key={said.notice?.key}>
          {said.text}
          {said.notice?.reason ? ` ${said.notice.reason}` : ""}
        </p>
      ) : null}
      {rows.length ? (
        <div class="hub-table-wrap">
          <table class="hub-table hub-merged">
            <thead>
              <tr>
                {head("name", "Name")}
                <th scope="col">Location</th>
                {head("state", "State")}
                {head("temp", "Temperature")}
                {head("alarms", "Alarms")}
                <th scope="col">Firmware</th>
                <th scope="col">
                  <span class="visually-hidden">Actions</span>
                </th>
              </tr>
            </thead>
            {/* a body per DSPi: its row and its details, on one stripe */}
            {rows.map((r) => {
                const d = inv.get(r.serial);
                const row: HubRow = d ?? { serial: r.serial, name: r.name, location: "", note: "", tags: r.tags, online: r.online, firmware: "", cpu: r.cpu, tempC: r.tempC, restart: r.restart, unresponsive: r.unresponsive, lastSeen: r.lastSeen, isOpen: r.isOpen };
                const may = rowActions(row, opts);
                const cannot = identifyRefused(r.serial);
                const open = shown.has(r.serial);
                const detailId = `hub-detail-${r.serial}`;
                const raised = r.alarms.filter((a) => a.state === "raised");
                return (
                  <tbody key={r.serial} class="hub-device">
                    <tr class={r.online ? "hub-health-row" : "hub-health-row offline"} aria-label={r.name}>
                      <td class="hub-health-name" data-label="Name">
                        <span class="hub-name">{r.name}</span>
                        <span class="hub-serial">{r.serial}</span>
                      </td>
                      <td data-label="Location">{row.location || "–"}</td>
                      <td class="hub-health-state" data-label="State">
                        <StateWord row={row} />
                      </td>
                      <ToneCell col="temp" label="Temperature" row={r} t={t} text={r.online && r.tempC !== null ? `${r.tempC.toFixed(1)} °C` : "–"} />
                      <td class="hub-health-alarms" data-label="Alarms">
                        {r.alarms.length ? (
                          <button
                            type="button"
                            class={`hub-alarm-count ${raised.some((a) => a.severity === "critical") ? "bad" : raised.length ? "warn" : "quiet"}`}
                            disabled={!p.onAlarms}
                            title={r.alarms.map((a) => view.notices[a.id]?.title || a.message || a.code).join("\n")}
                            aria-label={`${alarmCountText(r.alarms.length)} about ${r.name}: show the alarms`}
                            onClick={() => p.onAlarms?.()}
                          >
                            {alarmCountText(r.alarms.length)}
                          </button>
                        ) : (
                          "–"
                        )}
                      </td>
                      <td data-label="Firmware">{row.firmware || "–"}</td>
                      <td class="hub-actions">
                        <div class="hub-action-row">
                          {may.identify && !cannot ? (
                            <button type="button" disabled={busy || !!may.identifyOff} title={may.identifyOff || "Blink this DSPi's LED"} onClick={() => void identify(r)}>
                              Identify
                            </button>
                          ) : null}
                          {may.open ? (
                            <button type="button" onClick={() => session.openDevice!(r.serial)}>
                              Open
                            </button>
                          ) : null}
                          <button type="button" class="hub-more" aria-expanded={open} aria-controls={detailId} aria-label={`Details of ${r.name}`} onClick={() => toggle(r.serial)}>
                            {open ? "Less" : "More"}
                          </button>
                        </div>
                      </td>
                    </tr>
                    {open ? (
                      <tr class="hub-detail-row" id={detailId}>
                        <td colSpan={COLS}>
                          <dl class="hub-details" aria-label={`Details of ${r.name}`}>
                            <Detail label="CPU" cls="hub-health-cpu" tone={tone("cpu", r, t)} title={toneTitle("cpu", t, r)} text={r.online && r.cpu !== null ? `${r.cpu}%` : "–"} />
                            <Detail label="Clip" cls="hub-health-clip" tone={tone("clip", r, t)} title={toneTitle("clip", t, r)} text={r.online ? clipText(r) : "–"} />
                            {counters ? <Detail label="Underruns" cls="hub-health-underruns" text={r.online && r.underruns !== null ? String(r.underruns) : "–"} /> : null}
                            {counters ? <Detail label="Starvation" cls="hub-health-starvation" text={r.online && r.starvation !== null ? String(r.starvation) : "–"} /> : null}
                            <Detail label="Uptime" cls="hub-health-uptime" text={r.online && r.uptimeS !== null ? durationText(r.uptimeS) : "–"} />
                            <Detail label="Hangs" cls="hub-health-hangs" text={r.online && r.hangCount !== null ? String(r.hangCount) : "–"} />
                            <Detail label="Last restart" cls="hub-health-restart" text={r.restart || "–"} />
                            <Detail
                              label="Last change"
                              cls="hub-health-change"
                              text={
                                r.lastChange
                                  ? `${r.lastChange.text}${r.lastChange.by ? ` (by ${r.lastChange.by}${r.lastChange.at ? `, ${shortTime(r.lastChange.at)}` : ""})` : r.lastChange.at ? ` (${shortTime(r.lastChange.at)})` : ""}`
                                  : "–"
                              }
                            />
                            {r.tags.length ? <Detail label="Tags" cls="hub-tags" text={r.tags.join(", ")} /> : null}
                            {row.note ? <Detail label="Note" cls="hub-note" text={row.note} /> : null}
                          </dl>
                          {may.edit || may.forget ? (
                            <div class="hub-action-row hub-detail-actions">
                              {may.edit ? (
                                <button
                                  type="button"
                                  disabled={busy}
                                  aria-label={`Edit the details of ${r.name}`}
                                  onClick={() => setEditing({ serial: r.serial, location: row.location, note: row.note, tags: row.tags.join(", ") })}
                                >
                                  Edit…
                                </button>
                              ) : null}
                              {may.forget ? (
                                <button type="button" class="danger-text" disabled={busy} aria-label={`Forget ${r.name}`} onClick={() => void forget(row)}>
                                  Forget…
                                </button>
                              ) : null}
                            </div>
                          ) : null}
                          {editing?.serial === r.serial ? (
                            <EditForm
                              name={r.name}
                              editing={editing}
                              busy={busy}
                              onChange={setEditing}
                              onCancel={() => setEditing(null)}
                              onSave={() => {
                                const d2 = { location: editing.location.trim(), note: editing.note, tags: tagsFrom(editing.tags) };
                                void run(async () => {
                                  const res = await feed.setDetails(editing.serial, d2);
                                  if (!res.bad) setEditing(null);
                                  return res;
                                });
                              }}
                            />
                          ) : null}
                        </td>
                      </tr>
                    ) : null}
                  </tbody>
                );
              })}
          </table>
        </div>
      ) : (
        <p class="note">
          {!all.length
            ? view.caps?.inventory
              ? "The hub has seen no DSPi yet."
              : "The hub has no DSPi to show."
            : inGroup && filterRows(all, filter).length
              ? "No DSPi of that group to show."
              : "No DSPi has that tag."}
        </p>
      )}
    </section>
  );
}

/** A DSPi's state: a dot before the word. */
export function StateWord(props: { row: { online: boolean; unresponsive: boolean; isOpen: boolean; lastSeen: string } }) {
  const r = props.row;
  if (!r.online) return <span class="hub-state offline">Offline{r.lastSeen ? `, last seen ${shortTime(r.lastSeen)}` : ""}</span>;
  if (r.unresponsive) return <span class="hub-state bad">Not answering</span>;
  return <span class="hub-state ok">{r.isOpen ? "Online (open here)" : "Online"}</span>;
}

export function roleWord(role: string): string {
  return role === "admin" ? "Admin" : role === "control" ? "Control" : role === "viewer" ? "Viewer" : role;
}

function tone(col: ToneColumn, r: BoardRow, t: Thresholds): Tone {
  return r.online ? alarmTone(col, r) || cellTone(col, r, t) : ""; // -- WP P3WRO -- the raised alarm's severity first
}

/** Why a cell is coloured: the rule's threshold in words ("" when it is not). */
function toneTitle(col: ToneColumn, t: Thresholds, r: BoardRow): string | undefined {
  if (!tone(col, r, t)) return undefined;
  // -- WP P3WRO -- coloured by a raised alarm whose rule's threshold is not known here
  if ((col === "cpu" && t.cpuPct === null) || (col === "temp" && t.tempC === null) || (col === "clip" && r.clipPerMin !== null && t.clipPerMin === null))
    return "The hub's alarm about this is raised";
  if (col === "cpu") return `At or above the hub's CPU alarm rule (${t.cpuPct}%)`;
  if (col === "temp") return `At or above the hub's temperature alarm rule (${t.tempC} °C)`;
  return r.clipPerMin !== null ? `At or above the hub's clip alarm rule (${t.clipPerMin} a minute)` : "Clipped since someone cleared it";
}

function ToneCell(p: { col: ToneColumn; label: string; row: BoardRow; t: Thresholds; text: string }) {
  const tn = tone(p.col, p.row, p.t);
  return (
    <td class={`hub-health-${p.col}${tn ? ` tone-${tn}` : ""}`} data-label={p.label} data-tone={tn || undefined} title={toneTitle(p.col, p.t, p.row)}>
      {p.text}
    </td>
  );
}

function Detail(p: { label: string; cls: string; text: string; tone?: Tone; title?: string }) {
  return (
    <div class={`hub-detail ${p.cls}${p.tone ? ` tone-${p.tone}` : ""}`} data-tone={p.tone || undefined} title={p.title}>
      <dt>{p.label}</dt>
      <dd>{p.text}</dd>
    </div>
  );
}

function EditForm(p: {
  name: string;
  editing: { serial: string; location: string; note: string; tags: string };
  busy: boolean;
  onChange: (e: { serial: string; location: string; note: string; tags: string }) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const e0 = p.editing;
  return (
    <form
      class="hub-edit"
      aria-label={`Details of ${p.name}`}
      onSubmit={(e) => {
        e.preventDefault();
        p.onSave();
      }}
    >
      <h3>Details of {p.name}</h3>
      <label>
        Location
        <input value={e0.location} maxLength={256} onInput={(e) => p.onChange({ ...e0, location: (e.target as HTMLInputElement).value })} />
      </label>
      <label>
        Note
        <textarea value={e0.note} maxLength={256} rows={2} onInput={(e) => p.onChange({ ...e0, note: (e.target as HTMLTextAreaElement).value })} />
      </label>
      <label>
        Tags (separated by commas)
        <input value={e0.tags} onInput={(e) => p.onChange({ ...e0, tags: (e.target as HTMLInputElement).value })} />
      </label>
      <div class="hub-edit-actions">
        <button type="submit" disabled={p.busy}>
          Save
        </button>
        <button type="button" onClick={p.onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
