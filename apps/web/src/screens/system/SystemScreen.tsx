// WP SW3, WB6d: the System view. Rooms and zones with their units, each
// unit's state (connected, the audio link's lock, alarms), and the topology:
// the sender, fan-outs, units, the alinks between them and the audio link channels
// each unit takes. A device the console holds itself is a system of one unit.
// -- WP WEB7 -- This is Qt's Units tab; Clock and Links, Installation and the signal chain
// are pages of their own (pages.tsx).
// (WB6d's signal chain, read only, a block opening the screen that edits it, is the
// section's Signal Chain tab.)

import { useEffect, useState } from "preact/hooks";
import type { ScreenProps } from "../../ui/screens";
import { ZoneVolume } from "./ZoneVolume"; // -- WP FEATUI --
import type { Session } from "../../ui/session";
import { NoticeRow } from "../../ui/design"; // -- WP WEB7 --
import { Group } from "../../ui/Group"; // -- WP WEB7 --
import { NotOnHub } from "./pages"; // -- WP WEB7 --
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
  if (!feed) return null;
  // -- WP WEB7 -- Qt's Units tab: on a device this console holds itself, Qt's empty state with the
  // way to the hubs; Clock and Links, Installation and the signal chain are tabs of their own (pages.tsx)
  if (!feed.onHub) return <NotOnHub go={props.go} />;
  return <SystemView feed={feed} ask={props.ask} session={props.session} />;
}

function SystemView(props: { feed: SystemFeed; ask: ScreenProps["ask"]; session: Session /* -- WP FEATUI -- */ }) {
  const st = useFeed(props.feed);
  useEffect(() => props.feed.show(), [props.feed]);
  const { model, status } = st;
  if (!model) return <p class="sys-note">{st.error || "Reading the system…"}</p>;
  const single = st.kind === "single";
  const unit = model.units[0];
  const observed = unit ? status?.observed[unit.serial] : undefined;
  // -- WP WEB7 -- Qt's look: the error a notice row, each part a group with its icon and title outside the panel
  return (
    <section class="sys sys-page" aria-label="System">
      {st.error ? <NoticeRow kind="warning" role="alert">{st.error}</NoticeRow> : null}
      {single && unit ? (
        <Group title="This Device" icon="cpu" footer="This hub has no audio link installation: the device is shown as a single unit.">
          <section class="sys-room" aria-label="This device">
            <UnitRow u={singleRow(st)} role={alinkRoleWord(observed?.role)} />
          </section>
        </Group>
      ) : (
        <>
          {/* -- WP FEATUI -- a room's or zone's volume (FEAT2): Control or Admin on a hub */}
          {!props.feed.readOnly && (st.role === "admin" || st.role === "control") ? <ZoneVolume model={model} session={props.session} ask={props.ask} /> : null}
          <Group title="Units" icon="network" flat>
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
          </Group>
          <Group title="Topology" icon="link">
            <ul class="sys-topo" aria-label="Topology">
              {topology(model, status).map((n) => (
                <TopoItem key={n.id} n={n} />
              ))}
            </ul>
          </Group>
        </>
      )}
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
