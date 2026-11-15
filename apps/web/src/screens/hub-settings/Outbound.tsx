// WP P2WB2: notifications out (`outbound.get`, `outbound.set`, `outbound.test`; Admin). Each target
// is a webhook (a signed JSON POST) or a push (a text POST), picks the events it gets, and is sent
// over https (the public CAs, a CA or certificate of its own, or a key pin) or, marked unencrypted,
// over http to a private, loopback or link-local address only: the hub checks that on every send.
// The list is saved whole; a new secret is made by the hub and shown here once.

import { useEffect, useState } from "preact/hooks";
import type { HubOutboundTargetRecord, HubOutboundTestRecord } from "../../core/hub-settings-protocol";
import { failure } from "../installer/HubAdmin";
import { EVENT_KINDS, lastText, newTarget, outboundSetFrom, targetsForSet, transportText, type NewSecret } from "./model";
import { Done, Problem, type SectionProps } from "./parts";

export function Outbound(props: SectionProps) {
  const { session, edit } = props;
  const [targets, setTargets] = useState<HubOutboundTargetRecord[] | null>(null);
  const [problem, setProblem] = useState("");
  const [done, setDone] = useState("");
  const [secrets, setSecrets] = useState<NewSecret[]>([]);
  const [tested, setTested] = useState<Record<string, HubOutboundTestRecord>>({});
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const read = async () => {
    try {
      setTargets(await session.core.call("hubOutboundGet", undefined));
      setDirty(false);
      setProblem("");
    } catch (e) {
      setTargets(null);
      setProblem(failure(e));
    }
  };
  useEffect(() => {
    void read();
  }, [session]);
  const change = (i: number, patch: Partial<HubOutboundTargetRecord>) => {
    setTargets((l) => (l ? l.map((t, j) => (j === i ? { ...t, ...patch } : t)) : l));
    setDirty(true);
    setDone("");
  };
  const save = async () => {
    if (!targets) return;
    setBusy(true);
    try {
      const r = outboundSetFrom(await session.core.call("hubOutboundSet", { targets: targetsForSet(targets) }));
      setTargets(r.targets);
      setSecrets(r.secrets);
      setDirty(false);
      setProblem("");
      setDone("The hub's notification targets are saved.");
    } catch (e) {
      setProblem(failure(e));
    } finally {
      setBusy(false);
    }
  };
  const test = async (t: HubOutboundTargetRecord) => {
    setBusy(true);
    try {
      const r = await session.core.call("hubOutboundTest", { id: t.id });
      setTested((x) => ({ ...x, [t.id]: r }));
      setProblem("");
      // Its `last` is the test's now.
      const now = await session.core.call("hubOutboundGet", undefined).catch(() => null);
      if (now && !dirty) setTargets(now);
    } catch (e) {
      setProblem(failure(e));
    } finally {
      setBusy(false);
    }
  };
  const disabled = !edit || busy;
  return (
    <section class="card hub-section" aria-label="Notifications">
      <h3>Notifications</h3>
      <p class="note">
        The hub tells a webhook or a push service when an alarm is raised or cleared, a DSPi restarts or a schedule runs. Each
        request is signed with the target's secret.
      </p>
      {/* -- HOOK SECWEB (PLAN-SEC SEC-C) -- a receiver's certificate fingerprint, once the hub reports it, goes beside "Sent" */}
      <Problem text={problem} />
      {secrets.map((s) => (
        <div key={s.target} class="card limits-report warn hub-secret" role="status" aria-label="New secret">
          <strong>The secret for “{s.name}”</strong>
          <p class="recovery-code">
            <code>{s.secret.reveal()}</code>
          </p>
          <p class="note">It is shown only now. The receiver checks the X-DSPi-Signature header with it.</p>
          <button type="button" onClick={() => setSecrets((l) => l.filter((x) => x !== s))}>
            Done
          </button>
        </div>
      ))}
      {targets ? (
        <>
          {targets.length ? null : <p class="note">No targets: the hub sends nothing.</p>}
          {targets.map((t, i) => (
            <TargetForm
              key={t.id || `new-${i}`}
              t={t}
              n={i}
              disabled={disabled}
              tested={t.id ? tested[t.id] : undefined}
              canTest={!!t.id && !dirty && !disabled}
              onChange={(p) => change(i, p)}
              onRemove={() => {
                setTargets((l) => (l ? l.filter((_, j) => j !== i) : l));
                setDirty(true);
              }}
              onTest={() => void test(t)}
            />
          ))}
          <div class="installer-buttons">
            <button
              type="button"
              disabled={disabled || targets.length >= 16}
              onClick={() => {
                setTargets([...targets, newTarget()]);
                setDirty(true);
              }}
            >
              Add Target
            </button>
            <button type="button" disabled={disabled || !dirty} onClick={() => void save()}>
              Save Targets
            </button>
            {dirty ? (
              <button type="button" disabled={busy} onClick={() => void read()}>
                Discard Changes
              </button>
            ) : null}
          </div>
          <Done text={done} />
        </>
      ) : null}
    </section>
  );
}

