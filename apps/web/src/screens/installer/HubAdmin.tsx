// WP HUBADMIN: the Installer page's hub parts. Which copy the page edits is the core's word
// (`installerTarget`, no I/O): through a hub that keeps limits it is the HUB's limits profile,
// installer PIN and pinned modes, read with `hubLimitsGet` and changed by an Admin of the hub
// (`hubLimitsSet`, `hubLimitsClear`, `hubPinSet`, `hubPinClear`, `hubModePin`), read-only below
// Admin with the core's row saying why; a direct (USB) DSPi keeps this console's copy as before.
// Every call is the core's, in the Worker (off this page's thread). The hub's admin events
// (`linkTick`'s `hubEvents`, the session's `hubAdminRev`) make the page read the hub again.

import { useEffect, useState } from "preact/hooks";
import type { HubAdminStateRecord, InstallerTargetRecord, ModeManifest } from "../../core/protocol";
import type { Session } from "../../ui/session";
import { noticeOf, noticeRow } from "../../ui/texts";
import { hubEditable, hubLockLine, pinPairProblem, pinnedModeChoices, pinnedModeOf, pinnedRoles, roleLabel, editsHubCopy } from "./model";

/** The page's target and, through a hub, the hub's copy; read again on every hub admin event and Link phase. */
export function useHubAdmin(session: Session, rev: number, phase: string) {
  const [target, setTarget] = useState<InstallerTargetRecord | null>(null);
  const [state, setState] = useState<HubAdminStateRecord | null>(null);
  const [error, setError] = useState("");
  const read = async (live: () => boolean) => {
    let t: InstallerTargetRecord | null = await session.core.call("installerTarget", undefined).catch(() => null);
    let s: HubAdminStateRecord | null = null;
    let why = "";
    if (editsHubCopy(t) && t!.readOnlyKey !== "disconnected") {
      try {
        s = await session.core.call("hubLimitsGet", undefined);
        // The hub's live word on the role, now the core has it.
        t = (await session.core.call("installerTarget", undefined).catch(() => null)) ?? t;
      } catch (e) {
        why = failure(e);
      }
    }
    if (!live()) return;
    setTarget(t);
    setState(s);
    setError(why);
  };
  useEffect(() => {
    let live = true;
    void read(() => live);
    return () => {
      live = false;
    };
  }, [session, rev, phase]);
  /** Take the hub's answer to a change, and the target again (the role it carries). */
  const adopt = (s: HubAdminStateRecord) => {
    setState(s);
    setError("");
    void session.core.call("installerTarget", undefined).then(setTarget, () => undefined);
  };
  return { target, state, error, adopt, setError, editable: hubEditable(target, state) };
}

export type HubAdmin = ReturnType<typeof useHubAdmin>;

/** A failure in the core's words: its notice row's title and reason, else its message. */
export function failure(e: unknown): string {
  const n = noticeOf(e);
  return n ? `${n.title}: ${n.reason}` : String((e as Error)?.message ?? e);
}

/** One of the core's notice rows (title, reason, what to do), `data-key` its key. Nothing before the table is read. */
export function NoticeRow(props: { rowKey: string; label: string; cls?: string }) {
  const n = props.rowKey ? noticeRow(props.rowKey) : null;
  if (!n) return null;
  return (
    <div class={`card installer-copy notice-${n.kind} ${props.cls ?? ""}`} role="note" aria-label={props.label} data-key={n.key}>
      <strong>{n.title}.</strong> {n.reason} <span class="notice-action">{n.action}</span>
    </div>
  );
}

