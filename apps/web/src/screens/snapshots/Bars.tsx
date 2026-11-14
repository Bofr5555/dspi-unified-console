// WP P1WB1: the parts of the desk every screen shows. The A/B toggle sits in the scene bar
// (WB6e's host-side half: the hub's scenes stay above it), the countdown bar of a guarded
// apply under it in every mode, and what the last action did as a line that can be dismissed
// (no modal for a success).

import { useEffect, useState } from "preact/hooks";
import { mayToggle, secondsLeft, type DeskState, type Outcome, type SnapshotDesk } from "./model";
import { abKeyHint } from "../../ui/keys";

export function useDesk(desk: SnapshotDesk): DeskState {
  const [, bump] = useState(0);
  const rendered = desk.state;
  useEffect(() => {
    const stop = desk.watch(() => bump((n) => n + 1));
    if (desk.state !== rendered) bump((n) => n + 1);
    return stop;
  }, [desk]);
  return desk.state;
}

/** The compare's row in the scene bar: which side is on, the toggle, Keep A and Keep B. Only while one runs. */
export function AbBar(props: { desk: SnapshotDesk }) {
  const st = useDesk(props.desk);
  if (!st.ab.active) return null;
  const ro = !!props.desk.readOnly;
  const can = mayToggle(st, ro);
  const waiting = st.guard?.phase === "waiting";
  const side = st.ab.side === "b" ? "B" : "A";
  return (
    <div class="sys-scenes ab-bar" role="group" aria-label="A/B compare">
      <div class="sys-scene-row">
        <span class="sys-label">A/B</span>
        <span class={`ab-side${st.ab.side === "a" ? " on" : ""}`}>A: {st.ab.aName}</span>
        <button
          type="button"
          class="sys-scene ab-toggle"
          disabled={!can}
          aria-keyshortcuts={abKeyHint().aria}
          title={waiting ? "Finish the safe apply first: Keep or Revert" : `Switch to ${side === "A" ? "B" : "A"} (${abKeyHint().text})`}
          onClick={() => void props.desk.abToggle()}
        >
          Now {side} · switch
        </button>
        <span class={`ab-side${st.ab.side === "b" ? " on" : ""}`}>B: {st.ab.bName}</span>
        <span class="ab-keep">
          <button type="button" class="sys-scene" disabled={!can} onClick={() => void props.desk.abKeep("a")}>
            Keep A
          </button>
          <button type="button" class="sys-scene" disabled={!can} onClick={() => void props.desk.abKeep("b")}>
            Keep B
          </button>
        </span>
      </div>
      {waiting ? <p class="sys-note">Finish the safe apply first: Keep or Revert.</p> : null}
    </div>
  );
}

/** A guarded apply's countdown with Keep and Revert, while it waits. */
export function GuardBar(props: { desk: SnapshotDesk }) {
  const st = useDesk(props.desk);
  const g = st.guard;
  if (!g || g.phase !== "waiting") return null;
  const left = secondsLeft(g);
  const part = g.timeoutMs > 0 ? Math.max(0, Math.min(1, g.remainingMs / g.timeoutMs)) : 0;
  return (
    <div class="guard-bar" role="alert" aria-label="Safe apply">
      <div class="guard-text">
        <strong>{g.label || "Applied"}</strong>
        <span>
          {" "}
          · reverts in <span class="guard-left">{left}</span> s unless you keep it{g.hub ? " (the hub reverts it if this page goes away)" : ""}
        </span>
      </div>
      <progress class="guard-progress" max={1} value={part} aria-label="Time left" />
      <div class="guard-buttons">
        <button type="button" class="primary" onClick={() => void props.desk.keep()}>
          Keep
        </button>
        <button type="button" onClick={() => void props.desk.revert()}>
          Revert
        </button>
      </div>
    </div>
  );
}

/** What the last action did, until dismissed or the next. */
export function OutcomeLine(props: { desk: SnapshotDesk; outcome: Outcome | null }) {
  const o = props.outcome;
  if (!o) return null;
  return (
    <div class={`desk-outcome ${o.tone}`} role={o.tone === "error" ? "alert" : "status"} aria-label="Result">
      <div class="desk-outcome-head">
        <strong>{o.title}</strong>
        <button type="button" class="dismiss" aria-label="Dismiss" onClick={() => props.desk.dismiss()}>
          ×
        </button>
      </div>
      {o.lines.length ? (
        <ul class="change-list">
          {o.lines.slice(0, 15).map((l, i) => (
            <li key={i}>{l}</li>
          ))}
          {o.lines.length > 15 ? <li>…and {o.lines.length - 15} more</li> : null}
        </ul>
      ) : null}
      {o.notes.length ? (
        <ul class="change-list notes" aria-label="Notes">
          {o.notes.map((l, i) => (
            <li key={i}>{l}</li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
