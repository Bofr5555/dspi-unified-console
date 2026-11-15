// WP P2WB2: the hub's schedule (`schedule.list`, every role; `schedule.set`, `schedule.delete`,
// `schedule.run_now`, Admin). A rule runs on days at a local time, or once, in the hub's time zone;
// its actions go to its targets as the principal "Schedule: <name>". The hub checks every field and
// answers each action's result; this page shows them.

import { useEffect, useState } from "preact/hooks";
import type { HubScheduleActionRecord, HubScheduleRuleRecord, HubScheduleRunRecord, HubSchedulesRecord } from "../../core/hub-settings-protocol";
import { failure } from "../installer/HubAdmin";
import {
  ACTION_TYPES,
  DAYS,
  PARAM_PATHS,
  actionText,
  localTime,
  newAction,
  newRule,
  numberOf,
  pathOf,
  resultText,
  ruleForSet,
  targetsOf,
  whenText,
  withPath,
} from "./model";
import { Done, Problem, type SectionProps } from "./parts";

export function Schedules(props: SectionProps) {
  const { session, edit } = props;
  const [list, setList] = useState<HubSchedulesRecord | null>(null);
  const [problem, setProblem] = useState("");
  const [editing, setEditing] = useState<HubScheduleRuleRecord | null>(null);
  const [run, setRun] = useState<HubScheduleRunRecord | null>(null);
  const [busy, setBusy] = useState(false);
  const read = async () => {
    try {
      setList(await session.core.call("hubSchedules", undefined));
      setProblem("");
    } catch (e) {
      setProblem(failure(e));
    }
  };
  useEffect(() => {
    void read();
  }, [session]);
  const act = async (f: () => Promise<void>) => {
    setBusy(true);
    try {
      await f();
      setProblem("");
    } catch (e) {
      setProblem(failure(e));
    } finally {
      setBusy(false);
    }
  };
  const save = (r: HubScheduleRuleRecord) =>
    act(async () => {
      await session.core.call("hubScheduleSet", { rule: ruleForSet(r) });
      setEditing(null);
      await read();
    });
  const runNow = (r: HubScheduleRuleRecord) =>
    act(async () => {
      setRun(await session.core.call("hubScheduleRunNow", { id: r.id }));
      await read();
    });
  const remove = async (r: HubScheduleRuleRecord) => {
    const a = await props.ask({
      title: "Delete Schedule",
      body: <p>Delete the schedule “{r.name}”?</p>,
      buttons: [
        { label: "Delete", value: "delete", danger: true },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (a === "delete")
      await act(async () => {
        await session.core.call("hubScheduleDelete", { id: r.id });
        await read();
      });
  };
  const disabled = !edit || busy;
  return (
    <section class="card hub-section" aria-label="Schedules">
      <h3>Schedules</h3>
      {list ? (
        <p class="note">
          Times are the hub's local time ({list.zone || "its time zone"}); it is {list.localTime.replace("T", " ") || "–"} there now.
        </p>
      ) : null}
      {list?.clockUnset ? (
        <p class="note warn" role="note">
          The hub's clock is not set: no schedule runs until it is.
        </p>
      ) : null}
      <Problem text={problem} />
      {list ? (
        list.rules.length ? (
          <ul class="snap-list" aria-label="Schedule rules">
            {list.rules.map((r) => (
              <li key={r.id} class="snap-row" aria-label={r.name}>
                <div class="snap-what">
                  <strong>{r.name}</strong>
                  {r.enabled ? null : <span class="badge">Off</span>}
                  <span class="note">
                    {" "}
                    {whenText(r)} · {r.actions.map(actionText).join("; ")}
                    {r.nextRun ? ` · next ${localTime(r.nextRun)}` : ""}
                    {r.last ? ` · last ${localTime(r.last.at)} ${r.last.skipped ? "(skipped)" : r.last.ok ? "(done)" : "(failed)"}` : ""}
                  </span>
                </div>
                <div class="snap-actions">
                  <button type="button" disabled={disabled} onClick={() => setEditing(r)}>
                    Edit
                  </button>
                  <button type="button" disabled={disabled} onClick={() => void runNow(r)}>
                    Run Now
                  </button>
                  <button type="button" class="danger-text" disabled={disabled} aria-label={`Delete ${r.name}`} onClick={() => void remove(r)}>
                    Delete
                  </button>
                </div>
              </li>
            ))}
          </ul>
        ) : (
          <p class="note">No schedules yet.</p>
        )
      ) : null}
      {run ? (
        <div class="card hub-run" role="status" aria-label="Schedule run">
          <strong>
            “{run.name}” ran {localTime(run.at)}
            {run.by ? `, by ${run.by}` : ""}
          </strong>
          <ul class="change-list">
            {run.results.map((x, i) => (
              <li key={i} class={x.ok ? "" : "error"}>
                {resultText(x)}
              </li>
            ))}
          </ul>
          <button type="button" onClick={() => setRun(null)}>
            Done
          </button>
        </div>
      ) : null}
      {editing ? (
        <RuleEditor rule={editing} now={list?.localTime ?? ""} busy={busy} onSave={(r) => void save(r)} onCancel={() => setEditing(null)} />
      ) : (
        <div class="installer-buttons">
          <button type="button" disabled={disabled} onClick={() => setEditing(newRule(props.serial))}>
            New Schedule…
          </button>
        </div>
      )}
    </section>
  );
}

function RuleEditor(props: { rule: HubScheduleRuleRecord; now: string; busy: boolean; onSave: (r: HubScheduleRuleRecord) => void; onCancel: () => void }) {
  const [r, setR] = useState(props.rule);
  const [targets, setTargets] = useState(props.rule.targets.join(", "));
  const weekly = !r.once;
  const patch = (p: Partial<HubScheduleRuleRecord>) => setR((x) => ({ ...x, ...p }));
  const setAction = (i: number, a: HubScheduleActionRecord) => patch({ actions: r.actions.map((x, j) => (j === i ? a : x)) });
  return (
    <form
      class="card hub-editor"
      aria-label="Schedule rule"
      onSubmit={(e) => {
        e.preventDefault();
        props.onSave({ ...r, targets: targetsOf(targets) });
      }}
    >
      <h4>{r.id ? `Edit “${props.rule.name}”` : "New schedule"}</h4>
      <div class="power-rows">
        <label class="limit-field">
          <span>Name</span>
          <input value={r.name} maxLength={48} onInput={(e) => patch({ name: (e.target as HTMLInputElement).value })} />
        </label>
        <label class="limit-field">
          <span>On</span>
          <input type="checkbox" checked={r.enabled} onChange={(e) => patch({ enabled: (e.target as HTMLInputElement).checked })} />
        </label>
        <label class="limit-field">
          <span>Runs</span>
          <select
            aria-label="Runs"
            value={weekly ? "weekly" : "once"}
            onChange={(e) => {
              const v = (e.target as HTMLSelectElement).value;
              patch(v === "once" ? { once: r.once || props.now, days: [] } : { once: "", days: r.days.length ? r.days : ["mon"], time: r.time || "18:00" });
            }}
          >
            <option value="weekly">On days of the week</option>
            <option value="once">Once</option>
          </select>
        </label>
        {weekly ? (
          <label class="limit-field">
            <span>Time</span>
            <input type="time" value={r.time} onInput={(e) => patch({ time: (e.target as HTMLInputElement).value })} />
          </label>
        ) : (
          <label class="limit-field">
            <span>Date and time</span>
            <input type="datetime-local" value={r.once} onInput={(e) => patch({ once: (e.target as HTMLInputElement).value })} />
          </label>
        )}
      </div>
      {weekly ? (
        <fieldset class="hub-days" aria-label="Days">
          {DAYS.map((d) => (
            <label key={d.id}>
              <input
                type="checkbox"
                checked={r.days.includes(d.id)}
                onChange={(e) => {
                  const on = (e.target as HTMLInputElement).checked;
                  patch({ days: DAYS.map((x) => x.id).filter((x) => (x === d.id ? on : r.days.includes(x))) });
                }}
              />{" "}
              {d.label}
            </label>
          ))}
        </fieldset>
      ) : null}
      <fieldset class="hub-actions" aria-label="Actions">
        <legend>Actions</legend>
        {r.actions.map((a, i) => (
          <ActionRow key={i} n={i} a={a} onChange={(x) => setAction(i, x)} onRemove={r.actions.length > 1 ? () => patch({ actions: r.actions.filter((_, j) => j !== i) }) : null} />
        ))}
        {r.actions.length < 16 ? (
          <button type="button" onClick={() => patch({ actions: [...r.actions, newAction("param.set")] })}>
            Add Action
          </button>
        ) : null}
      </fieldset>
      <label class="limit-field hub-wide">
        <span>DSPis (serials)</span>
        <input aria-label="Targets" value={targets} onInput={(e) => setTargets((e.target as HTMLInputElement).value)} />
      </label>
      <div class="installer-buttons">
        <button type="submit" disabled={props.busy}>
          Save Schedule
        </button>
        <button type="button" onClick={props.onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}

function ActionRow(props: { n: number; a: HubScheduleActionRecord; onChange: (a: HubScheduleActionRecord) => void; onRemove: (() => void) | null }) {
  const { a, onChange } = props;
  const label = `Action ${props.n + 1}`;
  const p = pathOf(a.path);
  return (
    <div class="power-rows hub-action" role="group" aria-label={label}>
      <label class="limit-field">
        <span>{label}</span>
        <select aria-label={`${label} type`} value={a.t} onChange={(e) => onChange(newAction((e.target as HTMLSelectElement).value))}>
          {ACTION_TYPES.map((x) => (
            <option key={x.t} value={x.t}>
              {x.label}
            </option>
          ))}
        </select>
      </label>
      {a.t === "param.set" ? (
        <>
          <label class="limit-field">
            <span>What</span>
            <select aria-label={`${label} value`} value={a.path} onChange={(e) => onChange(withPath(a, (e.target as HTMLSelectElement).value))}>
              {PARAM_PATHS.map((x) => (
                <option key={x.path} value={x.path}>
                  {x.label}
                </option>
              ))}
            </select>
          </label>
          {p.idx ? (
            <label class="limit-field">
              <span>Output</span>
              <input type="number" min={1} max={32} value={String((a.idx[0] ?? 0) + 1)} onChange={(e) => onChange({ ...a, idx: [Math.max(0, (numberOf((e.target as HTMLInputElement).value) ?? 1) - 1)] })} />
            </label>
          ) : null}
          {p.flag ? (
            <label class="limit-field">
              <span>On</span>
              <input type="checkbox" checked={!!a.flag} onChange={(e) => onChange({ ...a, flag: (e.target as HTMLInputElement).checked })} />
            </label>
          ) : (
            <label class="limit-field">
              <span>dB</span>
              <input
                type="number"
                step={0.5}
                aria-label={`${label} dB`}
                value={a.value == null ? "" : String(a.value)}
                onChange={(e) => onChange({ ...a, value: numberOf((e.target as HTMLInputElement).value) })}
              />
            </label>
          )}
        </>
      ) : null}
      {a.t === "preset.load" ? (
        <label class="limit-field">
          <span>Slot</span>
          <input type="number" min={1} max={10} value={String((a.slot ?? 0) + 1)} onChange={(e) => onChange({ ...a, slot: Math.max(0, (numberOf((e.target as HTMLInputElement).value) ?? 1) - 1) })} />
        </label>
      ) : null}
      {a.t === "snapshot.recall" ? (
        <label class="limit-field">
          <span>Snapshot name</span>
          <input value={a.name} onInput={(e) => onChange({ ...a, name: (e.target as HTMLInputElement).value, snapshot: "" })} />
        </label>
      ) : null}
      {a.t === "system.scene.apply" ? (
        <label class="limit-field">
          <span>Scene</span>
          <input value={a.scene} onInput={(e) => onChange({ ...a, scene: (e.target as HTMLInputElement).value })} />
        </label>
      ) : null}
      {props.onRemove ? (
        <button type="button" class="danger-text" aria-label={`Remove ${label}`} onClick={props.onRemove}>
          Remove
        </button>
      ) : null}
    </div>
  );
}
