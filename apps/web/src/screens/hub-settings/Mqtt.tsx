// WP P2WB2: MQTT and Home Assistant (`mqtt.get`, `mqtt.set`, `mqtt.status`; Admin). The broker, its
// TLS (a CA or certificate of its own, or a key pin; a client certificate), and Home Assistant
// discovery with the API key its `set` topics act as (none chosen: the hub mints a Control key
// named "Home Assistant"). The password and the client key are never shown: `set` or `not set`.

import { useEffect, useState } from "preact/hooks";
import type { HubApiKeyRecord, HubMqttSettingsRecord, HubMqttStatusRecord } from "../../core/hub-settings-protocol";
import { failure } from "../installer/HubAdmin";
import { localTime, mqttForSet, mqttStateText, numberOf, roleWord } from "./model";
import { Done, Problem, type SectionProps } from "./parts";

export function Mqtt(props: SectionProps & { keys: boolean }) {
  const { session, edit } = props;
  const [s, setS] = useState<HubMqttSettingsRecord | null>(null);
  const [status, setStatus] = useState<HubMqttStatusRecord | null>(null);
  const [keys, setKeys] = useState<HubApiKeyRecord[]>([]);
  const [password, setPassword] = useState({ typed: "", clear: false });
  const [clientKey, setClientKey] = useState({ typed: "", clear: false });
  const [problem, setProblem] = useState("");
  const [done, setDone] = useState("");
  const [busy, setBusy] = useState(false);
  const read = async () => {
    try {
      setS(await session.core.call("hubMqttGet", undefined));
      setStatus(await session.core.call("hubMqttStatus", undefined).catch(() => null));
      if (props.keys) setKeys(await session.core.call("hubApiKeys", undefined).catch(() => []));
      setProblem("");
    } catch (e) {
      setS(null);
      setProblem(failure(e));
    }
  };
  useEffect(() => {
    void read();
  }, [session]);
  const patch = (p: Partial<HubMqttSettingsRecord>) => {
    setS((x) => (x ? { ...x, ...p } : x));
    setDone("");
  };
  const tls = (p: Partial<HubMqttSettingsRecord["tls"]>) => setS((x) => (x ? { ...x, tls: { ...x.tls, ...p } } : x));
  const save = async () => {
    if (!s) return;
    setBusy(true);
    try {
      setS(await session.core.call("hubMqttSet", { settings: mqttForSet(s, password, clientKey) }));
      setPassword({ typed: "", clear: false });
      setClientKey({ typed: "", clear: false });
      setStatus(await session.core.call("hubMqttStatus", undefined).catch(() => null));
      if (props.keys) setKeys(await session.core.call("hubApiKeys", undefined).catch(() => []));
      setProblem("");
      setDone("The hub's MQTT settings are saved.");
    } catch (e) {
      setProblem(failure(e));
    } finally {
      setBusy(false);
    }
  };
  const disabled = !edit || busy || !s;
  const on = s?.tls.mode === "on";
  return (
    <section class="card hub-section" aria-label="MQTT and Home Assistant">
      <h3>MQTT and Home Assistant</h3>
      <p class="note">The hub publishes its alarms and events to an MQTT broker and, with Home Assistant on, offers every DSPi there.</p>
      <Problem text={problem} />
      {status ? (
        <p class="note hub-mqtt-state" role="status" aria-label="Broker connection">
          {mqttStateText(status.state)}
          {status.state !== "off" ? (status.encrypted ? ", encrypted" : ", unencrypted") : ""}
          {status.since ? ` since ${localTime(status.since)}` : ""}
          {status.msg ? `: ${status.msg}` : ""}
          {status.queued ? ` · ${status.queued} waiting` : ""}
          {status.dropped ? ` · ${status.dropped} dropped` : ""}
        </p>
      ) : null}
      {s?.notice || status?.notice ? (
        <p class="note warn" role="note">
          {s?.notice || status?.notice}
        </p>
      ) : null}
      {/* -- HOOK SECWEB (PLAN-SEC SEC-C) -- the broker's certificate fingerprint, once the hub reports it, goes here */}
      {s ? (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void save();
          }}
        >
          <div class="power-rows">
            <label class="limit-field">
              <span>Connect</span>
              <input type="checkbox" checked={s.enabled} disabled={disabled} onChange={(e) => patch({ enabled: (e.target as HTMLInputElement).checked })} />
            </label>
            <label class="limit-field">
              <span>Broker</span>
              <input value={s.broker} maxLength={253} disabled={disabled} onInput={(e) => patch({ broker: (e.target as HTMLInputElement).value })} />
            </label>
            <label class="limit-field">
              <span>Port</span>
              <input
                type="number"
                min={1}
                max={65535}
                value={s.port == null ? "" : String(s.port)}
                placeholder={on ? "8883" : "1883"}
                disabled={disabled}
                onChange={(e) => patch({ port: numberOf((e.target as HTMLInputElement).value) })}
              />
            </label>
            <label class="limit-field">
              <span>Client id</span>
              <input value={s.clientId} maxLength={64} disabled={disabled} onInput={(e) => patch({ clientId: (e.target as HTMLInputElement).value })} />
            </label>
            <label class="limit-field">
              <span>User name</span>
              <input value={s.username} maxLength={256} autocomplete="off" disabled={disabled} onInput={(e) => patch({ username: (e.target as HTMLInputElement).value })} />
            </label>
            <label class="limit-field">
              <span>Password ({s.password === "set" ? "set" : "not set"})</span>
              <input
                type="password"
                autocomplete="new-password"
                value={password.typed}
                placeholder={s.password === "set" ? "unchanged" : ""}
                disabled={disabled || password.clear}
                onInput={(e) => setPassword({ typed: (e.target as HTMLInputElement).value, clear: false })}
              />
            </label>
            {s.password === "set" ? (
              <label class="limit-field">
                <span>Clear the password</span>
                <input type="checkbox" checked={password.clear} disabled={disabled} onChange={(e) => setPassword({ typed: "", clear: (e.target as HTMLInputElement).checked })} />
              </label>
            ) : null}
            <label class="limit-field">
              <span>Base topic</span>
              <input value={s.baseTopic} maxLength={128} disabled={disabled} onInput={(e) => patch({ baseTopic: (e.target as HTMLInputElement).value })} />
            </label>
          </div>
          <fieldset class="hub-tls" aria-label="TLS">
            <legend>
              <label>
                <input type="checkbox" checked={on} disabled={disabled} onChange={(e) => tls({ mode: (e.target as HTMLInputElement).checked ? "on" : "off" })} /> TLS
                to the broker
              </label>
            </legend>
            {on ? (
              <>
                <label class="hub-wide hub-pem">
                  <span>CA or self-signed certificate (PEM; none: the public CAs)</span>
                  <textarea rows={3} value={s.tls.caPem} disabled={disabled} onInput={(e) => tls({ caPem: (e.target as HTMLTextAreaElement).value })} />
                </label>
                <label class="limit-field hub-wide">
                  <span>Or the broker's key pin (sha256//…)</span>
                  <input value={s.tls.pinSha256} disabled={disabled} onInput={(e) => tls({ pinSha256: (e.target as HTMLInputElement).value })} />
                </label>
                <label class="hub-wide hub-pem">
                  <span>Client certificate (PEM)</span>
                  <textarea rows={3} value={s.tls.clientCertPem} disabled={disabled} onInput={(e) => tls({ clientCertPem: (e.target as HTMLTextAreaElement).value })} />
                </label>
                <label class="hub-wide hub-pem">
                  <span>Its key (PEM; {s.tls.clientKeyPem === "set" ? "set, unchanged unless given" : "not set"})</span>
                  <textarea
                    rows={3}
                    value={clientKey.typed}
                    autocomplete="off"
                    disabled={disabled || clientKey.clear}
                    onInput={(e) => setClientKey({ typed: (e.target as HTMLTextAreaElement).value, clear: false })}
                  />
                </label>
                {s.tls.clientKeyPem === "set" ? (
                  <label class="limit-field">
                    <span>Clear the client key</span>
                    <input type="checkbox" checked={clientKey.clear} disabled={disabled} onChange={(e) => setClientKey({ typed: "", clear: (e.target as HTMLInputElement).checked })} />
                  </label>
                ) : null}
                <div class="power-rows">
                  <label class="limit-field">
                    <span>Client certificate file on the hub</span>
                    <input value={s.tls.clientCertFile} disabled={disabled} onInput={(e) => tls({ clientCertFile: (e.target as HTMLInputElement).value })} />
                  </label>
                  <label class="limit-field">
                    <span>Its key file on the hub</span>
                    <input value={s.tls.clientKeyFile} disabled={disabled} onInput={(e) => tls({ clientKeyFile: (e.target as HTMLInputElement).value })} />
                  </label>
                </div>
              </>
            ) : (
              <label class="limit-field">
                <span>Send the password unencrypted to a broker not on the hub's machine</span>
                <input
                  type="checkbox"
                  checked={s.tls.allowPlaintextPassword}
                  disabled={disabled}
                  onChange={(e) => tls({ allowPlaintextPassword: (e.target as HTMLInputElement).checked })}
                />
              </label>
            )}
          </fieldset>
          <fieldset class="hub-ha" aria-label="Home Assistant">
            <legend>
              <label>
                <input type="checkbox" checked={s.haEnabled} disabled={disabled} onChange={(e) => patch({ haEnabled: (e.target as HTMLInputElement).checked })} /> Home
                Assistant
              </label>
            </legend>
            <label class="limit-field">
              <span>Acts as the API key</span>
              <select
                aria-label="Home Assistant key"
                value={s.haKey == null ? "" : String(s.haKey)}
                disabled={disabled || !props.keys}
                onChange={(e) => {
                  const v = (e.target as HTMLSelectElement).value;
                  patch({ haKey: v ? Number(v) : null });
                }}
              >
                <option value="">A new key (Control)</option>
                {keys.map((k) => (
                  <option key={k.cid} value={String(k.cid)}>
                    {k.name} ({roleWord(k.role)})
                  </option>
                ))}
                {s.haKey != null && !keys.some((k) => k.cid === s.haKey) ? <option value={String(s.haKey)}>Key {s.haKey}</option> : null}
              </select>
            </label>
          </fieldset>
          <div class="installer-buttons">
            <button type="submit" disabled={disabled}>
              Save MQTT Settings
            </button>
          </div>
          <Done text={done} />
        </form>
      ) : null}
    </section>
  );
}
