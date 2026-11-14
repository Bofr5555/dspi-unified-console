// WP P1WB2: the one lock prompt (PRO1 S4, OD-P1d) and the locked-user front page's banner.
//
// The lock is the core's `LockState`, read with `installerLockRefresh`: `heldBy` says whose it
// is, `hub` (the hub this DSPi is reached through keeps the PIN: unlocking asks the hub) or
// `console`. The web keeps no PIN of its own, so on the web a lock is always the hub's. The
// dialog says in one line that the lock is on the hub (or this console), not on the DSPi:
// the core's notice row for `scopeNoticeKey`. With no PIN set nothing is locked and nothing is
// asked: neither of these shows.

import { useState } from "preact/hooks";
import type { LockStateRecord } from "../../core/protocol";
import type { Session } from "../../ui/session";
import { noticeRow } from "../../ui/texts";
import { Dialog } from "../main/Dialog";
import { lockLine } from "./model";

/** The scope line: where the lock is, in the core's words. */
export function scopeLine(lock: LockStateRecord | null | undefined): string {
  if (!lock) return "";
  const n = noticeRow(lock.scopeNoticeKey);
  if (n) return n.reason;
  return lock.heldBy === "hub" ? "The hub holds this DSPi's installer lock." : "This lock is on this console; the DSPi itself is open.";
}

/** The PIN prompt. The PIN is checked off this page's thread (the core's Worker; on a hub, the hub). */
export function UnlockDialog(props: { session: Session; lock: LockStateRecord | null | undefined; onDone: (unlocked: boolean) => void }) {
  const [pin, setPin] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const submit = async () => {
    if (!pin || busy) return;
    setBusy(true);
    const why = await props.session.unlock(pin);
    setBusy(false);
    setPin("");
    if (!why) props.onDone(true);
    else setError(why);
  };
  const wait = props.lock?.retryAfterMs ?? 0;
  return (
    <Dialog
      title="Unlock"
      onCancel={() => props.onDone(false)}
      buttons={[
        { label: busy ? "Checking…" : "Unlock", onClick: () => void submit() },
        { label: "Cancel", onClick: () => props.onDone(false) },
      ]}
    >
      <p class="lock-scope">{scopeLine(props.lock)}</p>
      <form
        class="pin-entry"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <label>
          Installer PIN
          <input type="password" autoComplete="off" value={pin} onInput={(e) => setPin((e.target as HTMLInputElement).value)} />
        </label>
      </form>
      {error ? (
        <p class="error" role="alert">
          {error}
        </p>
      ) : null}
      {wait > 0 ? <p class="note">Try again in {Math.ceil(wait / 1000)} s.</p> : null}
    </Dialog>
  );
}

/** The locked-user front page's banner: the installer's note and contact, and Unlock. */
export function LockedBanner(props: { session: Session; lock: LockStateRecord }) {
  const [asking, setAsking] = useState(false);
  const l = props.lock;
  return (
    <section class="card locked-banner" aria-label="Locked">
      <p>
        <strong>{lockLine(l)}</strong>
      </p>
      {l.note ? <p class="locked-note">{l.note}</p> : null}
      {l.contact ? <p class="note">Installer: {l.contact}</p> : null}
      <button type="button" onClick={() => setAsking(true)}>
        Unlock…
      </button>
      {asking ? <UnlockDialog session={props.session} lock={l} onDone={() => setAsking(false)} /> : null}
    </section>
  );
}
