// WP FEATUI: a room's or zone's volume (FEAT2's systemVolumePlan/Set, as Qt's SystemVolumeCard):
// choose a room or zone and a level or a step, see per unit what it is and would be (louder ones
// marked), then Set, which asks first when a unit gets louder. A refused unit says why.

import { useState } from "preact/hooks";
import type { Ask } from "../../ui/screens";
import type { Session } from "../../ui/session";
import type { SystemModel } from "./model";
import { cleanText } from "./installation/model";
import type { SystemVolumePlan, SystemVolumeUnit } from "./volume-protocol";

function db(v: number | null | undefined): string {
  return v === null || v === undefined ? "–" : `${v.toFixed(1)} dB`;
}

export function rowText(u: SystemVolumeUnit): string {
  const to = u.afterDb ?? u.targetDb;
  const tail =
    u.state === "refused" ? ` (not set: ${cleanText(u.message)})` : u.state === "done" ? " (set)" : u.state === "unchanged" ? " (already there)" : u.louder ? " (louder)" : "";
  return `${cleanText(u.name)}: ${db(u.beforeDb)} → ${db(to)}${tail}`;
}

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function ZoneVolume(props: { model: SystemModel; session: Session; ask: Ask }) {
  const targets = [
    ...props.model.rooms.map((r) => ({ id: `room:${r.id}`, name: `Room ${cleanText(r.name || r.id)}` })),
    ...props.model.zones.map((z) => ({ id: `zone:${z.id}`, name: `Zone ${cleanText(z.name || z.id)}` })),
  ];
  const [target, setTarget] = useState(targets[0]?.id ?? "");
  const [step, setStep] = useState(false);
  const [level, setLevel] = useState(-20);
  const [master, setMaster] = useState(false);
  const [plan, setPlan] = useState<SystemVolumePlan | null>(null);
  const [set, setSet] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  if (!targets.length) return null;
  const chosen = targets.some((t) => t.id === target) ? target : targets[0]!.id;
  const drop = () => {
    setPlan(null);
    setSet(false);
    setMessage("");
  };
  const show = async () => {
    const [kind, id] = [chosen.slice(0, chosen.indexOf(":")), chosen.slice(chosen.indexOf(":") + 1)];
    setBusy(true);
    try {
      const p = await props.session.core.call("systemVolumePlan", {
        [kind === "zone" ? "zone" : "room"]: id,
        volume: master ? "master" : "user",
        [step ? "stepDb" : "levelDb"]: level,
      });
      setPlan(p);
      setSet(false);
      setMessage("");
    } catch (e) {
      setPlan(null);
      setMessage(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const ready = plan?.units.filter((u) => u.state === "ready") ?? [];
  const louder = ready.filter((u) => u.louder);
  const apply = async () => {
    if (!plan) return;
    if (louder.length) {
      const go = await props.ask({
        title: "Make it louder?",
        body: <p>{louder.length === 1 ? `1 unit of ${cleanText(plan.name)} gets louder.` : `${louder.length} units of ${cleanText(plan.name)} get louder.`}</p>,
        buttons: [
          { label: "Set", value: "set" },
          { label: "Cancel", value: "cancel" },
        ],
      });
      if (go !== "set") return;
    }
    setBusy(true);
    try {
      const done = await props.session.core.call("systemVolumeSet", { plan });
      setPlan(done);
      setSet(true);
      const n = done.units.filter((u) => u.state === "done").length;
      const refused = done.units.filter((u) => u.state === "refused").length;
      setMessage(`${cleanText(done.name)}: ${n} set, ${refused} refused.`);
    } catch (e) {
      setMessage(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section class="sys-room sys-volume" aria-label="Room and zone volume">
      <h3>Room and zone volume</h3>
      <div class="sys-row">
        <select
          aria-label="Room or zone"
          value={chosen}
          onChange={(e) => {
            setTarget((e.currentTarget as HTMLSelectElement).value);
            drop();
          }}
        >
          {targets.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
        <select
          aria-label="Level or step"
          value={step ? "step" : "level"}
          onChange={(e) => {
            const s = (e.currentTarget as HTMLSelectElement).value === "step";
            setStep(s);
            setLevel(s ? -3 : -20);
          }}
        >
          <option value="level">Set to</option>
          <option value="step">Step by</option>
        </select>
        <label class="sys-inline">
          <input
            type="number"
            aria-label="Volume in dB"
            min={step ? -20 : -80}
            max={step ? 20 : 0}
            step={1}
            value={level}
            onChange={(e) => setLevel(Number((e.currentTarget as HTMLInputElement).value))}
          />
          dB
        </label>
        <label class="sys-inline">
          <input type="checkbox" aria-label="Set the master volume instead of the user volume" checked={master} onChange={(e) => setMaster((e.currentTarget as HTMLInputElement).checked)} />
          Master volume
        </label>
        <button type="button" disabled={busy} onClick={() => void show()} title="Read each unit's volume and show what would change; nothing is written yet">
          Show changes
        </button>
      </div>
      {plan ? (
        <ul class="sys-results" aria-label="Volume per unit">
          {plan.units.map((u) => (
            <li key={u.unitId} class={u.state === "refused" ? "sys-issue" : u.louder && u.state === "ready" ? "sys-warn" : ""}>
              {rowText(u)}
            </li>
          ))}
        </ul>
      ) : null}
      {plan && !set ? (
        <div class="sys-row">
          <button type="button" disabled={busy || !ready.length} onClick={() => void apply()}>
            Set
          </button>
          <button type="button" onClick={drop}>
            Cancel
          </button>
        </div>
      ) : null}
      {message ? (
        <p class="sys-note" role="status">
          {message}
        </p>
      ) : null}
    </section>
  );
}
