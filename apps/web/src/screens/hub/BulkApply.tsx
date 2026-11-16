// WP P3WGR: a group's bulk apply (PLAN-P3 F5, F8): one of the hub's snapshots (any member's, as
// the hub's store keeps them: `hubSnapshotsList` per member) applied to every member, one after
// another at the hub (`hubGroupApply`, Control and Admin). With "Revert unless kept" each device
// gets its own guard and the page shows a bar counting down to the hub's `confirm_by`: Keep
// (`hubGroupApplyConfirm`) or Revert Now (`hubGroupApplyCancel`) settle every guard of the run;
// unsettled, each device reverts at its own deadline. Settling passes the client read-only
// mode (it writes nothing new). The rows stay until dismissed.

import { useEffect, useState } from "preact/hooks";
import type { HubGroupRecord, HubGroupRunRecord } from "../../core/protocol";
import type { SnapshotInfoRecord } from "../../core/hub-settings-protocol";
import type { Session } from "../../ui/session";
import type { HubView } from "./feed";
import { ResultsPanel, writeOff } from "./GroupActions";
import {
  APPLY_TIMEOUT_DEFAULT_S,
  APPLY_TIMEOUT_MAX_S,
  APPLY_TIMEOUT_MIN_S,
  GROUP_MEMBERS_MAX,
  applyOk,
  confirmOk,
  countdownText,
  failureWords,
  resultLines,
  runPending,
  secondsLeft,
  targetOf,
  timeoutFrom,
  type ResultSet,
} from "./groups-model";

/** The hub's snapshots of the group's members, newest first, each once. */
async function memberSnapshots(session: Session, members: readonly string[]): Promise<SnapshotInfoRecord[]> {
  const byId = new Map<string, SnapshotInfoRecord>();
  for (const serial of members.slice(0, GROUP_MEMBERS_MAX)) {
    try {
      const list = await session.core.call("hubSnapshotsList", { serial });
      for (const s of Array.isArray(list) ? list : []) if (s && typeof s.id === "string") byId.set(s.id, s);
    } catch {
      // A member without snapshots (or one the hub refuses to list) adds none.
    }
  }
  return [...byId.values()].sort((a, b) => b.savedMs - a.savedMs);
}

