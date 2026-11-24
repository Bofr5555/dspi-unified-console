// WP WEBM3: Library › History (MODLAY 2; the desktop app's History window): this connection's
// changes, newest first, read again whenever the device's state changes, with Undo and Redo.
// Read only (a viewer, firmware the core only reads): the list without the buttons' effect.

import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import type { HistoryEntry } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { UndoButtons } from "../../ui/undo";
import { HISTORY_ROWS, historyRows, sameHistory } from "./model";
import { EmptyState } from "../../ui/design"; // -- WP WEB --

export function HistoryScreen(props: ScreenProps) {
  const { session } = props;
  const st = useSessionState(session);
  const [entries, setEntries] = useState<HistoryEntry[] | null>(null);
  const labels = useRef(new Map<string, string>());
  const [named, bump] = useState(0);
  useEffect(() => {
    let alive = true;
    void session.core.call("history", { n: HISTORY_ROWS }).then(
      async (list) => {
        if (!alive) return;
        // -- WP WEBM5 -- (review RC16) the same list keeps its rows
        setEntries((was) => (was && sameHistory(was, list) ? was : list));
        // The paths' names, once each.
        const unknown = [...new Set(list.map((e) => e.path))].filter((p) => !labels.current.has(p));
        // -- WP WEBM5 -- asked once, even when the next snapshot's read comes before the answer
        for (const path of unknown) labels.current.set(path, path);
        for (const path of unknown) {
          const label = path === "eq.band" ? "Filter band" : await session.core.call("describe", { path }).then((i) => i.label || path, () => path);
          labels.current.set(path, label);
        }
        if (alive && unknown.length) bump((n) => n + 1);
      },
      () => alive && setEntries([]),
    );
    return () => {
      alive = false;
    };
  }, [session, st.snapshot, st.history.canUndo, st.history.canRedo]);
  // -- WP WEBM5 -- (review RC16) made again only for a new list or new names, not on every snapshot
  const rows = useMemo(() => historyRows(entries ?? [], (p) => labels.current.get(p) ?? p, Date.now()), [entries, named]);
  if (!st.snapshot) return null;
  return (
    <section class="history-screen" aria-label="History">
      <div class="settings-buttons">
        <UndoButtons session={session} state={st} />
      </div>
      {entries === null ? (
        <p class="empty">Reading the changes…</p>
      ) : rows.length ? (
        <ol class="history-list" aria-label="Changes, newest first">
          {rows.map((r) => (
            <li key={r.key}>
              <span class="h-time">{r.time}</span>
              <span class="h-what">
                {r.what}
                {r.source ? <span class="h-source"> · {r.source}</span> : null}
                {r.undoable ? null : <span class="h-source"> · cannot be undone</span>}
              </span>
              <span class="h-values">{r.values}</span>
            </li>
          ))}
        </ol>
      ) : (
        // -- WP WEB -- DL3a's empty state
        <EmptyState icon="history" title="No changes yet" text="What you change on this device is listed here, newest first, to undo or redo." />
      )}
      <p class="muted">The changes of this connection: a reconnect starts a new list.</p>
    </section>
  );
}
