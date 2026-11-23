// WP FEATUI: FEAT1 on the Installation tab (as Qt's InstallationPanel): the audio link sources,
// a unit's slots (locked while a zone's source owns them), what a zone plays and the units it
// routes, scenes with their actions (Switch zone source among them) and the programme sources.
// Channels and inputs are numbered from 0, as the core's sentences say them.

import { useState } from "preact/hooks";
import type { InstallationForm } from "./model";
import {
  PROGRAMME_KINDS,
  actionText,
  cleanText,
  numbersText,
  parseNumbers,
  parseSlots,
  programmeFeeds,
  programmeLimit,
  sceneTargets,
  slotsOwner,
  slotsText,
  zoneUnits,
  type Rerouted,
} from "./model";
import type { SystemModel, Unit, Zone } from "../model";
import type { InstallationProgrammeKind } from "./protocol";

/** A unit's slots: typed as "8 9 - -", or "Set by <zone>, which plays <source>". */
export function SlotsRow(props: { form: InstallationForm; model: SystemModel; u: Unit; edit: boolean }) {
  const { u, model, form } = props;
  const owner = slotsOwner(model, u);
  const label = cleanText(u.name || u.id);
  const shown = slotsText(u.slots);
  return (
    <div class="sys-row" role="group" aria-label={`Audio link slots of ${label}`}>
      <label class="sys-inline">
        Audio link channel on each input
        <input
          aria-label={`Channels on the inputs of ${label}`}
          value={shown}
          placeholder="Its own"
          disabled={!props.edit || !!owner}
          onChange={(e) => {
            const box = e.currentTarget as HTMLInputElement;
            const slots = parseSlots(box.value);
            if (slots === null) {
              box.value = shown;
              form.refuse(`${label}: a slot plays an audio link channel from 0 to 63, or nothing (-).`);
              return;
            }
            if (JSON.stringify(slots.length ? slots : null) === JSON.stringify(u.slots ?? null)) return;
            void form.edit({ op: "setUnitSlots", id: u.id, slots: slots.length ? slots : null }).then((ok) => {
              if (!ok) box.value = shown;
            });
          }}
        />
      </label>
      {owner ? (
        <span class="sys-muted">
          Set by {owner.zone}, which plays {owner.source}
        </span>
      ) : props.edit && u.slots ? (
        <button type="button" title="The unit plays its own slots again, as set on it" onClick={() => void form.edit({ op: "setUnitSlots", id: u.id, slots: null })}>
          Its own
        </button>
      ) : null}
    </div>
  );
}

/** What a zone plays, and the units its source routes. */
export function ZoneSource(props: { form: InstallationForm; model: SystemModel; z: Zone; edit: boolean }) {
  const { z, model, form } = props;
  const label = cleanText(z.name || z.id);
  const units = zoneUnits(model, z.id);
  return (
    <div class="sys-row" role="group" aria-label={`What ${label} plays`}>
      <label class="sys-inline">
        Plays
        <select
          aria-label={`Source of ${label}`}
          value={z.source ?? ""}
          disabled={!props.edit}
          onChange={(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            if (v !== (z.source ?? "")) void form.edit({ op: "setZoneSource", id: z.id, source: v || null });
          }}
        >
          <option value="">No source</option>
          {model.sources.map((s) => (
            <option key={s.id} value={s.id}>
              {cleanText(s.name || s.id)}
            </option>
          ))}
        </select>
      </label>
      <span class="sys-muted">
        {!units.length
          ? "No audio link unit is in its rooms."
          : z.source
            ? `Routes ${units.join(", ")} to its channels.`
            : `A source would route ${units.join(", ")}; their own slots stay until then.`}
      </span>
    </div>
  );
}

/** Before Save: the units whose own slots a zone's source replaces. */
export function ReroutedNotice(props: { rows: Rerouted[] }) {
  if (!props.rows.length) return null;
  return (
    <p class="sys-warn" role="alert" aria-label="Slots replaced">
      {props.rows
        .map((r) => (r.handMade ? `${r.name}'s own slots will be replaced: ${r.zone} plays ${r.source}.` : `${r.name} will play ${r.source}'s channels, as ${r.zone} does.`))
        .join(" ")}{" "}
      The units play them after the next alignment.
    </p>
  );
}

