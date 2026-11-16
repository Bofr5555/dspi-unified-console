// WP P2WB1: the hub screen of a session on a hub that has the hub's own
// families (PRO phase 2): every DSPi the hub has seen, online or not, with its
// health; Identify on every online DSPi and Open for the others; an Admin
// sees the hub's statistics; the alarm board with Acknowledge. Everything
// comes from the core's typed calls through `HubFeed`; on a hub without these
// families (an upstream hub) the screen is not offered.

import type { FunctionComponent } from "preact";
import { useEffect, useState } from "preact/hooks";
import type { HubAlarmRecord, HubStatsRecord } from "../../core/protocol";
import type { ScreenDef, ScreenProps } from "../../ui/screens";
import { HubFeed, type HubView, type Said } from "./feed";
import { alarmSummary, alarmTitle, deviceRows, durationText, rowActions, shortTime, tagsFrom, type HubRow } from "./model";
import { useSessionState } from "../../ui/components";
import { HealthBoard } from "./HealthBoard"; // -- WP P3WHB --
import { Rollout } from "./Rollout"; // -- WP P3WRO --
import { rolloutShown } from "./rollout-model"; // -- WP P3WRO --

/** The screen's entry (screens.tsx shows it when `hubScreenShown`). */
export const HUB_SCREEN_ID = "hub";

/** Follow a feed's view. */
export function useHubView(feed: HubFeed | undefined): HubView | null {
  const [, bump] = useState(0);
  const rendered = feed?.view;
  useEffect(() => {
    if (!feed) return;
    const stop = feed.subscribe(() => bump((n) => n + 1));
    if (feed.view !== rendered) bump((n) => n + 1);
    return stop;
  }, [feed]);
  return feed ? feed.view : null;
}

export const HubScreen: FunctionComponent<ScreenProps> = (props) => {
  const feed = props.hub;
  const view = useHubView(feed);
  const hubName = useSessionState(props.session).hubName; // -- WP P2WB3 --
  if (!feed || !view) return null;
  return (
    <div class="hub-screen">
      {/* -- SEC hook (SECWEB, PLAN-SEC section 6) -- the "unencrypted" banner and the hub's fingerprint go here */}
      <HubDevices feed={feed} view={view} props={props} />
      {/* -- WP P3WHB -- */ view.caps?.health ? <HealthBoard feed={feed} view={view} readOnly={props.session.readOnlyReason} /> : null}
      {/* -- WP P3WRO -- the firmware rollout: an Admin's, on a hub with the `rollout` cap */ rolloutShown(view.caps, view.role) ? (
        <Rollout session={props.session} hub={feed} view={view} ask={props.ask} readOnly={props.session.readOnlyReason} />
      ) : null}
      {view.caps?.alarms ? <AlarmBoard feed={feed} view={view} readOnly={props.session.readOnlyReason} /> : null}
      {/* -- WP P2WB3 -- */}
      {view.caps?.hubStats && view.role === "admin" && !props.session.readOnlyReason ? <HubName feed={feed} name={hubName} /> : null}
      {view.stats ? <HubStats stats={view.stats} name={(s) => feed.deviceName(s)} /> : null}
    </div>
  );
};

export const HUB_SCREEN: ScreenDef = { id: HUB_SCREEN_ID, label: "Hub", View: HubScreen };