function TargetForm(props: {
  t: HubOutboundTargetRecord;
  n: number;
  disabled: boolean;
  tested: HubOutboundTestRecord | undefined;
  canTest: boolean;
  onChange: (p: Partial<HubOutboundTargetRecord>) => void;
  onRemove: () => void;
  onTest: () => void;
}) {
  const { t, disabled, onChange } = props;
  const label = t.name || t.id || `New target ${props.n + 1}`;
  const trust = t.pinSha256 ? "pin" : t.caPem ? "ca" : "public";
  return (
    <fieldset class="hub-target" aria-label={label}>
      <legend>{label}</legend>
      {t.notice ? (
        <p class="note warn" role="note">
          {t.notice}
        </p>
      ) : null}
      <div class="power-rows">
        <label class="limit-field">
          <span>Name</span>
          <input value={t.name} maxLength={64} disabled={disabled} onInput={(e) => onChange({ name: (e.target as HTMLInputElement).value })} />
        </label>
        <label class="limit-field">
          <span>Kind</span>
          <select aria-label="Kind" value={t.kind} disabled={disabled} onChange={(e) => onChange({ kind: (e.target as HTMLSelectElement).value })}>
            <option value="webhook">Webhook (JSON)</option>
            <option value="push">Push (text)</option>
          </select>
        </label>
        <label class="limit-field">
          <span>On</span>
          <input type="checkbox" checked={t.enabled} disabled={disabled} onChange={(e) => onChange({ enabled: (e.target as HTMLInputElement).checked })} />
        </label>
      </div>
      <label class="limit-field hub-wide">
        <span>URL</span>
        <input type="url" value={t.url} maxLength={2048} disabled={disabled} onInput={(e) => onChange({ url: (e.target as HTMLInputElement).value })} />
      </label>
      <fieldset class="hub-days" aria-label="Events">
        {EVENT_KINDS.map((k) => (
          <label key={k.id}>
            <input
              type="checkbox"
              checked={t.kinds.includes(k.id)}
              disabled={disabled}
              onChange={(e) => {
                const on = (e.target as HTMLInputElement).checked;
                onChange({ kinds: EVENT_KINDS.map((x) => x.id).filter((x) => (x === k.id ? on : t.kinds.includes(x))) });
              }}
            />{" "}
            {k.label}
          </label>
        ))}
      </fieldset>
      <div class="power-rows">
        <label class="limit-field">
          <span>Secret</span>
          <select
            aria-label="Secret"
            value={t.secret || "set"}
            disabled={disabled}
            onChange={(e) => onChange({ secret: (e.target as HTMLSelectElement).value })}
          >
            {t.id ? <option value="set">Keep as it is</option> : null}
            <option value="new">Make a new one</option>
            <option value="not set">None (unsigned)</option>
          </select>
        </label>
        <label class="limit-field">
          <span>Trust</span>
          <select
            aria-label="Trust"
            value={trust}
            disabled={disabled}
            onChange={(e) => {
              const v = (e.target as HTMLSelectElement).value;
              onChange(v === "public" ? { caPem: "", pinSha256: "" } : v === "ca" ? { pinSha256: "", caPem: t.caPem || "-----BEGIN CERTIFICATE-----\n" } : { caPem: "", pinSha256: t.pinSha256 || "sha256//" });
            }}
          >
            <option value="public">Public CAs</option>
            <option value="ca">A CA or certificate of its own</option>
            <option value="pin">A key pin</option>
          </select>
        </label>
        <label class="limit-field">
          <span>Unencrypted (http)</span>
          <input type="checkbox" checked={t.plaintext} disabled={disabled} onChange={(e) => onChange({ plaintext: (e.target as HTMLInputElement).checked })} />
        </label>
      </div>
      {t.plaintext ? (
        <p class="note warn hub-plaintext" role="note">
          Unencrypted: the hub sends an http:// target only to a private (RFC 1918, fc00::/7), loopback or link-local address,
          checked on every send, and never across the internet. A receiver on any other address, a Tailscale one included, needs
          https://.
        </p>
      ) : null}
      {trust === "ca" ? (
        <label class="hub-wide hub-pem">
          <span>CA or self-signed certificate (PEM)</span>
          <textarea rows={4} value={t.caPem} disabled={disabled} onInput={(e) => onChange({ caPem: (e.target as HTMLTextAreaElement).value })} />
        </label>
      ) : null}
      {trust === "pin" ? (
        <label class="limit-field hub-wide">
          <span>Key pin (sha256//…)</span>
          <input value={t.pinSha256} disabled={disabled} onInput={(e) => onChange({ pinSha256: (e.target as HTMLInputElement).value })} />
        </label>
      ) : null}
      {t.id ? (
        <p class="note hub-target-state">
          Sent: {transportText(t)} · {lastText(t)}
          {t.queued ? ` · ${t.queued} waiting` : ""}
          {t.dropped ? ` · ${t.dropped} dropped` : ""}
        </p>
      ) : null}
      {props.tested ? (
        <p class={`note ${props.tested.delivered ? "" : "error"}`} role="status" aria-label="Test result">
          {props.tested.delivered
            ? `Test delivered${props.tested.status ? ` (${props.tested.status})` : ""}.`
            : `Test not delivered${props.tested.status ? ` (${props.tested.status})` : ""}${props.tested.msg ? `: ${props.tested.msg}` : "."}`}
        </p>
      ) : null}
      <div class="installer-buttons">
        <button type="button" disabled={!props.canTest} onClick={props.onTest}>
          Test
        </button>
        <button type="button" class="danger-text" disabled={disabled} aria-label={`Remove ${label}`} onClick={props.onRemove}>
          Remove
        </button>
      </div>
    </fieldset>
  );
}