export function Sources(props: { form: InstallationForm; model: SystemModel; edit: boolean }) {
  const { model, form, edit } = props;
  const [name, setName] = useState("");
  const [sender, setSender] = useState("");
  const [channels, setChannels] = useState("");
  const senderName = (id: string | null | undefined) => cleanText(model.senders.find((s) => s.id === id)?.name || id || "");
  const add = async () => {
    const list = parseNumbers(channels, 64);
    if (!list) {
      form.refuse('Give the new source\'s channels on the audio link, 0 to 63: "8 9" or "8-11".');
      return;
    }
    if (await form.edit({ op: "addSource", name: name.trim(), sender: sender || null, channels: list })) {
      setName("");
      setChannels("");
    }
  };
  return (
    <section aria-label="Sources">
      <h4>Sources: channels on the audio link</h4>
      <p class="sys-note">A source names channels of a sender (0 to 63); a zone that plays it routes its units' inputs to them.</p>
      <div class="sys-inst-list">
        {model.sources.map((s) => {
          const label = cleanText(s.name || s.id);
          return (
            <div key={s.id} class="sys-room sys-inst-card" aria-label={`Source ${label}`}>
              <div class="sys-row">
                <input
                  class="sys-inst-name"
                  aria-label="Name"
                  value={label}
                  disabled={!edit}
                  maxLength={64}
                  onChange={(e) => {
                    const n = (e.currentTarget as HTMLInputElement).value.trim();
                    if (n && n !== s.name) void form.edit({ op: "renameElement", id: s.id, name: n });
                  }}
                />
                {s.sender ? <span class="sys-muted">from {senderName(s.sender)}</span> : null}
                {edit ? (
                  <button type="button" aria-label={`Remove ${label}`} onClick={() => void form.edit({ op: "removeElement", id: s.id })}>
                    Remove
                  </button>
                ) : null}
              </div>
              <label class="sys-inline">
                Channels on the audio link
                <input
                  aria-label={`Channels of ${label}`}
                  value={numbersText(s.channels)}
                  disabled={!edit}
                  onChange={(e) => {
                    const box = e.currentTarget as HTMLInputElement;
                    const list = parseNumbers(box.value, 64);
                    if (!list) {
                      box.value = numbersText(s.channels);
                      form.refuse(`${label}: give its channels on the audio link, 0 to 63.`);
                    } else if (numbersText(list) !== numbersText(s.channels)) void form.edit({ op: "setSource", id: s.id, sender: s.sender ?? null, channels: list });
                  }}
                />
              </label>
            </div>
          );
        })}
      </div>
      {edit ? (
        <form
          class="sys-row"
          aria-label="New source"
          onSubmit={(e) => {
            e.preventDefault();
            void add();
          }}
        >
          <input aria-label="New source's name" placeholder="New source" value={name} maxLength={64} onInput={(e) => setName((e.currentTarget as HTMLInputElement).value)} />
          {model.senders.length ? (
            <select aria-label="New source's sender" value={sender} onChange={(e) => setSender((e.currentTarget as HTMLSelectElement).value)}>
              <option value="">Any sender</option>
              {model.senders.map((s) => (
                <option key={s.id} value={s.id}>
                  {cleanText(s.name || s.id)}
                </option>
              ))}
            </select>
          ) : null}
          <input aria-label="New source's channels" placeholder="Channels: 8 9" value={channels} onInput={(e) => setChannels((e.currentTarget as HTMLInputElement).value)} />
          <button type="submit" disabled={!name.trim()}>
            Add source
          </button>
        </form>
      ) : null}
    </section>
  );
}

const OPS: { id: string; label: string }[] = [
  { id: "source", label: "Switch zone source" },
  { id: "mute", label: "Mute" },
  { id: "unmute", label: "Unmute" },
  { id: "master_volume", label: "Master volume" },
];

export function Scenes(props: { form: InstallationForm; model: SystemModel; edit: boolean }) {
  const { model, form, edit } = props;
  const [name, setName] = useState("");
  const add = async () => {
    if (name.trim() && (await form.edit({ op: "addScene", name: name.trim(), actions: [] }))) setName("");
  };
  return (
    <section aria-label="Scenes">
      <h4>Scenes</h4>
      <div class="sys-inst-list">
        {model.scenes.map((sc) => (
          <SceneCard key={sc.id} form={form} model={model} id={sc.id} edit={edit} />
        ))}
      </div>
      {edit ? (
        <form
          class="sys-row"
          aria-label="New scene"
          onSubmit={(e) => {
            e.preventDefault();
            void add();
          }}
        >
          <input aria-label="New scene's name" placeholder="New scene" value={name} maxLength={64} onInput={(e) => setName((e.currentTarget as HTMLInputElement).value)} />
          <button type="submit" disabled={!name.trim()}>
            Add scene
          </button>
        </form>
      ) : null}
    </section>
  );
}

