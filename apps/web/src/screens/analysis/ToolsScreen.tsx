// WP WB6j: the Console's tool windows on the web, each a screen of the session (ids "generator",
// "analyser", "stats", "monitor", which WB6L's shortcuts open): the signal generator (M46), the
// Spectrum Analyser (M22), Stats for nerbs with Reset Watermarks (M53), the Interrupt Monitor (M54).
// Each part reads the core only while it is shown; every change is the core's call.

import { useEffect, useRef, useState } from "preact/hooks";
import type { MonitorEntryRecord } from "../../core/tools-protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { SpectrumPage } from "./SpectrumPage"; // -- WP WEB6 --
import { MONITOR_POLL_MS, appendLines, copyText, eventCount, lineTime } from "./stats";
import { Glyph, StatusChip } from "../../ui/design"; // -- WP WEB --
import "./analysis.css"; // -- WP WEB4 --
import { GeneratorPage } from "./GeneratorPage"; // -- WP WEB7 --
import { StatsPage } from "./StatsPage"; // -- WP WEB7 --

/** Signal Generator (M46). */
export function GeneratorScreen(props: ScreenProps) {
  return <GeneratorPage {...props} />; // -- WP WEB7 -- Qt Modern's SiggenPanel (GeneratorPage.tsx)
}

/** The Spectrum Analyser on a screen of its own (M22): -- WP WEB6 -- Qt's Spectrum page. */
export function AnalyserScreen(props: ScreenProps) {
  const st = useSessionState(props.session);
  const snap = st.snapshot;
  if (!snap) return null;
  return <SpectrumPage session={props.session} s={snap.state} labels={snap.outputLabels} />;
}

/** Stats for nerbs (M53). */
export function StatsScreen(props: ScreenProps) {
  return <StatsPage {...props} />; // -- WP WEB7 -- Qt Modern's StatsPanel (StatsPage.tsx)
}

/** Interrupt Monitor (M54). */
export function MonitorScreen(props: ScreenProps) {
  return <MonitorPanel session={props.session} />;
}

/** Run `fn` every `ms` while mounted and the page is seen, first at once. */
function usePoll(fn: () => Promise<void>, ms: number, on: boolean, deps: unknown[]): void {
  useEffect(() => {
    if (!on) return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const run = async () => {
      if (!alive) return;
      if (document.visibilityState !== "hidden") await fn().catch(() => undefined);
      if (alive) timer = setTimeout(run, ms);
    };
    void run();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [on, ms, ...deps]);
}

// -- WP WEB7 -- the signal generator and Stats for nerbs moved to GeneratorPage.tsx and StatsPage.tsx

// ------------------------------------------------------------ interrupt monitor

function MonitorPanel(props: { session: Session }) {
  const { session } = props;
  const st = useSessionState(session);
  const open = st.status === "open";
  const [lines, setLines] = useState<MonitorEntryRecord[]>([]);
  const [paused, setPaused] = useState(false);
  const last = useRef(0);
  const list = useRef<HTMLOListElement>(null);

  usePoll(
    async () => {
      const b = await session.core.call("monitorRead", { afterId: last.current });
      last.current = b.lastId;
      if (b.entries.length) setLines((l) => appendLines(l, b.entries));
    },
    MONITOR_POLL_MS,
    open && !paused,
    [session],
  );
  useEffect(() => {
    const el = list.current;
    if (el && !paused) el.scrollTop = el.scrollHeight;
  }, [lines.length, paused]);

  const clear = async () => {
    try {
      await session.core.call("monitorClear", undefined);
    } catch {
      // The page's copy is cleared all the same.
    }
    setLines([]);
  };
  const copy = () => {
    void navigator.clipboard?.writeText(copyText(lines)).catch(() => undefined);
  };
  return (
    // -- WP WEB4 -- no card: the tools and the state in one row at the top, the lines in mono, an empty state while there are none
    <section class="monitor" aria-label="Interrupt Monitor">
      <div class="monitor-tools">
        <button type="button" aria-pressed={paused} onClick={() => setPaused((p) => !p)}>
          {paused ? "Resume" : "Pause"}
        </button>
        <button type="button" onClick={() => void clear()}>
          Clear
        </button>
        <button type="button" title="Copies the lines shown, oldest first" disabled={!lines.length} onClick={copy}>
          Copy
        </button>
        <StatusChip kind={open && !paused ? "ok" : "idle"} role="status">
          {!open ? "Inactive" : paused ? "Paused" : "Listening"} · {eventCount(lines.length)}
        </StatusChip>
      </div>
      {/* the empty state's look without its status role: the chip above is this page's status */}
      {lines.length ? null : (
        <div class="empty-state">
          <Glyph name="activity" class="empty-icon" />
          <p class="empty-title">No notifications yet</p>
          <p class="empty-text">The device's notifications are listed here as they come, oldest first.</p>
        </div>
      )}
      <ol class="monitor-lines" ref={list} aria-label="Notification lines" hidden={!lines.length}>
        {lines.map((l) => (
          <li key={l.id}>
            <time>{lineTime(l.timeMs)}</time> <code>{l.text}</code>
          </li>
        ))}
      </ol>
    </section>
  );
}
