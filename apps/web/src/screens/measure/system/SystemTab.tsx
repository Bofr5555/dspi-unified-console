// WP ATWEB: the Measure screen's "Several units" tab (PLAN-AT 1.4 to 1.6): an alignment group
// across units of our hub, measured with one microphone, each unit's own generator started on
// the shared time base. The placement first (nothing plays before the person chose the listening
// position), the units and their outputs, the check (every unit judged by its own connection),
// the run with Stop always in view and the settle wait in words, the result (arrivals with their
// uncertainty, the delays across units, the transitions) drawn with P5WMS's graph, the proposal,
// the summed check, and for Admin the apply, the verify, Keep (asked twice after a failed verify)
// and Revert. Control measures and sees; it never gets Apply or Revert.

import { useEffect, useMemo, useState } from "preact/hooks";
import type { MeasureDocCurvesRecord, MeasureRole } from "../../../core/measure-protocol";
import { noticeRow } from "../../../ui/texts";
import { CONNECT } from "../../../ui/shell/sections"; // -- WP INTD --
import { MeasureGraph, type Series } from "../graph";
import { ROLES } from "../model";
import type { MeasureTab, MeasureTabProps } from "../MeasureScreen";
import { SystemDesk } from "./desk";
import {
  KEYS,
  SYSTEM_PLACEMENTS,
  arrivalWords,
  endedWords,
  guardWords,
  issueWords,
  outputLabel,
  phaseWords,
  placementProblem,
  proposalRights,
  runsWords,
  shownIssue,
  us,
  withOutput,
  withRole,
  withUnit,
  type SystemPlacement,
} from "./model";
import { EmptyState } from "../../../ui/design"; // -- WP WEB --

const col = { display: "flex", flexDirection: "column", gap: "8px", minWidth: 0 } as const;
const row = { display: "flex", flexWrap: "wrap", gap: "8px 16px", alignItems: "flex-end" } as const;
const field = { display: "flex", flexDirection: "column", gap: "2px", minWidth: 0 } as const;