function SceneCard(props: { form: InstallationForm; model: SystemModel; id: string; edit: boolean }) {
  const { model, form, edit } = props;
  const sc = model.scenes.find((x) => x.id === props.id)!;
  const label = cleanText(sc.name || sc.id);
  const [op, setOp] = useState("source");
  const [target, setTarget] = useState("");
  const [source, setSource] = useState("");
  const [level, setLevel] = useState(-12);
  const targets = sceneTargets(model).filter((t) => op !== "source" || t.id.startsWith("zone:"));
  const at = targets.some((t) => t.id === target) ? target : (targets[0]?.id ?? "");
  const src = model.sources.some((s) => s.id === source) ? source : (model.sources[0]?.id ?? "");
  const keep = sc.systemMute === undefined ? {} : { systemMute: sc.systemMute };
  const addAction = () => {
    if (!at) return;
    const action =
      op === "source"
        ? { target: at, op, source: src }
        : op === "master_volume"
          ? { target: at, op, value: Math.round(level * 100) }
          : { target: at, op };
    void form.edit({ op: "setScene", id: sc.id, actions: [...sc.actions, action], ...keep });
  };
  return (
    <div class="sys-room sys-inst-card" aria-label={`Scene ${label}`}>
      <div class="sys-row">
        <input
          class="sys-inst-name"
          aria-label="Name"
          value={label}
          disabled={!edit}
          maxLength={64}
          onChange={(e) => {
            const n = (e.currentTarget as HTMLInputElement).value.trim();
            if (n && n !== sc.name) void form.edit({ op: "renameElement", id: sc.id, name: n });
          }}
        />
        {sc.systemMute ? <span class="sys-muted">Mutes the whole system</span> : null}
        {edit ? (
          <button type="button" aria-label={`Remove ${label}`} onClick={() => void form.edit({ op: "removeElement", id: sc.id })}>
            Remove
          </button>
        ) : null}
      </div>
      {sc.actions.length ? (
        <ul class="sys-results" aria-label={`Actions of ${label}`}>
          {sc.actions.map((a, i) => {
            const words = actionText(model, a);
            return (
              <li key={`${i}-${words}`}>
                {words}
                {edit ? (
                  <button
                    type="button"
                    class="sys-inline"
                    aria-label={`Remove the action ${words}`}
                    onClick={() => void form.edit({ op: "setScene", id: sc.id, actions: sc.actions.filter((_, j) => j !== i), ...keep })}
                  >
                    ×
                  </button>
                ) : null}
              </li>
            );
          })}
        </ul>
      ) : (
        <p class="sys-note">No actions yet.</p>
      )}
      {edit ? (
        <div class="sys-row" role="group" aria-label={`New action of ${label}`}>
          <select aria-label="What it does" value={op} onChange={(e) => setOp((e.currentTarget as HTMLSelectElement).value)}>
            {OPS.map((o) => (
              <option key={o.id} value={o.id}>
                {o.label}
              </option>
            ))}
          </select>
          <select aria-label="Where" value={at} onChange={(e) => setTarget((e.currentTarget as HTMLSelectElement).value)}>
            {targets.map((t) => (
              <option key={t.id} value={t.id}>
                {t.name}
              </option>
            ))}
          </select>
          {op === "source" ? (
            <select aria-label="Switch to source" value={src} onChange={(e) => setSource((e.currentTarget as HTMLSelectElement).value)}>
              {model.sources.map((s) => (
                <option key={s.id} value={s.id}>
                  {cleanText(s.name || s.id)}
                </option>
              ))}
            </select>
          ) : null}
          {op === "master_volume" ? (
            <label class="sys-inline">
              <input type="number" aria-label="Master volume in dB" min={-60} max={0} step={1} value={level} onChange={(e) => setLevel(Number((e.currentTarget as HTMLInputElement).value))} />
              dB
            </label>
          ) : null}
          <button type="button" disabled={!at || (op === "source" && !src)} onClick={addAction}>
            Add action
          </button>
        </div>
      ) : null}
    </div>
  );
}

