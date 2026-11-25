// WP WEBM3: Library › History (MODLAY 2; the desktop app's History window): this connection's
// changes, newest first, read again whenever the device's state changes, with Undo and Redo.
// Read only (a viewer, firmware the core only reads): the list without the buttons' effect.

import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import type { HistoryEntry } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { appleKeys } from "../../ui/undo";
import type { Session, SessionState } from "../../ui/session";
import { HISTORY_ROWS, historyRows, sameHistory, type HistoryNames } from "./model";
import { EmptyState, Glyph } from "../../ui/design"; // -- WP WEB -- (Glyph: WEB4, INTD)
import type { DspState } from "../../core/protocol";
import "./history.css"; // -- WP WEB4 --

/** -- WP WEB4 -- The channels as Qt's history names them: the name the user gave, else "Out 3", "In 1". */
export function namesOf(s: DspState): HistoryNames {
  const given = (wire: number, fallbacks: string[]) => {
    const n = (s.channelNames[wire] ?? "").trim();
    return n && !fallbacks.includes(n) ? n : "";
  };
  const output = (o: number) => given(s.numInputChannels + o, [`Output ${o + 1}`, `Out ${o + 1}`, `OUT${o + 1}`]) || `Out ${o + 1}`;
  const input = (i: number) => given(i, [`Input ${i + 1}`, `In ${i + 1}`, `IN${i + 1}`]) || `In ${i + 1}`;
  return { output, input, wire: (w) => (w < s.numInputChannels ? input(w) : output(w - s.numInputChannels)) };
}

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
  const state = st.snapshot?.state;
  const names = useMemo(() => (state ? namesOf(state) : undefined), [state?.channelNames, state?.numInputChannels]);
  const rows = useMemo(() => historyRows(entries ?? [], (p) => labels.current.get(p) ?? p, Date.now(), names), [entries, named, names]);
  if (!st.snapshot) return null;
  // -- WP WEB4 -- DL3a's History: Undo and Redo at the top, the changes as striped rows in the 760 px
  // column (the time in mono, an icon for what Undo can do with it, the change in words with its values under it)
  return (
    <section class="history-screen" aria-label="History">
      <HistoryTools session={session} state={st} />
      {entries === null ? (
        <p class="empty">Reading the changes…</p>
      ) : rows.length ? (
        <ol class="history-list" aria-label="Changes, newest first">
          {rows.map((r) => {
            const mark = !r.undoable ? { icon: "lock", words: "cannot be undone" } : r.source === "Undo" ? { icon: "undo-2", words: "undone" } : r.source === "Redo" ? { icon: "redo-2", words: "redone" } : null;
            return (
              <li key={r.key}>
                <span class="h-time">{r.time}</span>
                <span class="h-mark" title={mark?.words}>
                  {mark ? <Glyph name={mark.icon} class="h-icon" /> : null}
                </span>
                <span class="h-what">
                  {r.what}
                  {r.source && r.source !== "Undo" && r.source !== "Redo" && r.source !== "Local" ? <span class="h-pill">{r.source}</span> : null}
                  {mark ? <span class="visually-hidden"> · {r.source === "Undo" || r.source === "Redo" ? `${r.source}: ` : ""}{mark.words}</span> : null}
                  {r.values ? <span class="h-values">{r.values}</span> : null}
                </span>
              </li>
            );
          })}
        </ol>
      ) : (
        // -- WP WEB -- DL3a's empty state
        <EmptyState icon="history" title="No changes yet" text="What you change on this device is listed here, newest first, to undo or redo." />
      )}
      <p class="h-caption">The changes of this connection: a reconnect starts a new list.</p>
    </section>
  );
}

/** -- WP WEB7 -- Qt's History page: Undo and Redo with their Lucide icons and words, over the list. */
function HistoryTools(props: { session: Session; state: SessionState }) {
  const { session, state } = props;
  const ro = !!session.readOnlyReason;
  const apple = appleKeys(typeof navigator === "undefined" ? "" : (navigator.platform ?? ""));
  const mod = apple ? "⌘" : "Ctrl+";
  return (
    <div class="history-tools" role="group" aria-label="Undo and Redo">
      <button type="button" disabled={ro || !state.history.canUndo} title={`Undo (${mod}Z)`} onClick={() => void session.undo()}>
        <Glyph name="undo-2" />
        Undo
      </button>
      <button type="button" disabled={ro || !state.history.canRedo} title={apple ? "Redo (⇧⌘Z)" : "Redo (Ctrl+Shift+Z)"} onClick={() => void session.redo()}>
        <Glyph name="redo-2" />
        Redo
      </button>
    </div>
  );
}
