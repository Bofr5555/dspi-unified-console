// WP P2WB1: the hub screen of a session on a hub that has the hub's own
// families (PRO phase 2): every DSPi the hub has seen, online or not, with its
// health; Identify on every online DSPi and Open for the others; an Admin
// sees the hub's statistics; the alarm board with Acknowledge. Everything
// comes from the core's typed calls through `HubFeed`; on a hub without these
// families (an upstream hub) the screen is not offered.

//
// -- WP UB6 -- (UXP B6, W10) Not one long stack: the page's parts as Qt's pages are, Devices · Alarms ·
// Updates · Groups · Hub (each where the hub has its family); the device table and the health board
// one table (DeviceTable.tsx), its details on expand; the hub's name, this console's role and the
// statistics in Hub. The alarm banner's Show Alarms opens the Alarms part.

import type { FunctionComponent } from "preact";
import { useEffect, useState } from "preact/hooks";
import type { HubAlarmRecord, HubStatsRecord } from "../../core/protocol";
import type { ScreenDef, ScreenProps } from "../../ui/screens";
import { HubFeed, type HubView, type Said } from "./feed";
import { alarmSummary, alarmTitle, durationText, shortTime } from "./model";
import { useSessionState } from "../../ui/components";
import { DeviceTable, roleWord } from "./DeviceTable"; // -- WP UB6 -- the device table and the health board in one
import { PageParts, usePart, type Part } from "../../ui/shell/Parts"; // -- WP UB6 --
import { placeOf } from "../../ui/shell/places"; // -- WP UB6 --
import { HubGroups, useHubGroups } from "./Groups"; // -- WP P3WGR --
import { Rollout } from "./Rollout"; // -- WP P3WRO --
import { rolloutShown } from "./rollout-model"; // -- WP P3WRO --
// -- WP WEB5 -- the page in the Modern design (Qt's Devices › Hubs): sections, not cards; its own stylesheet
import { Glyph } from "../../ui/design";
import "./hub.css";

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
  const st = useSessionState(props.session);
  const hubName = st.hubName; // -- WP P2WB3 --
  // -- WP WEB5 -- the hub gone: what the page shows is its last word. -- WP REVEFIX -- (REVE C4) when the hub did not
  // answer (the feed's `stale`), not whenever the link is not bound; its actions off meanwhile. (REVE D5) one notice:
  // the strip's, which says the link is lost; the page only dims
  const away = !!view?.stale;
  const groups = useHubGroups(props.session, !!view?.caps?.onHub && !!view.caps.groups); // -- WP P3WGR --
  // -- WP UB6 -- the parts the hub has families for
  const caps = view?.caps ?? null;
  const parts: Part[] = [
    ...(caps?.inventory || caps?.health ? [{ id: "devices", label: "Devices" }] : []),
    ...(caps?.alarms ? [{ id: "alarms", label: "Alarms" }] : []),
    ...(view && rolloutShown(caps, view.role) ? [{ id: "updates", label: "Updates" }] : []),
    ...(caps?.groups ? [{ id: "groups", label: "Groups" }] : []),
    { id: "hub", label: "Hub" },
  ];
  const [part, setPart] = usePart(props.session, HUB_SCREEN_ID, parts.map((x) => x.id));
  if (!feed || !view) return null;
  return (
    <div class={away ? "hub-screen away" : "hub-screen"} data-part={part} aria-disabled={away || undefined} title={away ? "The hub does not answer: this is what it said last" : undefined}>
      <PageParts label="Hub" parts={parts} current={part} onPart={setPart} />
      <fieldset class="hub-fence" disabled={away}>
      {/* -- SEC hook (SECWEB, PLAN-SEC section 6) -- the "unencrypted" banner and the hub's fingerprint go here */}
      {part === "devices" ? (
        <DeviceTable feed={feed} view={view} props={props} groups={groups.view.groups} onAlarms={caps?.alarms ? () => setPart("alarms") : null} identifyFailure={identifyFailure} />
      ) : null}
      {/* -- WP P3WGR -- */ part === "groups" && caps?.groups ? <HubGroups session={props.session} ask={props.ask} view={view} groups={groups} name={(s) => feed.deviceName(s)} /> : null}
      {/* -- WP P3WRO -- the firmware rollout: an Admin's, on a hub with the `rollout` cap */ part === "updates" && rolloutShown(caps, view.role) ? (
        <Rollout session={props.session} hub={feed} view={view} ask={props.ask} readOnly={props.session.readOnlyReason} />
      ) : null}
      {part === "alarms" && caps?.alarms ? <AlarmBoard feed={feed} view={view} readOnly={props.session.readOnlyReason} /> : null}
      {part === "hub" ? (
        <>
          {/* -- WP P2WB3 -- */}
          <HubName feed={feed} name={hubName} role={view.role} rename={!!caps?.hubStats && view.role === "admin" && !props.session.readOnlyReason} />
          {view.stats ? <HubStats stats={view.stats} name={(s) => feed.deviceName(s)} /> : null}
        </>
      ) : null}
      </fieldset>
    </div>
  );
};

// -- WP UWN -- (UXP A16)
/** "Identify <who>: <title>. <reason>": the core's notice joined into one sentence that names the action and the device. */
export function identifyFailure(who: string, said: Said): string {
  const n = said.notice;
  const title = n ? n.title : said.text;
  const head = title ? title[0]!.toLowerCase() + title.slice(1) : "it did not work";
  const reason = n?.reason ? ` ${n.reason}` : "";
  return `Identify ${who}: ${head.replace(/[.]$/, "")}.${reason}`;
}

export const HUB_SCREEN: ScreenDef = { id: HUB_SCREEN_ID, label: "Hub", View: HubScreen };

// -- WP P2WB3 --
/** The hub's name, and (Admin, a hub with `hub_stats`) Rename; -- WP UB6 -- this console's role on it. */
function HubName(props: { feed: HubFeed; name: string; role: string; rename: boolean }) {
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
        {props.role ? <span class="hub-role-tag">{roleWord(props.role)}</span> : null}{" "}
        {props.rename && text === null ? (
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
      <Glyph name={s.critical ? "error" : "warning"} class="notice-icon" />
      <span>{s.raised === 1 ? "The hub has 1 alarm to acknowledge." : `The hub has ${s.raised} alarms to acknowledge.`}</span>
      {props.onShow ? (
        <button
          type="button"
          onClick={() => {
            placeOf(props.feed.session).parts.set(HUB_SCREEN_ID, "alarms"); // -- WP UB6 -- the Alarms part
            props.onShow!();
          }}
        >
          Show Alarms
        </button>
      ) : null}
    </div>
  );
}
