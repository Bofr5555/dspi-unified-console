// WP P3WHB: the hub screen's health board (PLAN-P3 F7, OD-P3m). One row per online
// DSPi with what the hub's health poll read (CPU, temperature, clip events a minute or
// "Clipped" on latch-only firmware, the stream counters when the hub gives them, uptime,
// hang count, the last restart, the last change and who made it) and its alarms, each
// acknowledged from the row; the offline DSPis below. A cell is coloured from the hub's
// alarm rules (the feed's `rules`, `hubAlarmRulesGet`): nothing is hard-coded here. Sort
// by any column, filter by tag; a card per DSPi where the session is narrow. Everything
// comes through `HubFeed` (the core's typed calls).

import { useId, useState } from "preact/hooks";
import type { HubAlarmRecord } from "../../core/protocol";
import type { HubFeed, HubView, Said } from "./feed";
import {
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
import { alarmTitle, durationText, shortTime } from "./model";

interface Column {
  key: SortKey;
  label: string;
}

export function HealthBoard(props: { feed: HubFeed; view: HubView; readOnly: string }) {
  const { feed, view } = props;
  const [sort, setSort] = useState<Sort>({ key: "name", desc: false });
  const [filter, setFilter] = useState("");
  const [said, setSaid] = useState<Said | null>(null);
  const tagList = `hub-health-tags-${useId()}`;
  const all = boardRows(view.devices, view.health, view.alarms, feed.openSerial());
  const shown = cols(boardColumns(all).counters);
  const rows = sortRows(filterRows(all, filter), sort);
  const t = thresholdsFrom(view.rules);
  const tags = allTags(all);
  const mayAck = !props.readOnly && view.role !== "viewer";
  const ackOff = props.readOnly || (view.role === "viewer" ? "A Viewer cannot acknowledge alarms" : "");
  const ack = async (a: HubAlarmRecord) => {
    const r = await feed.acknowledge(a.id);
    setSaid(r.bad ? r : null);
  };
  return (
    <section class="card hub-health" aria-label="Health board">
      <div class="card-head">
        <h2>Health board</h2>
        <label class="hub-health-filter">
          Filter by tag
          <input type="search" value={filter} list={tagList} onInput={(e) => setFilter((e.target as HTMLInputElement).value)} />
          <datalist id={tagList}>
            {tags.map((g) => (
              <option key={g} value={g} />
            ))}
          </datalist>
        </label>
      </div>
      {said ? (
        <p class="error" role="alert" data-key={said.notice?.key}>
          {said.text}
        </p>
      ) : null}
      {rows.length ? (
        <div class="hub-health-wrap">
          <table class="hub-health-table">
            <thead>
              <tr>
                {shown.map((c) => (
                  <th key={c.key} scope="col" aria-sort={sort.key === c.key ? (sort.desc ? "descending" : "ascending") : "none"}>
                    <button type="button" class="hub-sort" onClick={() => setSort(nextSort(sort, c.key))}>
                      {c.label}
                      {sort.key === c.key ? (sort.desc ? " ▼" : " ▲") : ""}
                    </button>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.serial} class={r.online ? "hub-health-row" : "hub-health-row offline"} aria-label={r.name}>
                  {shown.map((c) => (
                    <Cell key={c.key} col={c.key} label={c.label} row={r} t={t} view={view} mayAck={mayAck} ackOff={ackOff} ack={ack} />
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p class="note">{all.length ? "No DSPi has that tag." : "The hub has no DSPi to show."}</p>
      )}
    </section>
  );
}

/** The columns in order; the counters' two only when the hub gives them. */
function cols(counters: boolean): Column[] {
  const c: Column[] = [
    { key: "name", label: "Name" },
    { key: "state", label: "State" },
    { key: "cpu", label: "CPU" },
    { key: "temp", label: "Temperature" },
    { key: "clip", label: "Clip" },
  ];
  if (counters) c.push({ key: "underruns", label: "Underruns" }, { key: "starvation", label: "Starvation" });
  c.push(
    { key: "uptime", label: "Uptime" },
    { key: "hangs", label: "Hangs" },
    { key: "restart", label: "Last restart" },
    { key: "change", label: "Last change" },
    { key: "alarms", label: "Alarms" },
  );
  return c;
}

const TONE_COLUMN: Partial<Record<SortKey, ToneColumn>> = { cpu: "cpu", temp: "temp", clip: "clip" };

/** Why a cell is coloured: the rule's threshold in words. */
function toneTitle(col: ToneColumn, t: Thresholds, r: BoardRow): string {
  if (col === "cpu") return `At or above the hub's CPU alarm rule (${t.cpuPct}%)`;
  if (col === "temp") return `At or above the hub's temperature alarm rule (${t.tempC} °C)`;
  return r.clipPerMin !== null ? `At or above the hub's clip alarm rule (${t.clipPerMin} a minute)` : "Clipped since someone cleared it";
}

function Cell(p: {
  col: SortKey;
  label: string;
  row: BoardRow;
  t: Thresholds;
  view: HubView;
  mayAck: boolean;
  ackOff: string;
  ack: (a: HubAlarmRecord) => Promise<void>;
}) {
  const { col, row: r } = p;
  const tc = TONE_COLUMN[col];
  const tone: Tone = tc && r.online ? cellTone(tc, r, p.t) : "";
  const cls = `hub-health-${col}${tone ? ` tone-${tone}` : ""}`;
  const attrs = { class: cls, "data-label": p.label, "data-tone": tone || undefined, title: tone && tc ? toneTitle(tc, p.t, r) : undefined };
  const dash = "–";
  const on = r.online;
  switch (col) {
    case "name":
      return (
        <td {...attrs}>
          <span class="hub-name">{r.name}</span>
          <span class="hub-serial">{r.serial}</span>
          {r.tags.length ? <span class="hub-tags">{r.tags.join(", ")}</span> : null}
        </td>
      );
    case "state":
      return (
        <td {...attrs}>
          {!on ? (
            <span class="hub-state offline">Offline{r.lastSeen ? `, last seen ${shortTime(r.lastSeen)}` : ""}</span>
          ) : r.unresponsive ? (
            <span class="hub-state bad">Not answering</span>
          ) : (
            <span class="hub-state ok">{r.isOpen ? "Online (open here)" : "Online"}</span>
          )}
        </td>
      );
    case "cpu":
      return <td {...attrs}>{on && r.cpu !== null ? `${r.cpu}%` : dash}</td>;
    case "temp":
      return <td {...attrs}>{on && r.tempC !== null ? `${r.tempC.toFixed(1)} °C` : dash}</td>;
    case "clip":
      return <td {...attrs}>{on ? clipText(r) : dash}</td>;
    case "underruns":
      return <td {...attrs}>{on && r.underruns !== null ? String(r.underruns) : dash}</td>;
    case "starvation":
      return <td {...attrs}>{on && r.starvation !== null ? String(r.starvation) : dash}</td>;
    case "uptime":
      return <td {...attrs}>{on && r.uptimeS !== null ? durationText(r.uptimeS) : dash}</td>;
    case "hangs":
      return <td {...attrs}>{on && r.hangCount !== null ? String(r.hangCount) : dash}</td>;
    case "restart":
      return <td {...attrs}>{r.restart || dash}</td>;
    case "change":
      return (
        <td {...attrs}>
          {r.lastChange ? (
            <>
              <span class="hub-change-text">{r.lastChange.text}</span>
              <span class="hub-change-by">
                {r.lastChange.by ? `by ${r.lastChange.by}` : ""}
                {r.lastChange.by && r.lastChange.at ? ", " : ""}
                {r.lastChange.at ? shortTime(r.lastChange.at) : ""}
              </span>
            </>
          ) : (
            dash
          )}
        </td>
      );
    case "alarms":
      return (
        <td {...attrs}>
          {r.alarms.length ? (
            <ul class="hub-row-alarms">
              {r.alarms.map((a) => (
                <li key={a.id} class={`hub-row-alarm ${a.state} ${a.severity === "critical" ? "bad" : "warn"}`} data-key={a.noticeKey}>
                  <span>{alarmTitle(a, p.view.notices[a.id] ?? null, "")}</span>
                  {a.state === "raised" ? (
                    <button type="button" disabled={!p.mayAck} title={p.ackOff || undefined} onClick={() => void p.ack(a)}>
                      Acknowledge
                    </button>
                  ) : (
                    <span class="hub-alarm-state">Acknowledged</span>
                  )}
                </li>
              ))}
            </ul>
          ) : (
            dash
          )}
        </td>
      );
  }
}