/** Re-render on the desk's changes, and once a second while the second Keep's question shows. */
function useSystemDesk(d: SystemDesk): SystemDesk {
  const [, bump] = useState(0);
  useEffect(() => d.watch(() => bump((n) => n + 1)), [d]);
  useEffect(() => {
    if (!d.question) return;
    const t = setInterval(() => bump((n) => n + 1), 250);
    return () => clearInterval(t);
  }, [d, d.question]);
  // -- review AT-D2 -- and while the hub's guard of an apply counts down
  const guarded = d.guarded;
  useEffect(() => {
    if (!guarded) return;
    const t = setInterval(() => bump((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [d, guarded]);
  return d;
}

function Notice(props: { k: string; text?: string; label?: string }) {
  const r = noticeRow(props.k);
  return (
    <p class={r?.kind === "warning" || r?.kind === "error" ? "notice-row notice-row-warning" : "m-caption"} role="note" aria-label={props.label ?? r?.title ?? "Notice"} data-key={props.k}>
      <strong>{r?.title ?? ""}</strong> {props.text ?? (r ? `${r.reason} ${r.action}` : "")}
    </p>
  );
}

export function SystemTabView(props: MeasureTabProps) {
  const { session, desk: screen } = props;
  const d = useMemo(() => new SystemDesk(session), [session]);
  useSystemDesk(d);
  useEffect(() => {
    void d.enter();
    return () => void d.leave();
  }, [d]);
  // The microphone and its calibration are the Measure tab's choice (P5WMS's desk).
  d.micDeviceId = screen.mic.kind === "browser" ? screen.mic.deviceId : "";
  d.micDocument = screen.draft?.micDocument ?? screen.micDocument;

  // -- WP WEB -- the empty state of DL2 (Qt's words), keeping the core's notice key
  if (d.unavailable)
    return (
      <section aria-label="Several units">
        <div role="note" aria-label={noticeRow(KEYS.needsHub)?.title ?? "Notice"} data-key={KEYS.needsHub}>
          <EmptyState
            icon="network"
            title={noticeRow(KEYS.needsHub)?.title ?? "Measuring several units needs the DSPi hub"}
            text="The hub starts every unit's generator on one shared clock. Connect through the DSPi hub, or measure one unit at a time."
            // -- WP INTD -- Qt's "Hubs…": the page that connects through a hub (Devices › Open a DSPi)
            action={props.go ? { label: "Hubs…", run: () => props.go?.(CONNECT) } : undefined}
          />
        </div>
      </section>
    );
  const busy = !!d.busy || d.running;
  const draft = d.draft;
  const placement = placementProblem(draft.placement);
  return (
    // -- WP WEB4 -- DL2's Several units on a hub: no card, the run bar in the notice look with a secondary Stop
    <section class="measure-units measure-column" aria-label="Several units" style={col}>
      {d.running ? (
        <div class="notice-row notice-row-info measure-stopbar" role="status" aria-label="Measuring across units">
          <span class="notice-text" aria-label="Phase">
            {phaseWords(d.state)}
          </span>
          {/* -- WP ATDEMO -- */}
          {d.simOn ? (
            <span class="muted" aria-label="Microphone across units">
              {d.sim?.label}
            </span>
          ) : null}
          <button type="button" onClick={() => void d.stop()}>
            Stop
          </button>
        </div>
      ) : null}
      <p class="m-caption">
        Every unit plays its own generator, each start on its sample of the units' shared clock, and one microphone hears them all. The hub stops every unit if anything changes.
      </p>
      <label style={field}>
        <span>Where is the microphone?</span>
        <select aria-label="Placement across units" value={draft.placement} disabled={busy} onChange={(e) => d.setDraft({ ...draft, placement: (e.currentTarget as HTMLSelectElement).value as SystemPlacement })}>
          <option value="">Choose…</option>
          {SYSTEM_PLACEMENTS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
            </option>
          ))}
        </select>
        {placement ? (
          <span class="muted" aria-label="Placement problem" data-key={KEYS.placement}>
            {placement}
          </span>
        ) : null}
      </label>
      {/* -- WP ATDEMO -- the demo hub's simulated microphone, where it has one */}
      {d.sim?.available ? (
        <label>
          <input type="checkbox" aria-label="Simulated microphone across units" checked={d.useSim} disabled={busy} onChange={(e) => d.setUseSim((e.currentTarget as HTMLInputElement).checked)} /> Use the {d.sim.label.toLowerCase()} (the demo hub's room hears its demo units; nothing plays anywhere)
        </label>
      ) : null}
      <UnitsPicker d={d} busy={busy} openDevice={session.openDevice} />
      <div style={row}>
        <label style={field}>
          <span>Sweep, s</span>
          <input type="number" aria-label="Sweep length across units" min={1} max={15} step={0.5} value={draft.sweepS} disabled={busy} onInput={(e) => d.setDraft({ ...draft, sweepS: Number((e.currentTarget as HTMLInputElement).value) })} />
        </label>
        <label style={field}>
          <span>Passes</span>
          <select aria-label="Passes across units" value={String(draft.passes)} disabled={busy} onChange={(e) => d.setDraft({ ...draft, passes: Number((e.currentTarget as HTMLSelectElement).value) })}>
            <option value="0">The plan's (5, or 3 for a long group)</option>
            {[2, 3, 4, 5, 6, 7, 8].map((n) => (
              <option key={n} value={String(n)}>
                {n}
              </option>
            ))}
          </select>
        </label>
        <label style={field}>
          <span>Anchor output</span>
          <select aria-label="Anchor output" value={draft.anchor} disabled={busy} onChange={(e) => d.setDraft({ ...draft, anchor: (e.currentTarget as HTMLSelectElement).value })}>
            <option value="">The first unit's first output</option>
            {draft.units.flatMap((u) =>
              u.outputs.map((o) => (
                <option key={`${u.serial}:${o.output}`} value={`${u.serial}:${o.output}`}>
                  {outputLabel(u.serial, o.output)}
                </option>
              )),
            )}
          </select>
        </label>
        <label style={{ ...field, flex: "1 1 10em" }}>
          <span>Name</span>
          <input type="text" aria-label="Name of the measurements" value={draft.name} disabled={busy} onInput={(e) => d.setDraft({ ...draft, name: (e.currentTarget as HTMLInputElement).value })} />
        </label>
      </div>
      <div style={row}>
        <button type="button" disabled={busy || !!d.blocker} onClick={() => void d.check()}>
          Check every unit
        </button>
        <button type="button" class="primary" disabled={busy || !!d.blocker} onClick={() => void d.start()}>
          Measure the units
        </button>
        {d.busy ? <span class="muted">{d.busy}…</span> : null}
      </div>
      {d.preflight ? <PreflightList p={d.preflight} /> : null}
      {d.running && runsWords(d.state) ? <Notice k={KEYS.runs} text={runsWords(d.state)} label="Runs" /> : null}
      {d.error ? (
        <p class="error" role="alert" data-key={d.errorKey}>
          {d.error}
        </p>
      ) : null}
      {d.said ? <p role="status">{d.said}</p> : null}
      {!d.running && d.state?.issue ? <StoppedIssue issue={d.state.issue} /> : null}
      {d.result ? <ResultView d={d} /> : null}
      {d.proposal ? <ProposalView d={d} busy={busy} /> : null}
    </section>
  );
}

function StoppedIssue(props: { issue: NonNullable<NonNullable<SystemDesk["state"]>["issue"]> }) {
  const i = shownIssue(props.issue);
  return (
    <p class="error" role="alert" aria-label="Why it stopped" data-key={i.key}>
      {i.text}
    </p>
  );
}

function UnitsPicker(props: { d: SystemDesk; busy: boolean; openDevice?: ((serial: string) => void) | null }) {
  const { d, busy } = props;
  const draft = d.draft;
  if (!d.units.length) return <p class="muted">The hub shares no DSPi with this console.</p>;
  return (
    <fieldset style={{ ...col, border: "none", padding: 0, margin: 0 }} aria-label="Units">
      <legend>Units of the group</legend>
      {d.units.map((u) => {
        const chosen = draft.units.find((x) => x.serial === u.serial);
        return (
          <div key={u.serial} style={col} aria-label={`Unit ${u.serial}`} role="group">
            <label>
              <input type="checkbox" aria-label={`Measure ${u.serial}`} checked={!!chosen} disabled={busy || !u.online} onChange={(e) => d.setDraft(withUnit(draft, u.serial, (e.currentTarget as HTMLInputElement).checked))} /> {u.name}
              {u.name !== u.serial ? <span class="muted"> ({u.serial})</span> : null}
              {u.own ? <span class="muted"> (this DSPi)</span> : null}
              {!u.online ? <span class="muted"> (offline)</span> : null}
            </label>
            {chosen ? (
              <div style={{ ...row, paddingLeft: "1.5em" }}>
                {u.opening ? <span class="muted">Opening {u.serial}…</span> : null}
                {u.error ? (
                  <span class="error" role="alert">
                    {u.error}
                  </span>
                ) : null}
                {u.outputs.map((o) => {
                  const on = chosen.outputs.find((x) => x.output === o.index);
                  return (
                    <span key={o.index} style={{ display: "inline-flex", gap: "4px", alignItems: "center", flexWrap: "wrap" }}>
                      <label>
                        <input type="checkbox" aria-label={outputLabel(u.serial, o.index)} checked={!!on} disabled={busy} onChange={(e) => d.setDraft(withOutput(draft, u.serial, o.index, (e.currentTarget as HTMLInputElement).checked))} /> {o.name}
                      </label>
                      {on ? (
                        <select aria-label={`Role of ${outputLabel(u.serial, o.index)}`} value={on.role} disabled={busy} onChange={(e) => d.setDraft(withRole(draft, u.serial, o.index, (e.currentTarget as HTMLSelectElement).value as MeasureRole))}>
                          {ROLES.map((r) => (
                            <option key={r.id} value={r.id}>
                              {r.label}
                            </option>
                          ))}
                        </select>
                      ) : null}
                    </span>
                  );
                })}
                {!u.own && !u.tab && props.openDevice ? (
                  // -- WP WEBM3 -- the desk's own core for it goes first: one session per DSPi
                  <button type="button" disabled={busy} onClick={() => void d.handOver(u.serial).then((ok) => ok && props.openDevice?.(u.serial))}>
                    Open {u.serial} in the console
                  </button>
                ) : null}
                {u.tab ? <span class="muted">Measured on its open tab ({u.tab})</span> : null /* -- WP WEBM3 -- */}
              </div>
            ) : null}
          </div>
        );
      })}
    </fieldset>
  );
}

function PreflightList(props: { p: NonNullable<SystemDesk["preflight"]> }) {
  const { p } = props;
  return (
    <ul aria-label="Preflight across units" style={{ margin: 0 }}>
      {p.clear ? <li>Ready: nothing blocks the measurement across units.</li> : null}
      {p.blockers.map((b, i) => (
        <li key={`b${i}`} class="error" data-key={b.key} data-serial={b.serial}>
          {issueWords(b)}
        </li>
      ))}
      {p.warnings.map((w, i) => (
        <li key={`w${i}`} class="muted" data-key={w.key} data-serial={w.serial}>
          {issueWords(w)}
        </li>
      ))}
    </ul>
  );
}

function ResultView(props: { d: SystemDesk }) {
  const { d } = props;
  const r = d.result!;
  const [curves, setCurves] = useState<{ serial: string; c: MeasureDocCurvesRecord }[]>([]);
  useEffect(() => {
    let alive = true;
    void Promise.all(
      r.documents.map((doc) =>
        d.session.core.call("measureDocCurves", { text: doc.document, options: { smoothing: 6 } }).then(
          (c) => ({ serial: doc.serial, c }),
          () => null,
        ),
      ),
    ).then((all) => alive && setCurves(all.filter((x): x is { serial: string; c: MeasureDocCurvesRecord } => !!x)));
    return () => {
      alive = false;
    };
  }, [r]);
  const series: Series[] = curves.flatMap((u, ui) => u.c.outputs.map((o, oi) => ({ label: outputLabel(u.serial, o.output), kind: "measured" as const, curve: o.measured, colour: ui * 4 + oi })));
  return (
    <section style={col} aria-label="Result across units">
      <h4 class="section-label">Result</h4>
      <p class={r.timingReliable ? "muted" : "error"} aria-label="Timing across units" data-key={r.timingReliable ? "" : "measure.system.timing_unreliable"}>
        {r.timingReliable ? "The timing of every run was reliable." : issueWords({ key: "measure.system.timing_unreliable", message: r.reasons[0] ?? "the capture clock wandered between the units' sweeps" })}
        {r.runs > 1 ? ` ${r.runs} runs, chained through the anchor output.` : ""}
      </p>
      {series.length ? <MeasureGraph label="Measured responses across units" series={series} unit="dB (relative)" /> : null}
      <div class="sys-scroll" style={{ overflowX: "auto" }}>
        <table aria-label="Arrivals">
          <thead>
            <tr>
              <th>Output</th>
              <th>Arrival</th>
              <th>Offered</th>
            </tr>
          </thead>
          <tbody>
            {r.arrivals.map((a) => (
              <tr key={`${a.serial}:${a.output}`}>
                <th scope="row">{outputLabel(a.serial, a.output)}</th>
                <td>{arrivalWords(a)}</td>
                <td>{a.offered ? "Yes" : a.reason || "No"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {r.delays.length ? (
        <div class="sys-scroll" style={{ overflowX: "auto" }}>
          <table aria-label="Delays across units">
            <thead>
              <tr>
                <th>From</th>
                <th>To</th>
                <th>Delay</th>
                <th>Offered</th>
              </tr>
            </thead>
            <tbody>
              {r.delays.map((x, i) => (
                <tr key={i}>
                  <td>{outputLabel(x.fromSerial, x.fromOutput)}</td>
                  <td>{outputLabel(x.toSerial, x.toOutput)}</td>
                  <td>
                    {us(x.delayUs)}
                    {x.uncertaintyUs != null ? ` ± ${us(x.uncertaintyUs)}` : ""}
                  </td>
                  <td>{x.offered ? "Yes" : x.reason || "No"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
      {r.reasons.length ? (
        <ul aria-label="Reasons" style={{ margin: 0 }}>
          {r.reasons.map((x, i) => (
            <li key={i}>{x}</li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}

function ProposalView(props: { d: SystemDesk; busy: boolean }) {
  const { d, busy } = props;
  const p = d.proposal!;
  const a = d.applied;
  const stage = a?.stage ?? "none";
  const rights = proposalRights(p, d.hubRole);
  const admin = d.hubRole === "admin";
  const q = d.question;
  const ready = d.confirmReady();
  const wait = q ? Math.max(0, Math.ceil((q.since + 1000 - Date.now()) / 1000)) : 0;
  return (
    <section style={col} aria-label="Proposal across units">
      <h4 class="section-label">Proposal</h4>
      <p class="muted">The measured arrivals go into the hub's installation; the hub's alignment then sets every unit's delays. Nothing else changes.</p>
      {p.arrivals.length ? (
        <ul aria-label="Arrivals to write" style={{ margin: 0 }}>
          {p.arrivals.map((x) => (
            <li key={`${x.serial}:${x.output}`}>
              {outputLabel(x.serial, x.output)}: {arrivalWords(x)}
            </li>
          ))}
        </ul>
      ) : null}
      {p.withheld.length ? (
        <ul aria-label="Not offered" style={{ margin: 0 }}>
          {p.withheld.map((x) => (
            <li key={`${x.serial}:${x.output}`} class="muted">
              {outputLabel(x.serial, x.output)}: {x.reason || "not offered"}
            </li>
          ))}
        </ul>
      ) : null}
      {p.transitions.length ? (
        <ul aria-label="Transitions" style={{ margin: 0 }}>
          {p.transitions.map((t, i) => (
            <li key={i} class={t.checked && !t.passed ? "error" : undefined}>
              {outputLabel(t.aSerial, t.aOutput)} with {outputLabel(t.bSerial, t.bOutput)}, {Math.round(t.bandLoHz)} to {Math.round(t.bandHiHz)} Hz: {t.message || (t.checked ? (t.passed ? "the summed check agreed" : "the summed check did not agree") : "not checked yet")}
            </li>
          ))}
        </ul>
      ) : null}
      {p.notice ? (
        <p class="muted" role="note" data-key={p.notice.key}>
          {issueWords(p.notice)}
        </p>
      ) : null}
      {/* -- review AT-D2 -- WP ATGUARD: what the hub does by itself after the apply, before it is pressed */}
      {p.guardNote && p.offered && admin && (stage === "none" || stage === "reverted" || stage === "revert_incomplete" || stage === "kept") ? (
        <p class={(p.guardS ?? 0) > 0 ? "muted" : "error"} role="note" aria-label="Guard of the apply">
          {p.guardNote.charAt(0).toUpperCase() + p.guardNote.slice(1)}.
        </p>
      ) : null}
      {stage === "none" || stage === "reverted" || stage === "revert_incomplete" || stage === "kept" ? (
        <div style={row}>
          {/* -- WP ATDEMO -- the core offers nothing before the summed check passed: the check is
              offered while there are arrivals to write */}
          {p.sumCheckNeeded && p.arrivals.length > 0 && !p.withdrawn ? (
            <button type="button" disabled={busy} onClick={() => void d.summed("sum")}>
              Run the summed check
            </button>
          ) : null}
          {rights.apply ? (
            <button type="button" class="primary" disabled={busy} onClick={() => void d.apply()}>
              Apply the alignment
            </button>
          ) : rights.why ? (
            <span class="muted" aria-label="Why not applied" data-key={!admin && p.offered && !p.sumCheckNeeded ? KEYS.admin : ""}>
              {rights.why}
            </span>
          ) : null}
        </div>
      ) : null}
      {a && (stage === "applied" || stage === "verified") ? (
        <div style={col} aria-label="Applied alignment">
          <p role="status">
            {stage === "applied"
              ? "Applied: the summed runs are measured again with the new delays to verify it."
              : a.verified
                ? "Verified: the summed runs agree with the new delays."
                : issueWords({ key: KEYS.verifyFailed, message: a.message || "the summed runs did not agree with the new delays" })}
          </p>
          {/* -- review AT-D2 -- the hub's guard, counting down (or that this hub has none) */}
          <p class={(a.guardS ?? 0) > 0 && d.guardLeftS() !== 0 ? "muted" : "error"} role="timer" aria-label="Guard countdown">
            {guardWords(a.guardS ?? 0, d.guardLeftS())}
          </p>
          <div style={row}>
            {stage === "applied" ? (
              <button type="button" class="primary" disabled={busy} onClick={() => void d.summed("verify")}>
                Verify
              </button>
            ) : (
              <button type="button" class="primary" disabled={busy || !!q} onClick={() => void d.keep()}>
                Keep
              </button>
            )}
            {admin ? (
              <button type="button" class="danger" disabled={busy} onClick={() => void d.revert()}>
                Revert
              </button>
            ) : null}
          </div>
          {q ? (
            <div role="alertdialog" aria-label="Keep anyway?" class="notice-row notice-row-warning m-question" style={col}>
              <p>{q.text}</p>
              <div style={row}>
                <button type="button" class="danger" disabled={!ready || busy} onClick={() => void d.confirmKeep()}>
                  {ready ? "Keep anyway" : `Keep anyway (in ${wait} s)`}
                </button>
                <button type="button" onClick={() => d.cancelKeep()}>
                  Not now
                </button>
              </div>
            </div>
          ) : null}
        </div>
      ) : null}
      {/* -- review AT-D2 -- the core's words: the hub's own when its guard ended it (it may not have put everything back) */}
      {/* -- WP REVERTW -- a revert not fully successful has its own stage: an alert in the notice's words */}
      {a && (stage === "kept" || stage === "reverted" || stage === "revert_incomplete") ? (
        <p
          role={stage === "revert_incomplete" ? "alert" : "status"}
          aria-label="How the apply ended"
          data-key={stage === "revert_incomplete" ? a.revertNotice || "revert.unknown" : undefined}
          class={stage === "revert_incomplete" || (stage === "reverted" && a.writeFailures.length > 0) ? "error" : undefined}
        >
          {endedWords(a)}
        </p>
      ) : null}
    </section>
  );
}

/** The tab in P5WMS's slot (MeasureScreen.tsx). Not offered to a connection that may not play. */
export const SYSTEM_TABS: MeasureTab[] = [{ id: "system", label: "Several units", View: SystemTabView }];
