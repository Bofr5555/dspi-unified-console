// WP SW3, WB6d: the System view. Rooms and zones with their units, each
// unit's state (connected, the audio link's lock, alarms), and the topology:
// the sender, fan-outs, units, the alinks between them and the audio link channels
// each unit takes. A device the console holds itself is a system of one unit.
// The Installer mode adds the "Clock and audio links" panel (AlinkClock.tsx).
// -- WP WB6d -- and, for the session's own device, its signal chain (SignalChain.tsx):
// read only, a block opens the screen that edits it.

import { useEffect, useState } from "preact/hooks";
import type { ScreenProps } from "../../ui/screens";
import { AlinkClock } from "./AlinkClock";
import { SignalChain } from "./SignalChain"; // -- WP WB6D --
import { InstallationTab } from "./installation/InstallationTab"; // -- WP INSTW --
import type { ComponentChildren } from "preact";
import type { FeedState, SystemFeed } from "./feed";
import { NO_ALINK, NO_USB_DIRECT, alinkRoleWord, lockWord, roomsView, slotWord, topology, type TopoNode, type UnitView } from "./model";

export function useFeed(feed: SystemFeed): FeedState {
  const [, bump] = useState(0);
  const rendered = feed.state;
  useEffect(() => {
    const stop = feed.watch(() => bump((n) => n + 1));
    if (feed.state !== rendered) bump((n) => n + 1);
    return stop;
  }, [feed]);
  return feed.state;
}

export function SystemScreen(props: ScreenProps) {
  const feed = props.system;
  const [tab, setTab] = useState<"overview" | "installation">("overview"); // -- WP INSTW --
  if (!feed) return null;
  // -- WP INSTW -- on a hub, the Installer mode sets the installation up in its own tab
  // -- WP WEBM3 -- (review RB C-2) in Advanced too, as Qt's INSTQ panel: saving stays the hub's Admin's (`mayEdit`); the owner may narrow it
  const tabs = (props.mode === "installer" || props.mode === "advanced") && feed.onHub;
  if (tabs && tab === "installation") return <SystemTabs tab={tab} show={setTab}><InstallationTab feed={feed} session={props.session} ask={props.ask} /></SystemTabs>;
  // -- WP WB6D -- the device's signal chain, whatever the system's state
  const overview = (
    <>
      <SystemView feed={feed} installer={props.mode === "installer"} ask={props.ask} />
      <SignalChain session={props.session} open={props.open} />
    </>
  );
  return tabs ? <SystemTabs tab={tab} show={setTab}>{overview}</SystemTabs> : overview;
}

// -- WP INSTW --
function SystemTabs(props: { tab: "overview" | "installation"; show: (t: "overview" | "installation") => void; children: ComponentChildren }) {
  return (
    <>
      <div class="screen-tabs" role="tablist" aria-label="System">
        {(["overview", "installation"] as const).map((t) => (
          <button type="button" role="tab" key={t} aria-selected={props.tab === t} onClick={() => props.show(t)}>
            {t === "overview" ? "Status" : "Installation"}
          </button>
        ))}
      </div>
      {props.children}
    </>
  );
}

function SystemView(props: { feed: SystemFeed; installer: boolean; ask: ScreenProps["ask"] }) {
  const st = useFeed(props.feed);
  useEffect(() => props.feed.show(), [props.feed]);
  const { model, status } = st;
  if (!model) return <p class="sys-note">{st.error || "Reading the system…"}</p>;
  const single = st.kind === "single";
  const unit = model.units[0];
  const observed = unit ? status?.observed[unit.serial] : undefined;
  return (
    <section class="sys" aria-label="System">
      {st.error ? (
        <p class="error" role="alert">
          {st.error}
        </p>
      ) : null}
      {single && unit ? (
        <>
          <h3>This device</h3>
          <p class="sys-note">
            {props.feed.onHub ? "This hub has no audio link installation: the device is shown as a single unit." : "One device, shown as a single unit."}
          </p>
          <div class="sys-rooms">
            <section class="sys-room" aria-label="This device">
              <UnitRow u={singleRow(st)} role={alinkRoleWord(observed?.role)} />
            </section>
          </div>
        </>
      ) : (
        <>
          <h3>Rooms and zones</h3>
          <div class="sys-rooms">
            {roomsView(model, status).map((r) => (
              <section class="sys-room" key={r.id || "none"} aria-label={`Room ${r.name}`}>
                <header>
                  <strong>{r.name}</strong>
                  {r.zones.map((z) => (
                    <span class="sys-chip" key={z} title="Zone">
                      {z}
                    </span>
                  ))}
                </header>
                {r.units.length ? r.units.map((u) => <UnitRow key={u.id} u={u} />) : <p class="sys-note">No units</p>}
              </section>
            ))}
          </div>
          <h3>Topology</h3>
          <ul class="sys-topo" aria-label="Topology">
            {topology(model, status).map((n) => (
              <TopoItem key={n.id} n={n} />
            ))}
          </ul>
        </>
      )}
      {props.installer && !single ? <AlinkClock feed={props.feed} ask={props.ask} /> : null}
      {props.installer && single ? (
        <p class="sys-note">Clock and audio links: the audio link's alignment is set by a hub with an installation.</p>
      ) : null}
    </section>
  );
}

