// WP INSTW: the System screen's Installation tab (MPLAN 1.4), a thin form over the core's
// Installation setup (model.ts, WP INST0): the units the hub sees that the installation does
// not hold, with Join; each unit's name, room, role and what feeds it; rooms and zones; the
// hub's validator's verdict at the field it is about; what saving changes, then Save (Admin).
//
// -- WP WEBM3 -- (review RB C-2, as Qt's INSTQ panel) also an S/PDIF source's role and its
// time-base slots, groups of units, and the source a new zone plays.

import { useEffect, useMemo, useState } from "preact/hooks";
import type { Ask } from "../../../ui/screens";
import type { Session } from "../../../ui/session";
import type { SystemFeed } from "../feed";
import type { Group, Room, SystemModel, Unit, Zone } from "../model";
import { writeNote, type Role } from "../model";
import {
  InstallationForm,
  NEW_SOURCE_MASK,
  ROLES,
  SPDIF_SLOTS,
  roleEdits,
  slotEdit,
  withMember,
  type UnitRole,
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
import type { InstallationIssue, InstallationUnseen } from "./protocol";
import { onAlink, rerouted } from "./model"; // -- WP FEATUI --
import { Programmes, ReroutedNotice, Scenes, SlotsRow, Sources, ZoneSource } from "./Routing"; // -- WP FEATUI --

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
  const [alignPending, setAlignPending] = useState(0); // -- WP FEATUI --
  const [aligned, setAligned] = useState(false);
  const [, bumpFeed] = useState(0);
  useEffect(() => props.feed.watch(() => bumpFeed((n) => n + 1)), [props.feed]);
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
  const routed = rerouted(draft); // -- WP FEATUI --
  const align = props.feed.state.align; // -- WP FEATUI --
  const alignNow = async () => {
    const go = await props.ask({
      title: "Align audio link delays?",
      body: <p>Every unit restarts its outputs in step with a short silence (2 to 6 ms).</p>,
      buttons: [
        { label: "Align", value: "align" },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (go !== "align") return;
    setAlignPending(0);
    setAligned(true);
    await props.feed.align(true);
  };
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
    const moving = routed.length; // -- WP FEATUI -- what the units take at the next alignment
    if ((await form.save()) === "ok") {
      setAlignPending(moving);
      setAligned(false);
      await props.feed.reread();
    }
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
      {/* -- WP FEATUI -- saved slots go live at the alignment */}
      {alignPending > 0 ? (
        <div class="sys-save" role="status" aria-label="Alignment needed">
          <span>{alignPending === 1 ? "1 unit plays the new channels after Align." : `${alignPending} units play the new channels after Align.`}</span>
          <button type="button" disabled={!!align?.running} onClick={() => void alignNow()}>
            Align
          </button>
        </div>
      ) : null}
      {aligned && align && !align.running && (align.message || align.error) ? (
        <p class={align.error ? "error" : "sys-note"} role="status" aria-label="Alignment">
          {align.error || align.message}
        </p>
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
      <Sources form={form} model={model} edit={edit} /* -- WP FEATUI -- */ />
      <Rooms form={form} model={model} issues={st.issues} edit={edit} />
      <Zones form={form} model={model} issues={st.issues} edit={edit} />
      <Groups form={form} model={model} issues={st.issues} edit={edit} /* -- WP WEBM3 -- */ />
      <Scenes form={form} model={model} edit={edit} /* -- WP FEATUI -- */ />
      <Programmes form={form} model={model} edit={edit} /* -- WP FEATUI -- */ />
      {form.dirty ? (
        <div class="sys-save" role="group" aria-label="Unsaved installation">
          <ReroutedNotice rows={routed} /* -- WP FEATUI -- */ />
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

function UpstreamSelect(props: { model: SystemModel; role: UnitRole; self?: string; value: string; label: string; edit: boolean; onUp: (u: string) => void }) {
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

function RoleSelect(props: { value: UnitRole; label: string; edit: boolean; onRole: (r: UnitRole) => void }) {
  return (
    <select aria-label={props.label} value={props.value} disabled={!props.edit} onChange={(e) => props.onRole((e.currentTarget as HTMLSelectElement).value as UnitRole)}>
      {ROLES.map((r) => (
        <option key={r.id} value={r.id}>
          {r.label}
        </option>
      ))}
    </select>
  );
}

/** What joining makes of a unit the hub sees: a unit in a role, or the alink's sender or a fan-out. */
type JoinAs = UnitRole | "sender" | "fan_out";

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
  const upRole: UnitRole = as === "sender" ? "usb_direct" : as === "fan_out" ? "receiver" : as;
  const choices = upstreamChoices(model, upRole);
  const upstream = choices.some((c) => c.id === up) ? up : (choices[0]?.id ?? "");
  const join = async () => {
    const shown = name.trim();
    const ok =
      as === "sender"
        ? await props.form.edit({ op: "addSender", name: shown || u.serial, serial: u.serial })
        : as === "fan_out"
          ? await props.form.edit({ op: "addFanOut", name: shown || u.serial, serial: u.serial, upstream })
          : as === "spdif_source"
            ? // -- WP WEBM3 -- joined as a follower, marked a source on the id the core made, in one step
              await props.form.editThen(joinEdit(u.serial, { name: shown, room, role: as, upstream }), (next) => {
                const id = next.model.units.find((x) => x.serial === u.serial)?.id;
                return id ? [{ op: "setSpdifSource", id, txMask: NEW_SOURCE_MASK }] : [];
              })
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
          <RoleSelect value={role} label="Role" edit={edit} onRole={(r) => void form.edit(...roleEdits(u, r)) /* -- WP WEBM3 -- an S/PDIF source's mark too */} />
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
      {u.spdifSource ? (
        // -- WP WEBM3 -- the S/PDIF output slots a source sends its time base on
        <div class="sys-row" role="group" aria-label={`Time base on outputs of ${label}`}>
          <span class="sys-muted">Time base on outputs</span>
          {SPDIF_SLOTS.map((x) => (
            <label key={x.slot} class="sys-inline">
              <input
                type="checkbox"
                aria-label={`${label} sends the time base on outputs ${x.outputs}`}
                checked={(((u.spdifTxMask ?? 0) >> x.slot) & 1) === 1}
                disabled={!edit}
                onChange={(e) => {
                  const box = e.currentTarget as HTMLInputElement;
                  const ed = slotEdit(u, x.slot, box.checked);
                  if (typeof ed === "string") {
                    box.checked = !box.checked;
                    form.refuse(ed);
                  } else void form.edit(ed);
                }}
              />
              {x.outputs}
            </label>
          ))}
        </div>
      ) : null}
      {onAlink(u) ? <SlotsRow form={form} model={model} u={u} edit={edit} /* -- WP FEATUI -- */ /> : null}
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
  const [source, setSource] = useState(""); // -- WP WEBM3 -- what the new zone plays
  const add = async () => {
    const kept = rooms.filter((r) => model.rooms.some((x) => x.id === r));
    const plays = model.sources.some((x) => x.id === source) ? source : null;
    if (name.trim() && (await form.edit({ op: "addZone", name: name.trim(), rooms: kept, source: plays }))) {
      setName("");
      setRooms([]);
      setSource("");
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
                <ZoneSource form={form} model={model} z={z} edit={edit} /* -- WP FEATUI -- what it plays, set here too */ />
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
          {model.sources.length ? (
            // -- WP WEBM3 -- the source it plays (set when it is made, as Qt's)
            <label class="sys-inline">
              Plays
              <select aria-label="New zone's source" value={source} onChange={(e) => setSource((e.currentTarget as HTMLSelectElement).value)}>
                <option value="">No source</option>
                {model.sources.map((x) => (
                  <option key={x.id} value={x.id}>
                    {cleanText(x.name || x.id)}
                  </option>
                ))}
              </select>
            </label>
          ) : null}
        </form>
      ) : null}
    </section>
  );
}


// -- WP WEBM3 -- (review RB C-2, as Qt's INSTQ) groups of units: a name, its units, made and removed
function UnitChecks(props: { model: SystemModel; units: string[]; edit: boolean; label: string; onUnits: (u: string[]) => void }) {
  return (
    <div class="sys-row" role="group" aria-label={props.label}>
      {props.model.units.map((u) => (
        <label key={u.id} class="sys-inline">
          <input
            type="checkbox"
            checked={props.units.includes(u.id)}
            disabled={!props.edit}
            onChange={(e) => props.onUnits(withMember(props.units, u.id, (e.currentTarget as HTMLInputElement).checked))}
          />
          {cleanText(u.name || u.id)}
        </label>
      ))}
    </div>
  );
}

function Groups(props: { form: InstallationForm; model: SystemModel; issues: InstallationIssue[]; edit: boolean }) {
  const { model, form, edit } = props;
  const [name, setName] = useState("");
  const [units, setUnits] = useState<string[]>([]);
  const add = async () => {
    const kept = units.filter((u) => model.units.some((x) => x.id === u));
    if (name.trim() && (await form.edit({ op: "addGroup", name: name.trim(), units: kept }))) {
      setName("");
      setUnits([]);
    }
  };
  return (
    <section aria-label="Groups">
      <h4>Groups</h4>
      {model.groups.length ? (
        <div class="sys-inst-list">
          {model.groups.map((g: Group) => {
            const label = cleanText(g.name) || g.id;
            return (
              <div key={g.id} class="sys-room sys-inst-card" aria-label={`Group ${label}`}>
                <div class="sys-row">
                  <NameField label="Name" name={g.name || g.id} edit={edit} onName={(n) => void form.edit({ op: "renameElement", id: g.id, name: n })} />
                  {edit ? (
                    <button type="button" aria-label={`Remove ${label}`} onClick={() => void form.edit({ op: "removeElement", id: g.id })}>
                      Remove
                    </button>
                  ) : null}
                </div>
                <UnitChecks model={model} units={g.units} edit={edit} label="Units" onUnits={(u) => void form.edit({ op: "setGroupUnits", id: g.id, units: u })} />
                <Issues issues={props.issues} id={g.id} field="" />
              </div>
            );
          })}
        </div>
      ) : (
        <p class="sys-note">No groups.</p>
      )}
      {edit ? (
        <form
          class="sys-room sys-inst-card"
          aria-label="New group"
          onSubmit={(e) => {
            e.preventDefault();
            void add();
          }}
        >
          <div class="sys-row">
            <input aria-label="New group's name" placeholder="New group" value={name} maxLength={64} onInput={(e) => setName((e.currentTarget as HTMLInputElement).value)} />
            <button type="submit" disabled={!name.trim()}>
              Add group
            </button>
          </div>
          {model.units.length ? <UnitChecks model={model} units={units} edit label="Units of the new group" onUnits={setUnits} /> : null}
        </form>
      ) : null}
    </section>
  );
}