/** Which copy the page edits, and why it is read-only when it is. */
export function CopyNotice(props: { target: InstallerTargetRecord | null; editable: boolean }) {
  const t = props.target;
  if (!t) return null;
  const readOnly = editsHubCopy(t) ? !props.editable : !t.editable;
  const roKey = readOnly ? t.readOnlyKey || (editsHubCopy(t) ? "installer.hub_role" : "") : "";
  return (
    <>
      <NoticeRow rowKey={t.noticeKey} label="Copy edited" />
      {roKey && roKey !== t.noticeKey ? <NoticeRow rowKey={roKey} label="Read-only" cls="read-only" /> : null}
      {editsHubCopy(t) && t.role ? <p class="note hub-role">This console's role on the hub: {roleLabel(t.role)}.</p> : null}
    </>
  );
}

/** The hub's installer PIN: set (two fields) or clear; the recovery code shown once. An Admin needs no current PIN. */
export function HubPinPanel(props: { session: Session; hub: HubAdmin; covers: number }) {
  const { session, hub } = props;
  const [pin, setPin] = useState("");
  const [again, setAgain] = useState("");
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const disabled = !hub.editable || busy;
  const set = async () => {
    const why = pinPairProblem(pin, again);
    setProblem(why);
    if (why) return;
    setBusy(true);
    try {
      const r = await session.core.call("hubPinSet", { pin });
      setPin("");
      setAgain("");
      // Shown once, by the session's screens: the new PIN may close the Installer mode here.
      session.showHubRecovery(r.recoveryCode);
      hub.adopt(r.state);
      await session.refreshLock();
    } catch (e) {
      setProblem(failure(e));
    } finally {
      setBusy(false);
    }
  };
  const clear = async () => {
    setBusy(true);
    try {
      hub.adopt(await session.core.call("hubPinClear", undefined));
      session.showHubRecovery("");
      setProblem("");
      await session.refreshLock();
    } catch (e) {
      setProblem(failure(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section class="card hub-pin" aria-label="Hub installer PIN">
      <h3>Installer PIN on the hub</h3>
      <p class="lock-state">{hubLockLine(hub.state, props.covers)}</p>
      <div class="power-rows">
        <label class="limit-field">
          <span>New PIN</span>
          <input type="password" autocomplete="new-password" value={pin} disabled={disabled} onInput={(e) => setPin((e.target as HTMLInputElement).value)} />
        </label>
        <label class="limit-field">
          <span>New PIN again</span>
          <input type="password" autocomplete="new-password" value={again} disabled={disabled} onInput={(e) => setAgain((e.target as HTMLInputElement).value)} />
        </label>
      </div>
      {problem ? (
        <p class="error" role="alert">
          {problem}
        </p>
      ) : null}
      <div class="installer-buttons">
        <button type="button" disabled={disabled} onClick={() => void set()}>
          {hub.state?.pinSet ? "Change PIN" : "Set PIN"}
        </button>
        <button type="button" class="danger" disabled={disabled || !hub.state?.pinSet} onClick={() => void clear()}>
          Clear PIN
        </button>
      </div>
    </section>
  );
}

/** Each role's pinned mode on the hub, a chooser per role (none, simple, advanced, installer). */
export function HubModesPanel(props: { session: Session; hub: HubAdmin }) {
  const { session, hub } = props;
  const [manifest, setManifest] = useState<ModeManifest | null>(null);
  const [problem, setProblem] = useState("");
  useEffect(() => {
    void session.core.call("modeManifest", undefined).then(setManifest, () => setManifest(null));
  }, [session]);
  const choices = pinnedModeChoices(manifest?.modes);
  const roles = pinnedRoles(hub.state, hub.editable, hub.state?.role || hub.target?.role || "");
  const pinMode = async (role: string, mode: string) => {
    try {
      hub.adopt(await session.core.call("hubModePin", { role, mode }));
      setProblem("");
      await session.refreshLock();
    } catch (e) {
      setProblem(failure(e));
    }
  };
  return (
    <section class="card hub-modes" aria-label="Pinned modes">
      <h3>Pinned modes on the hub</h3>
      <p class="note">A pinned mode is the one every console of that role shows for this DSPi.</p>
      <div class="power-rows">
        {roles.map((role) => (
          <label class="limit-field" key={role}>
            <span>{roleLabel(role)}</span>
            <select aria-label={roleLabel(role)} value={pinnedModeOf(hub.state, role)} disabled={!hub.editable} onChange={(e) => void pinMode(role, (e.target as HTMLSelectElement).value)}>
              {choices.map((c) => (
                <option key={c.value} value={c.value}>
                  {c.label}
                </option>
              ))}
            </select>
          </label>
        ))}
      </div>
      {problem ? (
        <p class="error" role="alert">
          {problem}
        </p>
      ) : null}
    </section>
  );
}

/** The recovery code of a PIN just set on the hub, with the core's row `installer_lock.hub_recovery`; shown once, until Done. */
export function RecoveryCode(props: { session: Session; code: string }) {
  const row = noticeRow("installer_lock.hub_recovery");
  return (
    <div class="card limits-report warn hub-recovery" role="status" aria-label="Recovery code" data-key="installer_lock.hub_recovery">
      <strong>{row ? row.title : "Recovery code"}</strong>
      <p class="recovery-code">
        <code>{props.code}</code>
      </p>
      {row ? (
        <p class="note">
          {row.reason} {row.action}
        </p>
      ) : null}
      <button type="button" onClick={() => props.session.showHubRecovery("")}>
        Done
      </button>
    </div>
  );
}