/** The one unit of a device on its own: its alink's state, or that it has none. */
function singleRow(st: FeedState): UnitView {
  const u = st.model!.units[0]!;
  const o = st.status?.observed[u.serial];
  return {
    id: u.id,
    name: u.name,
    serial: u.serial,
    connected: !!o?.online,
    lock: o?.role ? lockWord(o.state) : "No audio link",
    alarms: [],
    slots: u.slots ?? [],
  };
}

function UnitRow(props: { u: UnitView; role?: string }) {
  const { u } = props;
  const taken = u.slots.filter((c) => c !== 255);
  return (
    <div class="sys-unit" aria-label={`Unit ${u.name}`} data-connected={u.connected} data-lock={u.lock}>
      <span class={`dot ${u.connected ? "dot-open" : "dot-failed"}`} aria-hidden="true" />
      <span class="sys-unit-name">{u.name}</span>
      <span class="sys-state">{u.connected ? "Connected" : "Offline"}</span>
      {props.role ? <span class="sys-state">{props.role}</span> : null}
      <span class={`sys-lock sys-lock-${u.lock.toLowerCase().replace(/\s+/g, "-")}`}>
        {/* -- WP AUSB1 -- a USB-direct unit says so */}
        {u.lock === "No audio link" || u.lock === NO_ALINK || u.lock === NO_USB_DIRECT
          ? u.lock
          : u.usb
            ? `USB-direct: ${u.lock}`
            : `Audio link: ${u.lock}`}
      </span>
      {u.usb?.stats ? <span class="sys-usb-stats">{u.usb.stats}</span> : null}
      {u.alarms.length ? (
        <span class="sys-alarm-count" title={u.alarms.map((a) => a.message).join("\n")}>
          {u.alarms.length === 1 ? "1 alarm" : `${u.alarms.length} alarms`}
        </span>
      ) : null}
      {taken.length ? <span class="sys-slots">{taken.map(slotWord).join(", ")}</span> : null}
    </div>
  );
}

function kindWord(n: TopoNode): string {
  if (n.kind === "sender") return "Sender";
  if (n.kind === "usb_direct") return "USB-direct"; // -- WP AUSB1 --
  if (n.kind === "fan_out") return "Fan-out";
  return n.hopDelayFrames !== null ? "Repeater" : "Unit";
}

function TopoItem(props: { n: TopoNode }) {
  const { n } = props;
  return (
    <li>
      <div class={`sys-node sys-node-${n.kind}`} aria-label={`${kindWord(n)} ${n.name}`}>
        {n.output !== null ? <span class="sys-link">output {n.output + 1}</span> : null}
        <span class={`dot ${n.connected ? "dot-open" : "dot-failed"}`} aria-hidden="true" />
        <span class="sys-kind">{kindWord(n)}</span>
        <strong>{n.name}</strong>
        <span class="sys-state">{n.lock}</span>
        {n.hopDelayFrames !== null ? <span class="sys-chip">+{n.hopDelayFrames} frames</span> : null}
        {n.alarms ? <span class="sys-alarm-count">{n.alarms === 1 ? "1 alarm" : `${n.alarms} alarms`}</span> : null}
        {n.slots ? (
          <span class="sys-slot-row" aria-label={`Audio link slots of ${n.name}`}>
            {n.slots.map((c, i) => (
              <span key={i} class={`sys-slot${c === 255 ? " sys-slot-silent" : ""}`} title={`IN${i + 1}: ${slotWord(c)}`}>
                {c === 255 ? "–" : c}
              </span>
            ))}
          </span>
        ) : null}
      </div>
      {n.children.length ? (
        <ul>
          {n.children.map((c) => (
            <TopoItem key={c.id} n={c} />
          ))}
        </ul>
      ) : null}
    </li>
  );
}
