// WP P3WGR: the hub screen's groups (PLAN-P3 F3, F8's group half). Every group the hub keeps
// (`hubGroups`, kept current by `group.changed`), one picked at a time: its members joined
// to the inventory (online or offline), and for an Admin the editor (name, note, members
// picked from the inventory; `hubGroupSet`, `hubGroupDelete`); a Control or Viewer session
// sees the same group read-only. Below it the group's action bar (GroupActions.tsx), the
// bulk apply (BulkApply.tsx) and the fleet history (FleetHistory.tsx), each shown only when
// the hub has its cap. Shown only on a hub with `groups`. Everything goes through the core's
// typed calls; the hub judges every write by role, limits and lock.

import { useEffect, useState } from "preact/hooks";
import type { HubDeviceRecord, HubGroupRecord } from "../../core/protocol";
import type { ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { BulkApply } from "./BulkApply";
import type { HubView } from "./feed";
import { FleetHistory } from "./FleetHistory";
import { GroupActions } from "./GroupActions";
import {
  GROUP_MEMBERS_MAX,
  GROUP_NAME_MAX,
  GROUP_NOTE_MAX,
  GROUPS_MAX,
  GroupStore,
  draftOf,
  draftProblem,
  failureWords,
  memberRows,
  toggleMember,
  type GroupDraft,
  type GroupsView,
} from "./groups-model";

const NO_GROUPS: GroupsView = { groups: [], error: "", read: false };

/** The session's groups while the hub has `groups` (the health board's filter reads them too). */
export function useHubGroups(session: Session, on: boolean): { store: GroupStore | null; view: GroupsView } {
  const [store, setStore] = useState<GroupStore | null>(null);
  const [, bump] = useState(0);
  useEffect(() => {
    if (!on) {
      setStore(null);
      return;
    }
    const s = new GroupStore(session);
    const stop = s.subscribe(() => bump((n) => n + 1));
    s.start();
    setStore(s);
    return () => {
      stop();
      s.stop();
    };
  }, [session, on]);
  return { store, view: store ? store.view : NO_GROUPS };
}

/** A word for a role. */
function roleNeeds(role: string, readOnly: string): string {
  if (readOnly) return readOnly;
  return role === "admin" ? "" : "Groups are made and changed by an Admin of the hub.";
}

export function HubGroups(p: { session: Session; ask: ScreenProps["ask"]; view: HubView; groups: { store: GroupStore | null; view: GroupsView }; name: (serial: string) => string }) {
  const { session, view } = p;
  const store = p.groups.store;
  const list = p.groups.view.groups;
  const [picked, setPicked] = useState("");
  const [draft, setDraft] = useState<GroupDraft | null>(null);
  const [said, setSaid] = useState<{ text: string; bad: boolean; key: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [stamp, setStamp] = useState(0);
  const readOnly = session.readOnlyReason;
  const admin = view.role === "admin" && !readOnly;
  const why = roleNeeds(view.role, readOnly);
  const group = list.find((g) => g.id === picked) ?? list[0] ?? null;
  useEffect(() => {
    if (group && group.id !== picked) setPicked(group.id);
  }, [group?.id]);
  if (!store) return null;

  const save = async (d: GroupDraft) => {
    const problem = draftProblem(d);
    if (problem) {
      setSaid({ text: problem, bad: true, key: "" });
      return;
    }
    setBusy(true);
    try {
      const r = await session.core.call("hubGroupSet", { group: { ...(d.id ? { id: d.id } : {}), name: d.name.trim(), note: d.note, members: d.members } });
      store.took(r.group);
      setPicked(r.group.id);
      setDraft(null);
      setSaid({ text: r.created ? `Made the group ${r.group.name}.` : `Saved the group ${r.group.name}.`, bad: false, key: "" });
    } catch (e) {
      const f = failureWords(e);
      setSaid({ text: f.text, bad: true, key: f.key });
    } finally {
      setBusy(false);
    }
  };
  const remove = async (g: HubGroupRecord) => {
    const a = await p.ask({
      title: "Delete the group",
      body: <p>Delete the group {g.name}? Its DSPis stay on the hub; only the group goes.</p>,
      buttons: [
        { label: "Delete", value: "delete", danger: true },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (a !== "delete") return;
    setBusy(true);
    try {
      await session.core.call("hubGroupDelete", { group: g.id });
      store.dropped(g.id);
      setDraft(null);
      setSaid({ text: `Deleted the group ${g.name}.`, bad: false, key: "" });
    } catch (e) {
      const f = failureWords(e);
      setSaid({ text: f.text, bad: true, key: f.key });
    } finally {
      setBusy(false);
    }
  };

  const members = group ? memberRows(group, view.devices) : [];
  return (
    <section class="card hub-groups" aria-label="Groups">
      <div class="card-head">
        <h2>Groups</h2>
        {admin && !draft ? (
          <button type="button" disabled={busy || list.length >= GROUPS_MAX} title={list.length >= GROUPS_MAX ? `The hub keeps at most ${GROUPS_MAX} groups` : undefined} onClick={() => setDraft(draftOf(null))}>
            New Group…
          </button>
        ) : null}
      </div>
      {p.groups.view.error ? (
        <p class="error" role="alert">
          {p.groups.view.error}
        </p>
      ) : null}
      {said ? (
        <p class={said.bad ? "error" : "note"} role="status" aria-label="Group result" data-key={said.key || undefined}>
          {said.text}
        </p>
      ) : null}
      {list.length ? (
        <div class="hub-group-pick" role="group" aria-label="Pick a group">
          {list.map((g) => (
            <button key={g.id} type="button" aria-pressed={group?.id === g.id} onClick={() => setPicked(g.id)}>
              {g.name} <span class="hub-group-count">({g.members.length})</span>
            </button>
          ))}
        </div>
      ) : p.groups.view.read ? (
        <p class="note">The hub has no groups yet.{admin ? "" : ` ${why}`}</p>
      ) : null}
      {draft ? (
        <GroupEditor draft={draft} devices={view.devices} busy={busy} onChange={setDraft} onSave={(d) => void save(d)} onCancel={() => setDraft(null)} />
      ) : group ? (
        <div class="hub-group" aria-label={`Group ${group.name}`}>
          <div class="hub-group-head">
            <h3>{group.name}</h3>
            {admin ? (
              <div class="hub-edit-actions">
                <button type="button" disabled={busy} onClick={() => setDraft(draftOf(group))}>
                  Edit…
                </button>
                <button type="button" class="danger-text" disabled={busy} onClick={() => void remove(group)}>
                  Delete…
                </button>
              </div>
            ) : (
              <span class="note hub-group-ro">{why}</span>
            )}
          </div>
          {group.note ? <p class="hub-note">{group.note}</p> : null}
          {members.length ? (
            <ul class="hub-group-members" aria-label="Members">
              {members.map((m) => (
                <li key={m.serial} class={m.online ? "online" : "offline"} data-serial={m.serial}>
                  <span class="hub-name">{m.name}</span>
                  <span class="hub-serial">{m.serial}</span>
                  <span class={m.online ? "hub-state ok" : "hub-state offline"}>{m.online ? "Online" : "Offline"}</span>
                </li>
              ))}
            </ul>
          ) : (
            <p class="note">No DSPis in this group.</p>
          )}
        </div>
      ) : null}
      {group && !draft ? (
        <>
          {view.caps?.groupParams ? <GroupActions session={session} view={view} group={group} name={p.name} onDone={() => setStamp((n) => n + 1)} /> : null}
          {view.caps?.groupApply ? <BulkApply session={session} view={view} group={group} name={p.name} onDone={() => setStamp((n) => n + 1)} /> : null}
          {view.caps?.events ? <FleetHistory session={session} group={group} name={p.name} stamp={stamp} /> : null}
        </>
      ) : null}
    </section>
  );
}

function GroupEditor(p: { draft: GroupDraft; devices: HubDeviceRecord[]; busy: boolean; onChange: (d: GroupDraft) => void; onSave: (d: GroupDraft) => void; onCancel: () => void }) {
  const d = p.draft;
  const problem = draftProblem(d);
  // The inventory's devices, and a member the inventory no longer has (kept until saved).
  const known = new Set(p.devices.map((x) => x.serial));
  const extra = d.members.filter((s) => !known.has(s));
  return (
    <form
      class="hub-edit hub-group-edit"
      aria-label={d.id ? `Edit the group ${d.name}` : "New group"}
      onSubmit={(e) => {
        e.preventDefault();
        p.onSave(d);
      }}
    >
      <h3>{d.id ? "Edit the group" : "New group"}</h3>
      <label>
        Name
        <input value={d.name} maxLength={GROUP_NAME_MAX} required onInput={(e) => p.onChange({ ...d, name: (e.target as HTMLInputElement).value })} />
      </label>
      <label>
        Note
        <textarea value={d.note} maxLength={GROUP_NOTE_MAX} rows={2} onInput={(e) => p.onChange({ ...d, note: (e.target as HTMLTextAreaElement).value })} />
      </label>
      <fieldset class="hub-group-choose">
        <legend>
          DSPis in the group ({d.members.length} of at most {GROUP_MEMBERS_MAX})
        </legend>
        {p.devices.length || extra.length ? (
          <ul>
            {p.devices.map((x) => (
              <li key={x.serial}>
                <label>
                  <input type="checkbox" checked={d.members.includes(x.serial)} onChange={() => p.onChange(toggleMember(d, x.serial))} />
                  <span class="hub-name">{x.name || x.serial}</span> <span class="hub-serial">{x.serial}</span>
                  {x.online ? null : <span class="hub-state offline"> Offline</span>}
                </label>
              </li>
            ))}
            {extra.map((s) => (
              <li key={s}>
                <label>
                  <input type="checkbox" checked onChange={() => p.onChange(toggleMember(d, s))} />
                  <span class="hub-serial">{s}</span> <span class="hub-state offline"> Not on the hub</span>
                </label>
              </li>
            ))}
          </ul>
        ) : (
          <p class="note">The hub has seen no DSPi yet.</p>
        )}
      </fieldset>
      {problem && d.name.trim() ? <p class="error">{problem}</p> : null}
      <div class="hub-edit-actions">
        <button type="submit" disabled={p.busy || !!problem}>
          Save
        </button>
        <button type="button" onClick={p.onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
