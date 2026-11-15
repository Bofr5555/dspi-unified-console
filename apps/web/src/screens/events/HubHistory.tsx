// WP P2WB1: the event log's "On the hub" source: the hub's shared history of
// this session's DSPi (`hubEventsRead` over the hub's `event.list`), with who
// caused each record by name (`by`). -- WP P2WB3 -- Kept current by the hub's
// own `event.added` (`hubEventsWatch`, forwarded each DSPi Link turn) instead
// of a poll: `HubHistoryFeed` (hub-history.ts).

import { useEffect, useState } from "preact/hooks";
import type { Session } from "../../ui/session";
import { historyRow } from "../hub/model";
import { HubHistoryFeed, type HubHistoryView } from "./hub-history";
import { fullTime, timeText } from "./model";

/** The hub's records of the session's DSPi, oldest first, kept current while mounted. */
export function useHubHistory(session: Session): HubHistoryView {
  const [view, setView] = useState<HubHistoryView>({ records: [], error: "" });
  useEffect(() => {
    const feed = new HubHistoryFeed(session);
    const stop = feed.subscribe(() => setView(feed.view));
    feed.start();
    return () => {
      stop();
      feed.stop();
    };
  }, [session]);
  return view;
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
