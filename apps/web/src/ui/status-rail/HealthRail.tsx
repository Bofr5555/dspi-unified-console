// WP WB6f: the status rail's second row and its health panel, inside the rail
// SW3 built (screens/system/StatusRail.tsx: connection, scene, mode, system
// mute, the hub's installation alarms). This row: the device, the preset (and
// whether it has unsaved changes), the two cores' load (M26) and the health
// summary, which opens the panel. The panel lists every problem since it was
// raised until it is acknowledged; it opens by itself when a new one comes.

import { useEffect, useRef, useState } from "preact/hooks";
import type { Session } from "../session";
import { useSessionState } from "../components";
import { presetLabel } from "../../screens/main/model";
import { LOAD_HIGH, healthSummary, type Problem } from "./health";
import type { HealthFeed } from "./feed";

export function useHealth(feed: HealthFeed) {
  const [, bump] = useState(0);
  useEffect(() => feed.subscribe(() => bump((n) => n + 1)), [feed]);
  return feed.view;
}

export function HealthRail(props: { session: Session; feed: HealthFeed }) {
  const ss = useSessionState(props.session);
  const view = useHealth(props.feed);
  const snap = ss.snapshot;
  const summary = healthSummary(view.problems);
  const [open, setOpen] = useState(false);
  // A problem that is new, or on again, opens the panel.
  const raised = view.problems.filter((p) => !p.acknowledged).map((p) => `${p.key}#${p.count}`);
  const seen = useRef(new Set<string>());
  useEffect(() => {
    const fresh = raised.filter((k) => !seen.current.has(k));
    for (const k of raised) seen.current.add(k);
    if (fresh.length) setOpen(true);
  }, [raised.join(" ")]);
  const unack = view.problems.filter((p) => !p.acknowledged).length;
  const slot = snap?.state.activePresetSlot ?? 0;
  return (
    <>
      <div class="sys-rail-row health-row">
        <span class="sys-rail-item" aria-label="Device">
          {snap?.device.name || "DSPi"}
        </span>
        {snap ? (
          <span class="sys-rail-item" aria-label="Preset">
            Preset {presetLabel(snap.state, slot)}
            {/* -- WP WEB2 -- (REVD W5) a warning dot and muted words, never orange body text */}
            {ss.unsaved.length ? (
              <span class="health-unsaved">
                {" · "}
                <i class="status-dot warn" aria-hidden="true" />
                unsaved changes
              </span>
            ) : null}
          </span>
        ) : null}
        <span class="sys-rail-item health-load" aria-label="Core load">
          {view.load
            ? view.load.map((pct, c) => (
                <span key={c} class={pct > LOAD_HIGH ? "health-load-high" : undefined}>
                  {c ? " · " : ""}C{c} {pct}%
                </span>
              ))
            : "C0 – · C1 –"}
        </span>
        <button
          type="button"
          class={`sys-rail-item health-sum health-${summary.tone}`}
          aria-expanded={open}
          aria-controls={`health-${props.session.id}`}
          onClick={() => setOpen(!open)}
        >
          Health: {summary.text}
        </button>
      </div>
      {open ? (
        <section class="health-panel" id={`health-${props.session.id}`} aria-label="Health">
          {view.problems.length ? (
            <ul class="health-list" aria-label="Problems">
              {view.problems.map((p) => (
                <ProblemRow key={p.key} p={p} onAck={() => props.feed.acknowledge(p.key)} />
              ))}
            </ul>
          ) : (
            <p class="note health-none">No problems.</p>
          )}
          <div class="health-actions">
            {unack > 1 ? (
              <button type="button" onClick={() => props.feed.acknowledgeAll()}>
                Acknowledge All
              </button>
            ) : null}
            <button type="button" onClick={() => setOpen(false)}>
              Hide
            </button>
          </div>
        </section>
      ) : null}
    </>
  );
}

function ProblemRow(props: { p: Problem; onAck: () => void }) {
  const { p } = props;
  const state = p.ended !== null ? "ended" : p.acknowledged ? "acknowledged" : "on";
  return (
    <li class={`health-item health-${p.severity} health-${state}`} aria-label={p.text} data-key={p.key} data-state={state}>
      <span class="sys-sev" aria-hidden="true" />
      <span class="health-text">
        {p.text}
        {p.count > 1 ? ` (${p.count}×)` : ""}
        <span class="sys-muted">
          {" "}
          · since {clock(p.since)}
          {p.ended !== null ? ` · ended ${clock(p.ended)}` : p.acknowledged ? " · acknowledged" : " · now"}
        </span>
      </span>
      {p.acknowledged ? null : (
        <button type="button" onClick={props.onAck}>
          Acknowledge
        </button>
      )}
    </li>
  );
}

/** The time of day, to the second. */
export function clock(ms: number): string {
  const d = new Date(ms);
  return [d.getHours(), d.getMinutes(), d.getSeconds()].map((n) => String(n).padStart(2, "0")).join(":");
}