export function Programmes(props: { form: InstallationForm; model: SystemModel; edit: boolean }) {
  const { model, form, edit } = props;
  const list = model.programmeSources ?? [];
  const [name, setName] = useState("");
  const [kind, setKind] = useState<InstallationProgrammeKind>("computer_usb");
  const [feeds, setFeeds] = useState("");
  const [inputs, setInputs] = useState("");
  const choices = programmeFeeds(model, kind);
  const feed = choices.some((c) => c.id === feeds) ? feeds : (choices[0]?.id ?? "");
  const add = async () => {
    const limit = programmeLimit(kind);
    const ins = parseNumbers(inputs, limit);
    if (!ins) {
      form.refuse(`Give the inputs the new programme source plays on, 0 to ${limit - 1}: "0 1".`);
      return;
    }
    if (await form.edit({ op: "addProgrammeSource", name: name.trim(), kind, feeds: feed, inputs: ins })) {
      setName("");
      setInputs("");
    }
  };
  const nameOf = (id: string) => cleanText([...model.units, ...model.senders].find((e) => e.id === id)?.name || id);
  return (
    <section aria-label="Programme sources">
      <h4>Programme sources: where the music comes from</h4>
      <p class="sys-note">A DSPi has no player: it plays what its inputs receive. Say which computer or box feeds which unit or sender, so people and AI agents know.</p>
      <div class="sys-inst-list">
        {list.map((p) => {
          const label = cleanText(p.name || p.id);
          const limit = programmeLimit(p.kind);
          return (
            <div key={p.id} class="sys-room sys-inst-card" aria-label={`Programme source ${label}`}>
              <div class="sys-row">
                <input
                  class="sys-inst-name"
                  aria-label="Name"
                  value={label}
                  disabled={!edit}
                  maxLength={64}
                  onChange={(e) => {
                    const n = (e.currentTarget as HTMLInputElement).value.trim();
                    if (n && n !== p.name) void form.edit({ op: "renameElement", id: p.id, name: n });
                  }}
                />
                <span class="sys-muted">
                  {PROGRAMME_KINDS.find((k) => k.id === p.kind)?.label ?? p.kind} into {nameOf(p.feeds)}
                </span>
                {edit ? (
                  <button type="button" aria-label={`Remove ${label}`} onClick={() => void form.edit({ op: "removeElement", id: p.id })}>
                    Remove
                  </button>
                ) : null}
              </div>
              <label class="sys-inline">
                {p.kind === "alink_sender" ? "Sender channels" : "Inputs"}
                <input
                  aria-label={`Inputs of ${label}`}
                  value={numbersText(p.inputs)}
                  disabled={!edit}
                  onChange={(e) => {
                    const box = e.currentTarget as HTMLInputElement;
                    const ins = parseNumbers(box.value, limit);
                    if (!ins) {
                      box.value = numbersText(p.inputs);
                      form.refuse(`${label}: give the inputs it plays on, 0 to ${limit - 1}.`);
                    } else if (numbersText(ins) !== numbersText(p.inputs))
                      void form.edit({ op: "setProgrammeSource", id: p.id, kind: p.kind, feeds: p.feeds, inputs: ins });
                  }}
                />
              </label>
            </div>
          );
        })}
      </div>
      {edit ? (
        <form
          class="sys-row"
          aria-label="New programme source"
          onSubmit={(e) => {
            e.preventDefault();
            void add();
          }}
        >
          <input aria-label="New programme source's name" placeholder="New programme source" value={name} maxLength={64} onInput={(e) => setName((e.currentTarget as HTMLInputElement).value)} />
          <select aria-label="How it comes in" value={kind} onChange={(e) => setKind((e.currentTarget as HTMLSelectElement).value as InstallationProgrammeKind)}>
            {PROGRAMME_KINDS.map((k) => (
              <option key={k.id} value={k.id}>
                {k.label}
              </option>
            ))}
          </select>
          <select aria-label="What it feeds" value={feed} onChange={(e) => setFeeds((e.currentTarget as HTMLSelectElement).value)}>
            {choices.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </select>
          <input aria-label="New programme source's inputs" placeholder="Inputs: 0 1" value={inputs} onInput={(e) => setInputs((e.currentTarget as HTMLInputElement).value)} />
          <button type="submit" disabled={!name.trim() || !feed}>
            Add programme source
          </button>
        </form>
      ) : null}
    </section>
  );
}
