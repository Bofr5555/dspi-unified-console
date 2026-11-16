// WP P3WGR: a group's fleet history (PLAN-P3 F8): the hub's history of each member
// (`hubEventsRead`, the newest FLEET_HISTORY_PAGE records each, one member after another)
// merged by time, newest first, bounded, with who caused each record (`by`: a client's name,
// a key, a schedule). Read when the group is picked, after each of this page's group actions,
// and on Refresh.

import { useEffect, useRef, useState } from "preact/hooks";
import { CoreCallError, type HubGroupRecord, type HubHistoryRecord } from "../../core/protocol";
import type { Session } from "../../ui/session";
import { fullTime, timeText } from "../events/model";
import { FLEET_HISTORY_PAGE, GROUP_MEMBERS_MAX, failureWords, mergeFleet, type FleetRow } from "./groups-model";
import { hubKindWord } from "./model";

export function FleetHistory(p: { session: Session; group: HubGroupRecord; name: (serial: string) => string; stamp: number }) {
  const { session, group } = p;
  const [rows, setRows] = useState<FleetRow[] | null>(null);
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const turn = useRef(0);
  const key = group.members.join(",");
  const read = async () => {
    const mine = ++turn.current;
    setBusy(true);
    const pages: { serial: string; records: HubHistoryRecord[] }[] = [];
    const failed: string[] = [];
    for (const serial of group.members.slice(0, GROUP_MEMBERS_MAX)) {
      try {
        const page = await session.core.call("hubEventsRead", { serial, limit: FLEET_HISTORY_PAGE });
        pages.push({ serial, records: Array.isArray(page?.records) ? page.records : [] });
      } catch (e) {
        // A member the hub has no history of yet reads as none.
        if (e instanceof CoreCallError && (e.code === "not_found" || e.noticeKey === "dlink.not_found")) pages.push({ serial, records: [] });
        else failed.push(`${p.name(serial)}: ${failureWords(e).text}`);
      }
      if (mine !== turn.current) return;
    }
    setRows(mergeFleet(pages, p.name, hubKindWord));
    setProblem(failed.length ? `Not read: ${failed.join("; ")}` : "");
    setBusy(false);
  };
  useEffect(() => {
    setRows(null);
    void read();
    return () => {
      turn.current++;
    };
  }, [session, group.id, key, p.stamp]);
  const now = new Date();
  return (
    <div class="hub-fleet-history" aria-label="Fleet history">
      <div class="hub-group-head">
        <h3>History of the group</h3>
        <button type="button" disabled={busy} onClick={() => void read()}>
          Refresh
        </button>
      </div>
      {problem ? <p class="note">{problem}</p> : null}
      {rows === null ? (
        <p class="note">Reading the hub's history…</p>
      ) : rows.length ? (
        <div class="hub-table-wrap">
          <table class="event-list hub-event-list hub-fleet-list">
            <thead>
              <tr>
                <th scope="col">Time</th>
                <th scope="col">DSPi</th>
                <th scope="col">By</th>
                <th scope="col">Event</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.key} data-serial={r.serial}>
                  <td class="event-time" title={fullTime(r.t)}>
                    {timeText(r.t, now)}
                  </td>
                  <td class="event-unit">
                    {r.name}
                    {r.name !== r.serial ? <span class="hub-serial">{r.serial}</span> : null}
                  </td>
                  <td class="event-by">{r.by || "–"}</td>
                  <td class="event-text">
                    <span class="event-kind">{r.kind}</span> {r.text}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p class="note">No events on the hub for this group's DSPis yet.</p>
      )}
    </div>
  );
}
