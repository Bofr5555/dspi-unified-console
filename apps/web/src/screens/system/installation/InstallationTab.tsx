// WP INSTW: the System screen's Installation tab (MPLAN 1.4), a thin form over the core's
// Installation setup (model.ts, WP INST0): the units the hub sees that the installation does
// not hold, with Join; each unit's name, room, role and what feeds it; rooms and zones; the
// hub's validator's verdict at the field it is about; what saving changes, then Save (Admin).

import { useEffect, useMemo, useState } from "preact/hooks";
import type { Ask } from "../../../ui/screens";
import type { Session } from "../../../ui/session";
import type { SystemFeed } from "../feed";
import type { Room, SystemModel, Unit, Zone } from "../model";
import { writeNote, type Role } from "../model";
import {
  InstallationForm,
  ROLES,
  cleanText,
  generalIssues,
  issuesAt,
  joinEdit,
  needsUpstream,
  roleOf,
  upstreamChoices,
  type FormState,
  type InstallationCall,
  type IssuePlace,
} from "./model";
import type { InstallationIssue, InstallationRole, InstallationUnseen } from "./protocol";

function useForm(form: InstallationForm): FormState {
  const [, bump] = useState(0);
  useEffect(() => form.watch(() => bump((n) => n + 1)), [form]);
  return form.state;
}

