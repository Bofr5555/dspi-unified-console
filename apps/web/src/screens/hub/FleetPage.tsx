// -- WP UB6 -- (UXP B6, owner D13) The Fleet: a hub-served console's first page, the hub's DSPis
// with no DSPi open. One table (name, location, state, alarms, firmware; Open, and Identify where
// the hub has it and the role allows) under the counts (online, offline, alarms to acknowledge).
//
// It reads the hub through a session of its own on the serving hub, left in Link's choose step
// (`choosing`: authenticated, no DSPi bound), which is enough for the hub's own families
// (`hubCaps`, `hubInventory`, `hubHealth`, `hubAlarms`: they need the hub, not a device); the
// same `HubFeed` as the hub page's, so its reads, events, reconnects and stale view are the same.
// Open opens the DSPi in a tab of its own, as the device list's Connect does. Shown when the page's
// backend lists the hub's inventory (a hub that wants its PIN first, or an upstream hub, has no
// Fleet: the page starts on This Computer as before).

import { useEffect, useState } from "preact/hooks";
import { HUB_INSTALLATION_ID } from "../../fleet/hub";
import type { Installation, Source } from "../../fleet/types";
import { useSessionState } from "../../ui/components";
import { EmptyState, NoticeRow } from "../../ui/design";
import type { Session } from "../../ui/session";
import { HubFeed, identifyRefused, refuseIdentify, type HubView, type Said } from "./feed";
import { roleWord, StateWord } from "./DeviceTable";
import { identifyFailure, useHubView } from "./HubScreen";
import { alarmCountText, alarmTitle, deviceAlarms, deviceRows, fleetSummary, rowActions, type HubRow } from "./model";
import "./hub.css";

/** The hub a page's Fleet is of: the backend's installation of its inventory, and the hub's address. */
export interface FleetHub {
  inst: Installation;
  url: string;
}

/** The serving hub of `installations` (the hub backend lists its inventory as an installation of its own); null without one. */
export function fleetHubOf(installations: readonly Installation[]): FleetHub | null {
  const inst = installations.find((i) => i.id === HUB_INSTALLATION_ID);
  if (!inst) return null;
  const listed = inst.sources.find((s) => s.kind === "hub");
  // A hub that has seen no DSPi yet lists none: its own entry in this console's list (the serving hub, listed first)
  const own = installations.flatMap((i) => i.sources).find((s) => s.kind === "hub" && !s.deviceId);
  const url = listed?.kind === "hub" ? listed.url : own?.kind === "hub" ? own.url : "";
  return url ? { inst, url } : null;
}

/** The same hub whatever the scheme the list gives it (`ws://` and, on an https page, `wss://`). */
const sameHub = (a: string, b: string) => a.replace(/^wss?:/, "") === b.replace(/^wss?:/, "");

/**
 * The installations as the device list shows them beside a Fleet: without the serving hub's own entry (Link's choose
 * step), whose DSPis the hub's installation already lists, each with its Connect (UXP B6: the hub was listed twice).
 */
export function withoutFleetEntry(installations: readonly Installation[], fleet: FleetHub | null): Installation[] {
  if (!fleet) return [...installations];
  return installations.map((i) =>
    i.id === fleet.inst.id ? i : { ...i, sources: i.sources.filter((s) => !(s.kind === "hub" && !s.deviceId && sameHub(s.url, fleet.url))) },
  );
}

/** The Fleet's session on the hub and its feed; `retry` starts them again. */
export interface Fleet {
  session: Session;
  feed: HubFeed;
  retry: () => void;
}

/**
 * The Fleet's own session on `hub` (made by `make`), opened into the hub's choose step; its feed reads while `active`
 * (the Fleet on show) once the session is there. Both stop when the hub goes from the list or the console goes away.
 */