export function BulkApply(p: { session: Session; view: HubView; group: HubGroupRecord; name: (serial: string) => string; onDone: () => void }) {
  const { session, view, group } = p;
  const off = writeOff(view.role, session.readOnlyReason);
  const [snaps, setSnaps] = useState<SnapshotInfoRecord[] | null>(null);
  const [pick, setPick] = useState("");
  const [guard, setGuard] = useState(true);
  const [timeout, setTimeoutText] = useState(String(APPLY_TIMEOUT_DEFAULT_S));
  const [run, setRun] = useState<HubGroupRunRecord | null>(null);
  const [result, setResult] = useState<ResultSet | null>(null);
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const key = group.members.join(",");
  const load = async () => {
    const list = await memberSnapshots(session, group.members);
    setSnaps(list);
    setPick((had) => (list.some((s) => s.id === had) ? had : (list[0]?.id ?? "")));
  };
  useEffect(() => {
    setSnaps(null);
    void load();
  }, [session, group.id, key]);
  // The hub's `snapshot.changed`: the list again.
  useEffect(() => session.watchHub((events) => void (events.some((e) => e.kind === "snapshot.changed") && load())), [session, group.id, key]);
  // The countdown, while a run waits.
  const pending = !!run && !!run.confirmBy && runPending(run.results);
  useEffect(() => {
    if (!pending) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [pending, run?.run]);

  const left = pending ? secondsLeft(run!.confirmBy, now) : null;
  const timedOut = pending && left === 0;
  const secs = timeoutFrom(timeout);

  const apply = async () => {
    const s = snaps?.find((x) => x.id === pick);
    if (!s) return;
    setBusy(true);
    setRun(null);
    const title = `${s.name || "Snapshot"} on ${group.name}${guard ? " (revert unless kept)" : ""}`;
    try {
      const r = await session.core.call("hubGroupApply", { target: targetOf(group), snapshot: s.id, ...(guard ? { guard: true, timeoutMs: (secs ?? APPLY_TIMEOUT_DEFAULT_S) * 1000 } : {}) });
      setRun(r);
      setNow(Date.now());
      setResult({ title, lines: resultLines(r.results, p.name, applyOk), note: r.msg });
    } catch (e) {
      setResult({ title, lines: [], note: failureWords(e).text });
    } finally {
      setBusy(false);
      p.onDone();
    }
  };
  const settle = async (keep: boolean) => {
    if (!run) return;
    setBusy(true);
    const title = keep ? `Kept the changes on ${group.name}` : `Reverted the changes on ${group.name}`;
    try {
      const r = await session.core.call(keep ? "hubGroupApplyConfirm" : "hubGroupApplyCancel", { run: run.run });
      setRun(null);
      setResult({ title, lines: resultLines(r.results, p.name, confirmOk), note: r.msg });
    } catch (e) {
      setResult({ title, lines: [], note: failureWords(e).text });
    } finally {
      setBusy(false);
      p.onDone();
    }
  };
  const waiting = run ? run.results.filter((r) => r.ok && r.guard !== null && r.guard !== undefined && !r.reverted).length : 0;

  return (
    <div class="hub-bulk" aria-label="Bulk apply">
      <h3>Apply a snapshot to the group</h3>
      {off ? <p class="note">{off}.</p> : null}
      {snaps && !snaps.length ? <p class="note">The hub keeps no snapshot of this group's DSPis. Save one to the hub from the Snapshots screen.</p> : null}
      <div class="hub-edit hub-bulk-form">
        <label>
          Snapshot on the hub
          <select value={pick} disabled={!snaps?.length || !!off} onChange={(e) => setPick((e.target as HTMLSelectElement).value)}>
            {(snaps ?? []).map((s) => (
              <option key={s.id} value={s.id}>
                {`${s.name || s.id} (${p.name(s.serial)})`}
              </option>
            ))}
          </select>
        </label>
        <label class="hub-check">
          <input type="checkbox" checked={guard} disabled={!!off} onChange={(e) => setGuard((e.target as HTMLInputElement).checked)} />
          Revert unless kept
        </label>
        {guard ? (
          <label>
            Keep within (seconds)
            <input
              type="number"
              min={APPLY_TIMEOUT_MIN_S}
              max={APPLY_TIMEOUT_MAX_S}
              value={timeout}
              disabled={!!off}
              aria-invalid={secs === null}
              onInput={(e) => setTimeoutText((e.target as HTMLInputElement).value)}
            />
          </label>
        ) : null}
        <div class="hub-edit-actions">
          <button type="button" disabled={busy || !!off || !pick || pending || !group.members.length || (guard && secs === null)} title={off || (pending ? "Keep or revert the last apply first" : undefined)} onClick={() => void apply()}>
            Apply to the Group
          </button>
        </div>
      </div>
      {pending ? (
        <div class="guard-bar hub-confirm-bar" role="alert" aria-label="Keep the group's changes">
          <span>
            {timedOut
              ? "The time to keep ran out: the hub reverts each DSPi at its own deadline."
              : `Keep the changes on ${waiting} ${waiting === 1 ? "DSPi" : "DSPis"}? They revert in `}
            {timedOut ? null : <strong class="hub-countdown">{countdownText(left ?? 0)}</strong>}
            {timedOut ? null : "."}
          </span>
          <div class="hub-edit-actions">
            <button type="button" disabled={busy || timedOut} onClick={() => void settle(true)}>
              Keep
            </button>
            <button type="button" class="danger-text" disabled={busy} onClick={() => void settle(false)}>
              Revert Now
            </button>
          </div>
        </div>
      ) : null}
      {result ? <ResultsPanel result={result} label="Bulk apply result" onDismiss={() => setResult(null)} /> : null}
    </div>
  );
}