export function InstallationTab(props: { feed: SystemFeed; session: Session; ask: Ask }) {
  const call: InstallationCall = (method, params) => props.session.core.call(method, params as never) as never;
  const form = useMemo(() => new InstallationForm(call), [props.session]);
  useEffect(() => void form.load(), [form]);
  const st = useForm(form);
  const draft = st.draft;
  if (!draft || !st.view) {
    return (
      <section class="sys" aria-label="Installation">
        {st.error ? (
          <p class="error" role="alert">
            {st.error}
          </p>
        ) : (
          <p class="sys-note">Reading the installation…</p>
        )}
      </section>
    );
  }
  const edit = form.editable(props.feed.readOnly);
  const model = draft.model;
  const save = async () => {
    const go = await props.ask({
      title: "Save the installation?",
      body: (
        <>
          <p>The hub keeps the installation as saved here; every client sees it. These changes are sent:</p>
          <ul>
            {st.changes.map((c) => (
              <li key={c}>{c}</li>
            ))}
          </ul>
        </>
      ),
      buttons: [
        { label: "Save", value: "save" },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (go !== "save") return;
    if ((await form.save()) === "ok") await props.feed.reread();
  };
  const general = generalIssues(st.issues, model);
  return (
    <section class="sys sys-inst" aria-label="Installation">
      <h3>Installation</h3>
      {edit ? null : (
        <p class="sys-note">
          {props.feed.readOnly ? "This session is read only" : writeNote(st.view.role as Role, "model")}: the installation is shown, not changed.
        </p>
      )}
      {st.error ? (
        <p class="error" role="alert">
          {st.error}
        </p>
      ) : null}
      {st.lost ? (
        <div class="sys-save sys-stale" role="alert" aria-label="Installation changed elsewhere">
          <span>
            The installation was changed elsewhere and has been read again, so your {st.lost.length === 1 ? "change was" : `${st.lost.length} changes were`} not
            saved. Make them again on the installation as it is now, or leave them.
          </span>
          <button type="button" onClick={() => void form.remake()}>
            Make them again
          </button>
          <button type="button" onClick={() => form.dismissLost()}>
            Leave them
          </button>
        </div>
      ) : null}
      {st.saved ? (
        <div role="status" aria-label="Installation saved">
          <p class="sys-note">Saved: the hub keeps the installation at revision {st.saved.revision}.</p>
          {st.saved.warnings.length ? (
            <ul class="sys-warnings" aria-label="Installation warnings">
              {st.saved.warnings.map((w) => (
                <li key={w} class="sys-result sys-warn">
                  {w}
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}
      <Unseen form={form} st={st} edit={edit} />
      <h4>Units</h4>
      {model.units.length ? (
        <div class="sys-inst-list">
          {model.units.map((u) => (
            <UnitCard key={u.id} form={form} model={model} u={u} issues={st.issues} edit={edit} />
          ))}
        </div>
      ) : (
        <p class="sys-note">No units yet: join the units the hub sees.</p>
      )}
      <Feeds form={form} model={model} issues={st.issues} edit={edit} />
      <Rooms form={form} model={model} issues={st.issues} edit={edit} />
      <Zones form={form} model={model} issues={st.issues} edit={edit} />
      {form.dirty ? (
        <div class="sys-save" role="group" aria-label="Unsaved installation">
          <ul class="sys-results" aria-label="Changes">
            {st.changes.map((c) => (
              <li key={c}>{c}</li>
            ))}
          </ul>
          {general.length ? (
            <ul class="sys-results" aria-label="Problems">
              {general.map((i) => (
                <li key={i.message} class="sys-issue">
                  {i.message}
                </li>
              ))}
            </ul>
          ) : null}
          {st.issues.length ? <p class="sys-note">The hub would refuse this installation: {st.issues.length === 1 ? "the problem is" : "the problems are"} shown where it is.</p> : null}
          <button type="button" disabled={!form.savable} onClick={() => void save()}>
            {st.saving ? "Saving…" : "Save"}
          </button>
          <button type="button" disabled={st.saving} onClick={() => void form.revert()}>
            Revert
          </button>
        </div>
      ) : null}
    </section>
  );
}

function Issues(props: { issues: InstallationIssue[]; id: string; field: IssuePlace["field"] }) {
  const at = issuesAt(props.issues, props.id, props.field);
  return (
    <>
      {at.map((i) => (
        <p key={i.message} class="sys-issue" role="status">
          {i.message}
        </p>
      ))}
    </>
  );
}

/** A name field: the rename is made when the field is left or Enter is pressed. */
function NameField(props: { label: string; name: string; edit: boolean; onName: (n: string) => void }) {
  return (
    <input
      class="sys-inst-name"
      aria-label={props.label}
      value={cleanText(props.name)}
      disabled={!props.edit}
      maxLength={64}
      onChange={(e) => {
        const n = (e.currentTarget as HTMLInputElement).value.trim();
        if (n && n !== props.name) props.onName(n);
        else (e.currentTarget as HTMLInputElement).value = cleanText(props.name);
      }}
    />
  );
}

function RoomSelect(props: { model: SystemModel; value: string; label: string; edit: boolean; onRoom: (r: string) => void }) {
  const known = props.model.rooms.some((r) => r.id === props.value);
  return (
    <select aria-label={props.label} value={props.value} disabled={!props.edit} onChange={(e) => props.onRoom((e.currentTarget as HTMLSelectElement).value)}>
      <option value="">No room</option>
      {props.value && !known ? <option value={props.value}>{cleanText(props.value)} (missing)</option> : null}
      {props.model.rooms.map((r) => (
        <option key={r.id} value={r.id}>
          {cleanText(r.name || r.id)}
        </option>
      ))}
    </select>
  );
}

function UpstreamSelect(props: { model: SystemModel; role: InstallationRole; self?: string; value: string; label: string; edit: boolean; onUp: (u: string) => void }) {
  const choices = upstreamChoices(props.model, props.role, props.self);
  const known = choices.some((c) => c.id === props.value);
  return (
    <select aria-label={props.label} value={props.value} disabled={!props.edit} onChange={(e) => props.onUp((e.currentTarget as HTMLSelectElement).value)}>
      {props.value && !known ? <option value={props.value}>{cleanText(props.value)} (missing)</option> : null}
      {!props.value ? <option value="">Choose…</option> : null}
      {choices.map((c) => (
        <option key={c.id} value={c.id}>
          {cleanText(c.name)}
        </option>
      ))}
    </select>
  );
}

function RoleSelect(props: { value: InstallationRole; label: string; edit: boolean; onRole: (r: InstallationRole) => void }) {
  return (
    <select aria-label={props.label} value={props.value} disabled={!props.edit} onChange={(e) => props.onRole((e.currentTarget as HTMLSelectElement).value as InstallationRole)}>
      {ROLES.map((r) => (
        <option key={r.id} value={r.id}>
          {r.label}
        </option>
      ))}
    </select>
  );
}

/** What joining makes of a unit the hub sees: a unit in a role, or the alink's sender or a fan-out. */
type JoinAs = InstallationRole | "sender" | "fan_out";

function Unseen(props: { form: InstallationForm; st: FormState; edit: boolean }) {
  const { view, draft } = props.st;
  if (!view || !draft) return null;
  const held = new Set<string>([
    ...draft.model.units.map((u) => u.serial),
    ...draft.model.senders.map((s) => s.serial ?? ""),
    ...draft.model.fanOuts.map((f) => f.serial ?? ""),
  ]);
  const left = view.unseen.filter((u) => !held.has(u.serial));
  return (
    <section aria-label="Units not in the installation">
      <h4>Seen by the hub, not in the installation</h4>
      {!view.inventory ? (
        <p class="sys-note">This hub keeps no inventory, so the units it sees are not listed here.</p>
      ) : left.length ? (
        <div class="sys-inst-list">
          {left.map((u) => (
            <JoinCard key={u.serial} form={props.form} model={draft.model} u={u} edit={props.edit} />
          ))}
        </div>
      ) : (
        <p class="sys-note">Every unit the hub sees is in the installation.</p>
      )}
    </section>
  );
}

function JoinCard(props: { form: InstallationForm; model: SystemModel; u: InstallationUnseen; edit: boolean }) {
  const { u, model } = props;
  const [open, setOpen] = useState(false);
  const [name, setName] = useState(cleanText(u.name));
  const [room, setRoom] = useState("");
  const [as, setAs] = useState<JoinAs>("receiver");
  const [up, setUp] = useState("");
  const upRole: InstallationRole = as === "sender" ? "usb_direct" : as === "fan_out" ? "receiver" : as;
  const choices = upstreamChoices(model, upRole);
  const upstream = choices.some((c) => c.id === up) ? up : (choices[0]?.id ?? "");
  const join = async () => {
    const shown = name.trim();
    const ok =
      as === "sender"
        ? await props.form.edit({ op: "addSender", name: shown || u.serial, serial: u.serial })
        : as === "fan_out"
          ? await props.form.edit({ op: "addFanOut", name: shown || u.serial, serial: u.serial, upstream })
          : await props.form.edit(joinEdit(u.serial, { name: shown, room, role: as, upstream }));
    if (ok) setOpen(false);
  };
  const label = cleanText(u.name) || u.serial;
  return (
    <div class="sys-room sys-inst-card" aria-label={`Seen ${label}`}>
      <div class="sys-row">
        <span class={`dot ${u.online ? "dot-open" : "dot-failed"}`} aria-hidden="true" />
        <strong>{label}</strong>
        <span class="sys-muted">{u.serial}</span>
        <span class="sys-state">{u.online ? "Online" : "Offline"}</span>
        {u.fw ? <span class="sys-muted">firmware {cleanText(u.fw)}</span> : null}
        {!open && props.edit ? (
          <button type="button" onClick={() => setOpen(true)}>
            Join
          </button>
        ) : null}
      </div>
      {open ? (
        <div role="group" aria-label={`Join ${label}`}>
          <div class="sys-row">
            <label class="sys-inline">
              Name
              <input aria-label="Name" value={name} maxLength={64} onInput={(e) => setName((e.currentTarget as HTMLInputElement).value)} />
            </label>
            <label class="sys-inline">
              As
              <select aria-label="As" value={as} onChange={(e) => setAs((e.currentTarget as HTMLSelectElement).value as JoinAs)}>
                {ROLES.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.label}
                  </option>
                ))}
                <option value="sender">Sender</option>
                <option value="fan_out">Fan-out</option>
              </select>
            </label>
            {as !== "sender" && as !== "fan_out" ? (
              <label class="sys-inline">
                Room
                <RoomSelect model={model} value={room} label="Room" edit onRoom={setRoom} />
              </label>
            ) : null}
            {as === "fan_out" || needsUpstream(upRole) ? (
              <label class="sys-inline">
                Fed by
                <UpstreamSelect model={model} role={upRole} value={upstream} label="Fed by" edit onUp={setUp} />
              </label>
            ) : null}
          </div>
          <div class="sys-row">
            <button type="button" onClick={() => void join()}>
              Add to the installation
            </button>
            <button type="button" onClick={() => setOpen(false)}>
              Cancel
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function UnitCard(props: { form: InstallationForm; model: SystemModel; u: Unit; issues: InstallationIssue[]; edit: boolean }) {
  const { u, model, form, edit } = props;
  const role = roleOf(u);
  const label = cleanText(u.name) || u.id;
  return (
    <div class="sys-room sys-inst-card" aria-label={`Unit ${label}`}>
      <div class="sys-row">
        <NameField label="Name" name={u.name || u.id} edit={edit} onName={(name) => void form.edit({ op: "renameElement", id: u.id, name })} />
        <span class="sys-muted">{u.serial}</span>
        {edit ? (
          <button type="button" aria-label={`Remove ${label}`} onClick={() => void form.edit({ op: "removeElement", id: u.id })}>
            Remove
          </button>
        ) : null}
      </div>
      <div class="sys-row">
        <label class="sys-inline">
          Room
          <RoomSelect model={model} value={u.room ?? ""} label="Room" edit={edit} onRoom={(r) => void form.edit({ op: "setUnitRoom", id: u.id, room: r || null })} />
        </label>
        <label class="sys-inline">
          Role
          <RoleSelect value={role} label="Role" edit={edit} onRole={(r) => void form.edit({ op: "setUnitRole", id: u.id, role: r })} />
        </label>
        {needsUpstream(role) ? (
          <label class="sys-inline">
            Fed by
            <UpstreamSelect
              model={model}
              role={role}
              self={u.id}
              value={u.upstream}
              label="Fed by"
              edit={edit}
              onUp={(up) => void (up ? form.edit({ op: "setUnitUpstream", id: u.id, upstream: up }) : undefined)}
            />
          </label>
        ) : null}
      </div>
      <Issues issues={props.issues} id={u.id} field="room" />
      <Issues issues={props.issues} id={u.id} field="upstream" />
      <Issues issues={props.issues} id={u.id} field="" />
    </div>
  );
}

/** The alink's senders and fan-outs: what feeds the units. */
function Feeds(props: { form: InstallationForm; model: SystemModel; issues: InstallationIssue[]; edit: boolean }) {
  const { model, form, edit } = props;
  const rows = [
    ...model.senders.map((s) => ({ id: s.id, name: s.name, serial: s.serial ?? "", kind: "Sender", upstream: "" })),
    ...model.fanOuts.map((f) => ({ id: f.id, name: f.name, serial: f.serial ?? "", kind: "Fan-out", upstream: f.upstream })),
  ];
  if (!rows.length) return null;
  const nameOf = (id: string) => cleanText(model.senders.find((s) => s.id === id)?.name ?? model.fanOuts.find((f) => f.id === id)?.name ?? model.units.find((u) => u.id === id)?.name ?? id);
  return (
    <section aria-label="Senders and fan-outs">
      <h4>Senders and fan-outs</h4>
      <div class="sys-inst-list">
        {rows.map((r) => {
          const label = cleanText(r.name) || r.id;
          return (
            <div key={r.id} class="sys-room sys-inst-card" aria-label={`${r.kind} ${label}`}>
              <div class="sys-row">
                <NameField label="Name" name={r.name || r.id} edit={edit} onName={(name) => void form.edit({ op: "renameElement", id: r.id, name })} />
                <span class="sys-state">{r.kind}</span>
                {r.serial ? <span class="sys-muted">{r.serial}</span> : null}
                {r.upstream ? <span class="sys-muted">fed by {nameOf(r.upstream)}</span> : null}
                {edit ? (
                  <button type="button" aria-label={`Remove ${label}`} onClick={() => void form.edit({ op: "removeElement", id: r.id })}>
                    Remove
                  </button>
                ) : null}
              </div>
              <Issues issues={props.issues} id={r.id} field="upstream" />
              <Issues issues={props.issues} id={r.id} field="" />
            </div>
          );
        })}
      </div>
    </section>
  );
}

function Rooms(props: { form: InstallationForm; model: SystemModel; issues: InstallationIssue[]; edit: boolean }) {
  const { model, form, edit } = props;
  const [name, setName] = useState("");
  const add = async () => {
    if (name.trim() && (await form.edit({ op: "addRoom", name: name.trim() }))) setName("");
  };
  return (
    <section aria-label="Rooms">
      <h4>Rooms</h4>
      <div class="sys-inst-list">
        {model.rooms.map((r: Room) => {
          const label = cleanText(r.name) || r.id;
          const units = model.units.filter((u) => u.room === r.id).length;
          return (
            <div key={r.id} class="sys-room sys-inst-card" aria-label={`Room ${label}`}>
              <div class="sys-row">
                <NameField label="Name" name={r.name || r.id} edit={edit} onName={(n) => void form.edit({ op: "renameElement", id: r.id, name: n })} />
                <span class="sys-muted">{units === 1 ? "1 unit" : `${units} units`}</span>
                {edit ? (
                  <button type="button" aria-label={`Remove ${label}`} onClick={() => void form.edit({ op: "removeElement", id: r.id })}>
                    Remove
                  </button>
                ) : null}
              </div>
              <Issues issues={props.issues} id={r.id} field="" />
            </div>
          );
        })}
      </div>
      {edit ? (
        <form
          class="sys-row"
          aria-label="New room"
          onSubmit={(e) => {
            e.preventDefault();
            void add();
          }}
        >
          <input aria-label="New room's name" placeholder="New room" value={name} maxLength={64} onInput={(e) => setName((e.currentTarget as HTMLInputElement).value)} />
          <button type="submit" disabled={!name.trim()}>
            Add room
          </button>
        </form>
      ) : null}
    </section>
  );
}

function RoomChecks(props: { model: SystemModel; rooms: string[]; edit: boolean; onRooms: (r: string[]) => void }) {
  return (
    <div class="sys-row" role="group" aria-label="Rooms">
      {props.model.rooms.map((r) => (
        <label key={r.id} class="sys-inline">
          <input
            type="checkbox"
            checked={props.rooms.includes(r.id)}
            disabled={!props.edit}
            onChange={(e) => {
              const on = (e.currentTarget as HTMLInputElement).checked;
              props.onRooms(on ? [...props.rooms, r.id] : props.rooms.filter((x) => x !== r.id));
            }}
          />
          {cleanText(r.name || r.id)}
        </label>
      ))}
    </div>
  );
}

function Zones(props: { form: InstallationForm; model: SystemModel; issues: InstallationIssue[]; edit: boolean }) {
  const { model, form, edit } = props;
  const [name, setName] = useState("");
  const [rooms, setRooms] = useState<string[]>([]);
  const add = async () => {
    const kept = rooms.filter((r) => model.rooms.some((x) => x.id === r));
    if (name.trim() && (await form.edit({ op: "addZone", name: name.trim(), rooms: kept }))) {
      setName("");
      setRooms([]);
    }
  };
  return (
    <section aria-label="Zones">
      <h4>Zones</h4>
      {model.zones.length ? (
        <div class="sys-inst-list">
          {model.zones.map((z: Zone) => {
            const label = cleanText(z.name) || z.id;
            return (
              <div key={z.id} class="sys-room sys-inst-card" aria-label={`Zone ${label}`}>
                <div class="sys-row">
                  <NameField label="Name" name={z.name || z.id} edit={edit} onName={(n) => void form.edit({ op: "renameElement", id: z.id, name: n })} />
                  {edit ? (
                    <button type="button" aria-label={`Remove ${label}`} onClick={() => void form.edit({ op: "removeElement", id: z.id })}>
                      Remove
                    </button>
                  ) : null}
                </div>
                <RoomChecks model={model} rooms={z.rooms} edit={edit} onRooms={(r) => void form.edit({ op: "setZoneRooms", id: z.id, rooms: r })} />
                <Issues issues={props.issues} id={z.id} field="rooms" />
                <Issues issues={props.issues} id={z.id} field="" />
              </div>
            );
          })}
        </div>
      ) : (
        <p class="sys-note">No zones.</p>
      )}
      {edit ? (
        <form
          class="sys-room sys-inst-card"
          aria-label="New zone"
          onSubmit={(e) => {
            e.preventDefault();
            void add();
          }}
        >
          <div class="sys-row">
            <input aria-label="New zone's name" placeholder="New zone" value={name} maxLength={64} onInput={(e) => setName((e.currentTarget as HTMLInputElement).value)} />
            <button type="submit" disabled={!name.trim()}>
              Add zone
            </button>
          </div>
          {model.rooms.length ? <RoomChecks model={model} rooms={rooms} edit onRooms={setRooms} /> : null}
        </form>
      ) : null}
    </section>
  );
}

