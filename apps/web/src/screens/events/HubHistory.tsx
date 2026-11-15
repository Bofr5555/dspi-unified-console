// WP P2WB1: the event log's "On the hub" source: the hub's shared history of
// this session's DSPi (`hubEventsRead` over the hub's `event.list`), with who
// caused each record by name (`by`). The newest page first, then only what is
// new, every second while shown and the page is visible; when the hub's
// numbering started again (a forget), read afresh.

import { useEffect, useState } from "preact/hooks";
import type { HubHistoryRecord } from "../../core/protocol";
import type { Session } from "../../ui/session";
import { noticeOf } from "../../ui/texts";
import { historyRow, mergeHistory } from "../hub/model";
import { fullTime, timeText } from "./model";

/** How often the hub's history is read while shown. */
export const HUB_HISTORY_POLL_MS = 1000;
/** Records asked for in one read. */
const PAGE = 200;

/** The hub's records of the session's DSPi, oldest first, kept current while mounted. */
export function useHubHistory(session: Session, pollMs = HUB_HISTORY_POLL_MS): { records: HubHistoryRecord[]; error: string } {
  const [records, setRecords] = useState<HubHistoryRecord[]>([]);
  const [error, setError] = useState("");
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let after = 0;
    let kept: HubHistoryRecord[] = [];
    const read = async () => {
      const visible = typeof document === "undefined" || document.visibilityState === "visible";
      if (visible) {
        try {
          let page = await session.core.call("hubEventsRead", { afterId: after, limit: PAGE });
          let restarted = false;
          if (after > 0 && page.newestId < after) {
            // The hub's numbering started again: the newest page afresh.
            restarted = true;
            after = 0;
            page = await session.core.call("hubEventsRead", { afterId: 0, limit: PAGE });
          }
          if (!alive) return;
          const last = page.records.length ? page.records[page.records.length - 1]!.id : after;
          after = Math.max(restarted ? 0 : after, last);
          if (page.records.length || restarted) {
            kept = mergeHistory(kept, page.records, restarted);
            setRecords(kept);
          }
          setError("");
          // More than one page is new: read on at once.
          if (page.more && alive) {
            timer = setTimeout(() => void read(), 0);
            return;
          }
        } catch (e) {
          if (alive) setError(noticeOf(e)?.title ?? (e instanceof Error ? e.message : String(e)));
        }
      }
      if (alive) timer = setTimeout(() => void read(), pollMs);
    };
    void read();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [session, pollMs]);
  return { records, error };
}

export function HubHistory(props: { session: Session }) {
  const log = useHubHistory(props.session);
  const rows = log.records.map(historyRow).sort((a, b) => b.t - a.t || (a.key < b.key ? 1 : -1));
  const now = new Date();
  return (
    <div class="hub-history" aria-label="On the hub">
      {log.error ? (
        <p class="error" role="alert">
          {log.error}
        </p>
      ) : null}
      {rows.length ? (
        <table class="event-list hub-event-list">
          <thead>
            <tr>
              <th scope="col">Time</th>
              <th scope="col">By</th>
              <th scope="col">Event</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.key} class={r.tone ? `tone-${r.tone}` : undefined}>
                <td class="event-time" title={fullTime(r.t)}>
                  {timeText(r.t, now)}
                </td>
                <td class="event-by">{r.by || "–"}</td>
                <td class="event-text">
                  <span class="event-kind">{r.kind}</span> {r.text}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <p class="note">No events on the hub yet.</p>
      )}
    </div>
  );
}
