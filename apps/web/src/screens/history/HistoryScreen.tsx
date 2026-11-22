// WP WEBM3: Library › History (MODLAY 2; the desktop app's History window): this connection's
// changes, newest first, read again whenever the device's state changes, with Undo and Redo.
// Read only (a viewer, firmware the core only reads): the list without the buttons' effect.

import { useEffect, useRef, useState } from "preact/hooks";
import type { HistoryEntry } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { UndoButtons } from "../../ui/undo";
import { HISTORY_ROWS, historyRows } from "./model";

export function HistoryScreen(props: ScreenProps) {
  const { session } = props;
  const st = useSessionState(session);
  const [entries, setEntries] = useState<HistoryEntry[] | null>(null);
  const labels = useRef(new Map<string, string>());
  const [, bump] = useState(0);
  useEffect(() => {
    let alive = true;
    void session.core.call("history", { n: HISTORY_ROWS }).then(
      async (list) => {
        if (!alive) return;
        setEntries(list);
        // The paths' names, once each.
        const unknown = [...new Set(list.map((e) => e.path))].filter((p) => !labels.current.has(p));
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
  if (!st.snapshot) return null;
  const rows = historyRows(entries ?? [], (p) => labels.current.get(p) ?? p, Date.now());
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
        <p class="empty">No changes since this DSPi was opened.</p>
      )}
      <p class="muted">The changes of this connection: a reconnect starts a new list.</p>
    </section>
  );
}