export function useFleet(hub: FleetHub | null, active: boolean, make: (hub: FleetHub) => Promise<Session>): Fleet | null {
  const key = hub ? `${hub.inst.id} ${hub.url}` : "";
  const [tries, setTries] = useState(0);
  const [fleet, setFleet] = useState<Fleet | null>(null);
  const [ready, setReady] = useState<Fleet | null>(null);
  useEffect(() => {
    setFleet(null);
    setReady(null);
    if (!hub) return;
    let alive = true;
    let made: Fleet | null = null;
    void make(hub).then(async (session) => {
      if (!alive) return void session.close();
      const f: Fleet = { session, feed: new HubFeed(session), retry: () => setTries((n) => n + 1) };
      made = f;
      setFleet(f);
      await session.open({ kind: "hub", url: hub.url, deviceId: "" });
      if (alive) setReady(f);
    });
    return () => {
      alive = false;
      if (!made) return;
      made.feed.stop();
      void made.session.close();
    };
  }, [key, tries]);
  useEffect(() => {
    if (!ready || !active) return;
    ready.feed.start();
    return () => ready.feed.stop();
  }, [ready, active]);
  return fleet;
}

/** How many DSPis the hub shares now: its inventory's online ones as the feed read them, else as the list had them. */
export function sharedCount(hub: FleetHub | null, view: HubView | null): number | null {
  if (view?.caps?.inventory) return view.devices.filter((d) => d.online).length;
  if (!hub) return null;
  return hub.inst.sources.filter((s) => s.kind === "hub" && s.online !== false).length;
}

export function FleetPage(props: {
  fleet: Fleet | null;
  /** The hub's name as the list has it, until its session says it. */
  name: string;
  onOpen: (source: Source) => void;
  /** The address the DSPis are opened on (the list's, so a DSPi open from the list counts as open here). */
  url: string;
  openKeys: Set<string>;
  readOnly: boolean;
}) {
  const fleet = props.fleet;
  if (!fleet)
    return (
      <section class="hub-screen fleet-page" aria-label="Fleet">
        <EmptyState icon="network" title="Asking the hub" text="The hub's DSPis show here in a moment." />
      </section>
    );
  return <FleetView {...props} fleet={fleet} />;
}

