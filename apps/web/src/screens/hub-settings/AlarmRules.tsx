// WP P2WB2: the hub's alarm rules (`alarm.rules.get`, every role; `alarm.rules.set`, Admin): per code
// on or off, the thresholds the code has, and "mute on clip storm" (off by default). The ranges are
// the hub's: a value out of range is refused in its words and nothing changes.

import { useEffect, useState } from "preact/hooks";
import type { HubAlarmRuleRecord } from "../../core/hub-settings-protocol";
import { noticeRow } from "../../ui/texts";
import { failure } from "../installer/HubAdmin";
import { alarmLabel, numberOf, ruleFields, withRule } from "./model";
import { Done, Problem, type SectionProps } from "./parts";

// -- WP P3WRO -- (review p3c N3) the rules the hub keeps on whatever is sent: `alarm.rules.set`
// cannot turn `rollout_failed` off (hub-api.md section 21), so its switch shows it on and fixed.
import { ALWAYS_ON_ALARMS as ALWAYS_ON } from "../hub/rollout-model";

export function AlarmRules(props: SectionProps) {
  const { session, edit } = props;
  const [rules, setRules] = useState<HubAlarmRuleRecord[] | null>(null);
  const [problem, setProblem] = useState("");
  const [done, setDone] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let live = true;
    void session.core.call("hubAlarmRulesGet", undefined).then(
      (r) => live && setRules(r),
      (e: unknown) => live && setProblem(failure(e)),
    );
    return () => {
      live = false;
    };
  }, [session]);
  const change = (code: string, patch: Partial<HubAlarmRuleRecord>) => {
    setRules((r) => (r ? withRule(r, code, patch) : r));
    setDone("");
  };
  const save = async () => {
    if (!rules) return;
    setBusy(true);
    try {
      setRules(await session.core.call("hubAlarmRulesSet", { rules }));
      setProblem("");
      setDone("The hub's alarm rules are saved.");
    } catch (e) {
      setProblem(failure(e));
    } finally {
      setBusy(false);
    }
  };
  const disabled = !edit || busy;
  return (
    <section class="card hub-section" aria-label="Alarm rules">
      <h3>Alarm rules</h3>
      <p class="note">When the hub raises an alarm about a DSPi. A rule turned off clears its alarms.</p>
      <Problem text={problem} />
      {rules ? (
        <div class="hub-rules">
          {rules.map((r) => {
            const label = alarmLabel(r.code, noticeRow(`alarm.${r.code}`)?.title);
            const always = ALWAYS_ON.has(r.code); // -- WP P3WRO --
            return (
              <fieldset key={r.code} class="hub-rule" aria-label={label}>
                <legend>
                  <label title={always ? "Always on: the hub does not let this alarm be turned off" : undefined}>
                    <input type="checkbox" checked={r.on || always} disabled={disabled || always} onChange={(e) => change(r.code, { on: (e.target as HTMLInputElement).checked })} />{" "}
                    {label}
                    {always ? <span class="hub-rule-always"> (always on)</span> : null}
                  </label>
                </legend>
                <div class="power-rows">
                  {ruleFields(r).map((f) => (
                    <label class="limit-field" key={f.field}>
                      <span>{f.label}</span>
                      <input
                        type="number"
                        min={f.min}
                        max={f.max}
                        step={f.step}
                        value={String(r[f.field] ?? "")}
                        disabled={disabled}
                        onChange={(e) => {
                          const v = numberOf((e.target as HTMLInputElement).value);
                          if (v != null) change(r.code, { [f.field]: v });
                        }}
                      />
                    </label>
                  ))}
                  {r.muteOnStorm != null ? (
                    <label class="limit-field">
                      <span>Mute the DSPi on a clip storm</span>
                      <input type="checkbox" checked={r.muteOnStorm} disabled={disabled} onChange={(e) => change(r.code, { muteOnStorm: (e.target as HTMLInputElement).checked })} />
                    </label>
                  ) : null}
                </div>
              </fieldset>
            );
          })}
        </div>
      ) : null}
      <div class="installer-buttons">
        <button type="button" disabled={disabled || !rules} onClick={() => void save()}>
          Save Alarm Rules
        </button>
      </div>
      <Done text={done} />
    </section>
  );
}