function HubDevices(p: { feed: HubFeed; view: HubView; props: ScreenProps }) {
  const { feed, view, props } = p;
  const session = props.session;
  const rows = deviceRows(view.devices, view.health, feed.openSerial());
  const [said, setSaid] = useState<Said | null>(null);
  const [busy, setBusy] = useState(false);
  // -- WP P2WB3 -- the device whose details an Admin edits, and the form's text
  const [editing, setEditing] = useState<{ serial: string; location: string; note: string; tags: string } | null>(null);
  const run = async (f: () => Promise<Said>) => {
    setBusy(true);
    setSaid(await f());
    setBusy(false);
  };
  const readOnly = session.readOnlyReason;
  const opts = { role: view.role, identify: !!view.caps?.identify, inventory: !!view.caps?.inventory, readOnly, canOpen: !!session.openDevice };
  const forget = async (r: HubRow) => {
    const a = await props.ask({
      title: "Forget the DSPi",
      body: (
        <>
          <p>
            Forget {r.name} ({r.serial}) on the hub? Its record and its history on the hub go. It is listed again the next time it is attached.
          </p>
          <p>Its snapshots on the hub stay unless you delete them too.</p>
        </>
      ),
      buttons: [
        { label: "Forget", value: "forget", danger: true },
        { label: "Forget and Delete Its Snapshots", value: "with-data", danger: true },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (a !== "forget" && a !== "with-data") return;
    if (editing?.serial === r.serial) setEditing(null);
    await run(() => feed.forget(r.serial, a === "with-data"));
  };
  const edited = editing ? rows.find((r) => r.serial === editing.serial) : undefined;
  return (
    <section class="card hub-devices" aria-label="Devices on the hub">
      <div class="card-head">
        <h2>Devices on the hub</h2>
        {view.role ? <span class="hub-role-tag">{roleWord(view.role)}</span> : null}
      </div>
      {view.error ? (
        <p class="error" role="alert">
          {view.error}
        </p>
      ) : null}
      {/* -- WP P2WB3 -- every action's line (Identify, Edit, Forget), not only Identify's */}
      {said ? (
        <p class={said.bad ? "error" : "note"} role="status" aria-label="Action result" data-key={said.notice?.key}>
          {said.text}
          {said.notice?.reason ? ` ${said.notice.reason}` : ""}
        </p>
      ) : null}
      {rows.length ? (
        <div class="hub-table-wrap">
          <table class="hub-table">
            <thead>
              <tr>
                <th scope="col">Name</th>
                <th scope="col">Location</th>
                <th scope="col">State</th>
                <th scope="col">Firmware</th>
                <th scope="col">CPU</th>
                <th scope="col">Temperature</th>
                <th scope="col">Last restart</th>
                <th scope="col">
                  <span class="visually-hidden">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => {
                const may = rowActions(r, opts);
                return (
                  <tr key={r.serial} class={r.online ? undefined : "offline"} aria-label={r.name}>
                    <td>
                      <span class="hub-name">{r.name}</span>
                      <span class="hub-serial">{r.serial}</span>
                      {r.tags.length ? <span class="hub-tags">{r.tags.join(", ")}</span> : null}
                      {r.note ? <span class="hub-note">{r.note}</span> : null}
                    </td>
                    <td>{r.location || "–"}</td>
                    <td>
                      <StateWord row={r} />
                    </td>
                    <td>{r.firmware || "–"}</td>
                    <td>{r.cpu === null ? "–" : `${r.cpu}%`}</td>
                    <td>{r.tempC === null ? "–" : `${r.tempC.toFixed(1)} °C`}</td>
                    <td>{r.restart || "–"}</td>
                    <td class="hub-actions">
                      <div class="hub-action-row">
                        {may.identify ? (
                          <button
                            type="button"
                            disabled={busy || !!may.identifyOff}
                            title={may.identifyOff || "Blink this DSPi's LED"}
                            onClick={() => void run(() => feed.identify(r.serial))}
                          >
                            Identify
                          </button>
                        ) : null}
                        {may.open ? (
                          <button type="button" onClick={() => session.openDevice!(r.serial)}>
                            Open
                          </button>
                        ) : null}
                        {may.edit ? (
                          <button
                            type="button"
                            disabled={busy}
                            aria-label={`Edit the details of ${r.name}`}
                            onClick={() => setEditing({ serial: r.serial, location: r.location, note: r.note, tags: r.tags.join(", ") })}
                          >
                            Edit…
                          </button>
                        ) : null}
                        {may.forget ? (
                          <button type="button" class="danger-text" disabled={busy} aria-label={`Forget ${r.name}`} onClick={() => void forget(r)}>
                            Forget…
                          </button>
                        ) : null}
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      ) : view.caps?.inventory ? (
        <p class="note">The hub has seen no DSPi yet.</p>
      ) : null}
      {editing && edited ? (
        <form
          class="hub-edit"
          aria-label={`Details of ${edited.name}`}
          onSubmit={(e) => {
            e.preventDefault();
            const d = { location: editing.location.trim(), note: editing.note, tags: tagsFrom(editing.tags) };
            void run(async () => {
              const r = await feed.setDetails(editing.serial, d);
              if (!r.bad) setEditing(null);
              return r;
            });
          }}
        >
          <h3>Details of {edited.name}</h3>
          <label>
            Location
            <input value={editing.location} maxLength={256} onInput={(e) => setEditing({ ...editing, location: (e.target as HTMLInputElement).value })} />
          </label>
          <label>
            Note
            <textarea value={editing.note} maxLength={256} rows={2} onInput={(e) => setEditing({ ...editing, note: (e.target as HTMLTextAreaElement).value })} />
          </label>
          <label>
            Tags (separated by commas)
            <input value={editing.tags} onInput={(e) => setEditing({ ...editing, tags: (e.target as HTMLInputElement).value })} />
          </label>
          <div class="hub-edit-actions">
            <button type="submit" disabled={busy}>
              Save
            </button>
            <button type="button" onClick={() => setEditing(null)}>
              Cancel
            </button>
          </div>
        </form>
      ) : null}
    </section>
  );
}

// -- WP P2WB3 --
/** The hub's name, and (Admin, a hub with `hub_stats`) Rename. */
function HubName(props: { feed: HubFeed; name: string }) {
  const [text, setText] = useState<string | null>(null);
  const [said, setSaid] = useState<Said | null>(null);
  const [busy, setBusy] = useState(false);
  const save = async (name: string) => {
    setBusy(true);
    const r = await props.feed.rename(name.trim());
    setSaid(r);
    if (!r.bad) setText(null);
    setBusy(false);
  };
  return (
    <section class="card hub-name-card" aria-label="The hub">
      <div class="card-head">
        <h2>The hub</h2>
      </div>
      <p>
        Called <strong class="hub-own-name">{props.name || "–"}</strong>.{" "}
        {text === null ? (
          <button type="button" onClick={() => setText(props.name)}>
            Rename the Hub…
          </button>
        ) : null}
      </p>
      {said ? (
        <p class={said.bad ? "error" : "note"} role="status" aria-label="Rename" data-key={said.notice?.key}>
          {said.text}
          {said.bad && said.notice?.reason ? ` ${said.notice.reason}` : ""}
        </p>
      ) : null}
      {text !== null ? (
        <form
          class="hub-edit"
          aria-label="Rename the hub"
          onSubmit={(e) => {
            e.preventDefault();
            if (text.trim()) void save(text);
          }}
        >
          <label>
            New name
            <input value={text} maxLength={64} onInput={(e) => setText((e.target as HTMLInputElement).value)} />
          </label>
          <div class="hub-edit-actions">
            <button type="submit" disabled={busy || !text.trim()}>
              Rename
            </button>
            <button type="button" onClick={() => setText(null)}>
              Cancel
            </button>
          </div>
        </form>
      ) : null}
    </section>
  );
}

function StateWord(props: { row: HubRow }) {
  const r = props.row;
  if (!r.online) return <span class="hub-state offline">Offline{r.lastSeen ? `, last seen ${shortTime(r.lastSeen)}` : ""}</span>;
  if (r.unresponsive) return <span class="hub-state bad">Not answering</span>;
  return <span class="hub-state ok">{r.isOpen ? "Online (open here)" : "Online"}</span>;
}

function roleWord(role: string): string {
  return role === "admin" ? "Admin" : role === "control" ? "Control" : role === "viewer" ? "Viewer" : role;
}

function AlarmBoard(props: { feed: HubFeed; view: HubView; readOnly: string }) {
  const { feed, view } = props;
  const [said, setSaid] = useState<Said | null>(null);
  const summary = alarmSummary(view.alarms);
  const mayAck = !props.readOnly && view.role !== "viewer";
  const ack = async (a: HubAlarmRecord) => {
    const r = await feed.acknowledge(a.id);
    setSaid(r.bad ? r : null);
  };
  return (
    <section class="card hub-alarms" aria-label="Alarms">
      <div class="card-head">
        <h2>Alarms</h2>
        <span class={summary.raised ? (summary.critical ? "hub-count bad" : "hub-count warn") : "hub-count"}>
          {summary.raised ? `${summary.raised} to acknowledge` : "None to acknowledge"}
        </span>
      </div>
      {said ? (
        <p class="error" role="alert" data-key={said.notice?.key}>
          {said.text}
        </p>
      ) : null}
      {view.alarms.length ? (
        <ul class="hub-alarm-list">
          {view.alarms.map((a) => {
            const n = view.notices[a.id] ?? null;
            const about = a.serial ? feed.deviceName(a.serial) : "";
            return (
              <li key={a.id} class={`hub-alarm ${a.state} ${a.severity === "critical" ? "bad" : "warn"}`} aria-label={alarmTitle(a, n, about)} data-key={a.noticeKey}>
                <div class="hub-alarm-head">
                  <strong>{alarmTitle(a, n, about)}</strong>
                  <span class="hub-alarm-state">{stateWord(a)}</span>
                </div>
                <p class="hub-alarm-detail">
                  {a.message}
                  {a.count > 1 ? ` (${a.count}×)` : ""}
                </p>
                {n?.action ? <p class="note">{n.action}</p> : null}
                <p class="hub-alarm-time">
                  Raised {shortTime(a.raisedAt)}
                  {a.clearedAt ? `, cleared ${shortTime(a.clearedAt)}` : ""}
                  {a.acknowledgedBy ? `, acknowledged by ${a.acknowledgedBy}` : ""}
                </p>
                {a.state === "raised" ? (
                  <button type="button" disabled={!mayAck} title={props.readOnly || (view.role === "viewer" ? "A Viewer cannot acknowledge alarms" : undefined)} onClick={() => void ack(a)}>
                    Acknowledge
                  </button>
                ) : null}
              </li>
            );
          })}
        </ul>
      ) : (
        <p class="note">No alarms.</p>
      )}
    </section>
  );
}

function stateWord(a: HubAlarmRecord): string {
  return a.state === "raised" ? "Raised" : a.state === "acknowledged" ? "Acknowledged" : a.state === "cleared" ? "Cleared" : a.state;
}

function HubStats(props: { stats: HubStatsRecord; name: (serial: string) => string }) {
  const s = props.stats;
  return (
    <section class="card hub-stats" aria-label="Hub statistics">
      <h2>Hub statistics</h2>
      <p>
        {s.sessions} {s.sessions === 1 ? "session" : "sessions"} open; running for {durationText(s.uptimeS)}.
      </p>
      {s.devices.length ? (
        <div class="hub-table-wrap">
          <table class="hub-table">
            <thead>
              <tr>
                <th scope="col">DSPi</th>
                <th scope="col">Commands</th>
                <th scope="col">Errors</th>
                <th scope="col">Mean time</th>
                <th scope="col">Dropped notifications</th>
              </tr>
            </thead>
            <tbody>
              {s.devices.map((d) => (
                <tr key={`${d.handle}:${d.serial}`}>
                  <td>{props.name(d.serial)}</td>
                  <td>{d.cmds}</td>
                  <td>{d.errors}</td>
                  <td>{`${d.avgRttMs.toFixed(1)} ms`}</td>
                  <td>{d.notifyDropped}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
    </section>
  );
}

/**
 * The alarm banner above every screen while the hub has device alarms to acknowledge (the
 * audio link's alarms have the System view's status rail).
 */
export function HubAlarmBanner(props: { feed: HubFeed; onShow: (() => void) | null }) {
  const view = useHubView(props.feed);
  if (!view?.caps?.alarms) return null;
  const s = alarmSummary(view.alarms.filter((a) => a.source !== "alink"));
  if (!s.raised) return null;
  return (
    <div class={s.critical ? "hub-alarm-banner bad" : "hub-alarm-banner"} role="status" aria-label="Hub alarms">
      <span>{s.raised === 1 ? "The hub has 1 alarm to acknowledge." : `The hub has ${s.raised} alarms to acknowledge.`}</span>
      {props.onShow ? (
        <button type="button" onClick={props.onShow}>
          Show Alarms
        </button>
      ) : null}
    </div>
  );
}