function FleetView(props: { fleet: Fleet; name: string; onOpen: (source: Source) => void; url: string; openKeys: Set<string>; readOnly: boolean }) {
  const { session, feed } = props.fleet;
  const st = useSessionState(session);
  const view = useHubView(feed);
  const [said, setSaid] = useState<Said | null>(null);
  const [busy, setBusy] = useState(false);
  const [, refused] = useState(0);
  useEffect(() => {
    if (!said) return;
    const t = setTimeout(() => setSaid(null), 10_000);
    return () => clearTimeout(t);
  }, [said]);
  const name = st.hubName || props.name;
  if (st.status === "failed")
    return (
      <section class="hub-screen fleet-page" aria-label="Fleet">
        <NoticeRow kind="error" role="alert" action={{ label: "Try Again", run: props.fleet.retry }}>
          Cannot read the hub's DSPis: {st.error}
        </NoticeRow>
      </section>
    );
  if (st.status === "pairing")
    return (
      <section class="hub-screen fleet-page" aria-label="Fleet">
        <NoticeRow kind="info">The hub asks for its PIN: connect to it on This Computer to pair.</NoticeRow>
      </section>
    );
  if (!view?.caps)
    return (
      <section class="hub-screen fleet-page" aria-label="Fleet">
        <EmptyState icon="network" title="Asking the hub" text="The hub's DSPis show here in a moment." />
      </section>
    );
  const rows = deviceRows(view.devices, view.health, "");
  const sum = fleetSummary(view.devices, view.alarms);
  const ro = props.readOnly ? "This page is read only" : "";
  const opts = { role: view.role, identify: !!view.caps.identify, inventory: !!view.caps.inventory, readOnly: ro, canOpen: true };
  const identify = async (r: HubRow) => {
    setBusy(true);
    const r1 = await feed.identify(r.serial);
    if (r1.bad) {
      const who = rows.some((o) => o.serial !== r.serial && o.name === r.name) ? `${r.name} (${r.serial})` : r.name;
      const text = identifyFailure(who, r1);
      if (r1.notice?.key === "unsupported") {
        refuseIdentify(r.serial, text);
        refused((n) => n + 1);
      }
      setSaid({ text, bad: true, notice: r1.notice ? { ...r1.notice, reason: "" } : null });
    } else setSaid(r1);
    setBusy(false);
  };
  const away = view.stale;
  return (
    <section class={away ? "hub-screen fleet-page away" : "hub-screen fleet-page"} aria-label="Fleet" title={away ? "The hub does not answer: this is what it said last" : undefined}>
      <div class="fleet-head">
        <strong class="fleet-hub-name">{name}</strong>
        <span class="hub-count fleet-online">{sum.online} online</span>
        {sum.offline ? <span class="hub-count fleet-offline">{sum.offline} offline</span> : null}
        {view.caps.alarms ? (
          <span class={sum.raised ? (sum.critical ? "hub-count bad" : "hub-count warn") : "hub-count"} aria-label="Alarms to acknowledge">
            {sum.raised ? `${alarmCountText(sum.raised)} to acknowledge` : "No alarms to acknowledge"}
          </span>
        ) : null}
        {view.role ? <span class="hub-role-tag">{roleWord(view.role)}</span> : null}
      </div>
      {view.error ? (
        <p class="error" role="alert">
          {view.error}
        </p>
      ) : null}
      {said ? (
        <p class={said.bad ? "error" : "note"} role="status" aria-label="Action result" data-key={said.notice?.key}>
          {said.text}
        </p>
      ) : null}
      {rows.length ? (
        <fieldset class="hub-fence" disabled={away}>
          <table class="hub-table fleet-table">
            <thead>
              <tr>
                <th scope="col">Name</th>
                <th scope="col">Location</th>
                <th scope="col">State</th>
                {view.caps.alarms ? <th scope="col">Alarms</th> : null}
                <th scope="col">Firmware</th>
                <th scope="col">
                  <span class="visually-hidden">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => {
                const may = rowActions(r, opts);
                const source: Source = { kind: "hub", url: props.url, deviceId: r.serial, name: r.name, online: r.online };
                const isOpen = props.openKeys.has(`${HUB_INSTALLATION_ID}/hub:${props.url}#${r.serial}`);
                const on = deviceAlarms(view.alarms, r.serial);
                const raised = on.filter((a) => a.state === "raised");
                const cannot = identifyRefused(r.serial);
                return (
                  <tr key={r.serial} class={r.online ? undefined : "offline"} aria-label={r.name}>
                    <td data-label="Name">
                      <span class="hub-name">{r.name}</span>
                      <span class="hub-serial">{r.serial}</span>
                    </td>
                    <td data-label="Location">{r.location || "–"}</td>
                    <td data-label="State">
                      <StateWord row={r} />
                    </td>
                    {view.caps?.alarms ? (
                      <td data-label="Alarms" class="fleet-alarms" title={on.map((a) => alarmTitle(a, view.notices[a.id] ?? null, "")).join("\n") || undefined}>
                        {on.length ? (
                          <span class={`fleet-alarm-count ${raised.some((a) => a.severity === "critical") ? "bad" : raised.length ? "warn" : "quiet"}`}>{alarmCountText(on.length)}</span>
                        ) : (
                          "–"
                        )}
                      </td>
                    ) : null}
                    <td data-label="Firmware">{r.firmware || "–"}</td>
                    <td class="hub-actions">
                      <div class="hub-action-row">
                        {may.identify && !cannot ? (
                          <button type="button" disabled={busy || !!may.identifyOff} title={may.identifyOff || "Blink this DSPi's LED"} aria-label={`Identify ${r.name}`} onClick={() => void identify(r)}>
                            Identify
                          </button>
                        ) : null}
                        {r.online ? (
                          <button type="button" disabled={isOpen} aria-label={isOpen ? `${r.name} is open` : `Open ${r.name}`} onClick={() => props.onOpen(source)}>
                            Open
                          </button>
                        ) : null}
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </fieldset>
      ) : (
        <EmptyState icon="network" title="No DSPi yet" text="The hub lists every DSPi attached to it here." />
      )}
    </section>
  );
}
