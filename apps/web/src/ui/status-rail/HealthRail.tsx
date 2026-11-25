// WP WB6f: the status rail's health summary and its panel, inside the rail
// SW3 built (screens/system/StatusRail.tsx: connection, scene, mode, system
// mute, the hub's installation alarms). The summary opens the panel. The panel
// lists every problem since it was raised until it is acknowledged; it opens by
// itself when a new one comes.
// -- WP REVEFIX -- (REVE §3.1, F8) on the rail's one line: the device and the
// preset are the top bar's, the two cores' load (M26) is the dock's (Qt's).

import { useEffect, useRef, useState } from "preact/hooks";
import type { Session } from "../session";
import { LOAD_HIGH, healthSummary, type Problem } from "./health";
import type { HealthFeed } from "./feed";

export function useHealth(feed: HealthFeed) {
  const [, bump] = useState(0);
  useEffect(() => feed.subscribe(() => bump((n) => n + 1)), [feed]);
  return feed.view;
}

export function HealthRail(props: { session: Session; feed: HealthFeed }) {
  const view = useHealth(props.feed);
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
  return (
    <>
      <div class="sys-rail-row health-row">
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

// -- WP REVEFIX --
/** The two cores' load (M26), at the dock's right end as Qt's: "C0 3%  C1 0%", a high one in the danger colour. */
export function CoreLoad(props: { feed: HealthFeed }) {
  const view = useHealth(props.feed);
  return (
    <span class="dock-load health-load" aria-label="Core load">
      {(view.load ?? [null, null]).map((pct, c) => (
        <span key={c} class={pct !== null && pct > LOAD_HIGH ? "health-load-high" : undefined}>
          C{c} {pct === null ? "–" : `${pct}%`}
        </span>
      ))}
    </span>
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
