// The skeleton's UI (WP WB5): device picker, connection strip, lost-link banner,
// a volume control with read-back, meters, and About. Later WPs (WB6 slices) add
// screens beside these; they share the session model and never reach the core
// except through it.

import { useEffect, useState } from "preact/hooks";
import type { Installation, Source } from "../fleet/types";
import { sourceKey } from "../fleet/types";
import type { PageEnv, UsbChoice } from "../fleet/env";
import { shortSerial } from "../core/usb-ids";
import type { HubDeviceRecord, LinkDeviceRecord } from "../core/protocol"; // -- WP UWN --
import { shortTime } from "../screens/hub/model"; // -- WP UWN --
import { peakDb } from "../core/protocol";
import type { Session } from "./session";
import { aboutData, linkText } from "./texts";
// -- WP WB6a --
import { Screens } from "./screens";
import { meterHidden, meterNames } from "../screens/main/model";
import { SenderScreen } from "../screens/device/DeviceScreen"; // -- WP PGWEB --
// -- WP WEBM1 -- the Modern shell
import type { ComponentChildren } from "preact";
import { Frame } from "./shell/Frame";
import { CONNECT, sectionsFor, type Page, type SectionId } from "./shell/sections";
import type { ActionBoard } from "./menus/board"; // -- WP WEBM4 --
import { EmptyState, Glyph } from "./design"; // -- WP WEB --
import { NoDeviceEmpty, NoDeviceFrame, NoDeviceStrip } from "./shell/NoDevice"; // -- WP REVEFIX --

export function useSessionState(s: Session) {
  const [, bump] = useState(0);
  const rendered = s.state;
  useEffect(() => {
    const stop = s.watch(() => bump((n) => n + 1));
    // -- WP WB4a -- The effect runs after paint: a change between this render and
    // the subscription (an unplug just after a tab switch) must not be lost.
    if (s.state !== rendered) bump((n) => n + 1);
    return stop;
  }, [s]);
  return s.state;
}

// ---------------------------------------------------------------- picker

export function DevicePicker(props: {
  installations: Installation[];
  env: PageEnv;
  openKeys: Set<string>;
  onOpen: (inst: Installation, source: Source) => void;
  onAddHub: ((url: string, name: string) => Promise<void>) | null;
  addHubError: string;
  /** -- WP WB4b -- the fleet's list of a hub's DSPis, when it has one. */
  hubDevices?: ((url: string) => Promise<{ deviceId: string; name: string }[] | null>) | null;
}) {
  return (
    <nav class="picker" aria-label="DSPi devices">
      {props.installations.map((inst) => (
        <section key={inst.id} class="installation" aria-label={inst.name}>
          <h2>{inst.name}</h2>
          <ul>
            {inst.sources.map((src) =>
              // -- WP WB4a --
              src.kind === "usb" && !src.serial && props.env.usb.ok ? (
                <UsbSource
                  key="usb"
                  env={props.env}
                  isOpen={(s) => props.openKeys.has(`${inst.id}/${sourceKey(s)}`)}
                  onOpen={(s) => props.onOpen(inst, s)}
                />
              ) : src.kind === "hub" && !src.deviceId && props.hubDevices && props.env.link.ok ? (
                // -- WP WB4b --
                <HubDevices
                  key={sourceKey(src)}
                  url={src.url}
                  list={props.hubDevices}
                  env={props.env}
                  isOpen={(s) => props.openKeys.has(`${inst.id}/${sourceKey(s)}`)}
                  onOpen={(s) => props.onOpen(inst, s)}
                />
              ) : (
                <SourceRow
                  key={sourceKey(src)}
                  src={src}
                  env={props.env}
                  // -- WP WB4b -- a hub's own list: open as many sessions as wanted
                  open={!(src.kind === "hub" && !src.deviceId) && props.openKeys.has(`${inst.id}/${sourceKey(src)}`)}
                  onOpen={() => props.onOpen(inst, src)}
                />
              ),
            )}
          </ul>
          {inst.editable && props.onAddHub ? <AddHub onAdd={props.onAddHub} error={props.addHubError} /> : null}
        </section>
      ))}
    </nav>
  );
}

function SourceRow(props: { src: Source; env: PageEnv; open: boolean; onOpen: () => void }) {
  const { src, env } = props;
  const label =
    src.kind === "demo"
      ? "Demo DSPi"
      : src.kind === "usb"
        ? src.serial
          ? `DSPi on USB · ${shortSerial(src.serial)}`
          : "DSPi on USB"
        : src.kind === "document"
          ? `File ${src.fileName}` // -- WP P1WB1 -- (never listed by a backend)
          : src.kind === "hub" && src.deviceId && src.online !== undefined
            ? `${src.name || src.deviceId} · ${src.online ? "online" : "offline"}` // -- WP P2WB1 -- the serving hub's inventory
          : src.deviceId
            ? `${src.name || src.deviceId} · hub ${hostOf(src.url)}`
            : `Hub ${hostOf(src.url)}`;
  const avail = src.kind === "demo" || src.kind === "document" ? { ok: true, reason: "" } : src.kind === "usb" ? env.usb : env.link;
  // -- WP P2WB1 -- an offline DSPi of the hub's inventory: listed, not offered
  const offline = src.kind === "hub" && src.online === false;
  return (
    <li class="source">
      <span class="source-name" title={src.kind === "usb" && src.serial ? src.serial : undefined}>
        {label}
      </span>
      {offline ? (
        <span class="unavailable">Not attached to the hub</span>
      ) : avail.ok ? (
        // -- WP UWN -- (UXP A15) a name that says which: "Connect Demo DSPi", not three buttons named "Connect"
        <button type="button" disabled={props.open} aria-label={props.open ? `${label} is open` : `Connect ${label}`} onClick={props.onOpen}>
          {props.open ? "Open" : "Connect"}
        </button>
      ) : (
        <span class="unavailable">{avail.reason}</span>
      )}
    </li>
  );
}

// -- WP WB4a --
/**
 * USB: Connect opens the browser's own picker (it needs the click); the DSPis
 * this page may already open are listed as entries of their own, kept current
 * as they are plugged in and out. Each opens its own session.
 */
function UsbSource(props: { env: PageEnv; isOpen: (s: Source) => boolean; onOpen: (s: Source) => void }) {
  const access = props.env.usbDevices;
  const [permitted, setPermitted] = useState<UsbChoice[]>([]);
  const [error, setError] = useState("");
  useEffect(() => {
    if (!access) return;
    let alive = true;
    const refresh = () =>
      void access.permitted().then(
        (l) => alive && setPermitted(l),
        () => alive && setPermitted([]),
      );
    refresh();
    const stop = access.watch(refresh);
    return () => {
      alive = false;
      stop();
    };
  }, [access]);
  const choose = async () => {
    if (!access) return;
    try {
      const chosen = await access.choose();
      setError("");
      if (!chosen) return;
      setPermitted(await access.permitted());
      props.onOpen({ kind: "usb", serial: chosen.serial });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  return (
    <>
      <li class="source source-usb">
        <span class="source-name">DSPi on USB</span>
        <button type="button" aria-label="Connect a DSPi on USB" onClick={() => void choose()}>
          Connect
        </button>
        {error ? (
          <p class="error" role="alert">
            {error}
          </p>
        ) : null}
      </li>
      {permitted.map((d) => {
        const src: Source = { kind: "usb", serial: d.serial };
        return (
          <SourceRow key={sourceKey(src)} src={src} env={props.env} open={props.isOpen(src)} onOpen={() => props.onOpen(src)} />
        );
      })}
    </>
  );
}

// -- WP WB4b --
/**
 * A hub whose DSPis the fleet backend lists (`hubDevices`): one entry each. When
 * it has no list for this hub (null), the hub's own entry, which asks the hub.
 */
function HubDevices(props: {
  url: string;
  list: (url: string) => Promise<{ deviceId: string; name: string }[] | null>;
  env: PageEnv;
  isOpen: (s: Source) => boolean;
  onOpen: (s: Source) => void;
}) {
  const [devices, setDevices] = useState<{ deviceId: string; name: string }[] | null | undefined>(undefined);
  const [error, setError] = useState("");
  useEffect(() => {
    let alive = true;
    props.list(props.url).then(
      (l) => alive && setDevices(l),
      (e: unknown) => alive && setError(e instanceof Error ? e.message : String(e)),
    );
    return () => {
      alive = false;
    };
  }, [props.url, props.list]);
  if (error)
    return (
      <li class="source">
        <span class="source-name">Hub {hostOf(props.url)}</span>
        <span class="unavailable">{error}</span>
      </li>
    );
  if (devices === null) {
    const src: Source = { kind: "hub", url: props.url };
    return <SourceRow src={src} env={props.env} open={false} onOpen={() => props.onOpen(src)} />;
  }
  return (
    <>
      {(devices ?? []).map((d) => {
        const src: Source = { kind: "hub", url: props.url, deviceId: d.deviceId, name: d.name };
        return <SourceRow key={sourceKey(src)} src={src} env={props.env} open={props.isOpen(src)} onOpen={() => props.onOpen(src)} />;
      })}
    </>
  );
}

function AddHub(props: { onAdd: (url: string, name: string) => Promise<void>; error: string }) {
  const [url, setUrl] = useState("");
  return (
    <form
      class="add-hub"
      onSubmit={(e) => {
        e.preventDefault();
        if (url.trim()) void props.onAdd(url, "").then(() => setUrl(""));
      }}
    >
      <label>
        Add a hub
        <input
          name="hub"
          placeholder="hub-name.local:11915"
          value={url}
          onInput={(e) => setUrl((e.target as HTMLInputElement).value)}
        />
      </label>
      <button type="submit">Add</button>
      {props.error ? <p class="error" role="alert">{props.error}</p> : null}
    </form>
  );
}

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

// ---------------------------------------------------------------- one session

/**
 * -- WP WEB2 -- (REVD W13) What a session is called before its DSPi answered: "Connecting" only while it
 * is still trying; a session that failed says so, as its notice does (a hub that does not answer).
 */
export function sessionName(st: { snapshot: { device: { name: string } } | null; status: string }): string {
  if (st.snapshot) return st.snapshot.device.name;
  if (st.status === "failed") return "Not connected";
  if (st.status === "closed") return "Closed";
  return "Connecting";
}

export function SessionView(props: { session: Session; onClose: () => void; shell: ShellProps }) {
  const s = props.session;
  const st = useSessionState(s);
  const snap = st.snapshot;
  const ro = s.readOnlyReason;
  // -- WP WEBM1 -- the parts of the shell's frame (ui/shell/Frame.tsx) that are the same whatever the session shows
  // -- WP REVEFIX -- (REVE §3.1) no bar of its own: the device's name, firmware and state are its tab's (Qt's
  // TopBar), Disconnect is the tab's ×, read only is said in the strip; the demo says so there, as Qt's strip does
  const strip = (
    <>
      {props.shell.note}
      {s.source.kind === "demo" && snap ? (
        <div class="banner banner-info demo-strip" role="note">
          <Glyph name="info" class="banner-icon" />
          <span>This is the demo DSPi, a simulated device: nothing you change here reaches a real DSPi.</span>
        </div>
      ) : null}
      {st.link && st.link.phase !== "bound" ? <LostLinkBanner phase={st.link} onClose={props.onClose} /> : null}
      {ro ? <p class="note read-only-reason">{ro}</p> : null}
      {st.error ? (
        // -- WP P1WB2 -- with its notice row (P1NT) when the core's table has one: title, reason, what to do
        st.errorNotice ? (
          <p class={`error notice notice-${st.errorNotice.kind}`} role="alert" data-key={st.errorNotice.key}>
            <strong>{st.errorNotice.title}.</strong> {st.errorNotice.reason} <span class="notice-detail">{st.error}</span>{" "}
            <span class="notice-action">{st.errorNotice.action}</span>
          </p>
        ) : (
          <p class="error" role="alert">
            {st.error}
          </p>
        )
      ) : null}
    </>
  );
  const open = snap && st.status === "open" ? snap : null;
  // -- WP PGWEB -- the alink sender: its own page, no DSP screen and no meters
  const sender = open !== null && st.protocol?.deviceKind === "alinkSender";
  return (
    // -- WP REVEFIX -- which DSPi (or file) this is, now that its name and firmware are its tab's
    <article class="session" aria-label={snap?.device.name ?? "DSPi"} data-status={st.status} data-serial={(snap && !st.document && snap.device.serial) || undefined} data-file={st.document ? st.document.fileName || st.document.name : undefined}>
      {open && !sender ? (
        // -- WP WB6a -- the session's screens (WB6 slices); the skeleton's volume is the main window's now
        <Screens session={s} shell={props.shell} strip={strip} meters={<Meters session={s} names={meterNames(open.state, open.outputLabels)} hidden={meterHidden(open.state, open.outputLabels)} />} />
      ) : sender ? (
        <BareFrame shell={props.shell} strip={strip}>
          <SenderScreen session={s} />
        </BareFrame>
      ) : (
        // -- WP REVEFIX -- (REVD D12) not open (yet, or any more): the whole rail and an empty state on every page, as Qt
        <NoDeviceFrame
          key={st.status === "failed" || st.status === "closed" ? "none" : "on-the-way" /* a fresh frame once it has failed */}
          shell={props.shell}
          connecting={st.status === "opening" || st.status === "pairing" || st.status === "choosing"}
          strip={
            <>
              {strip}
              {st.status === "failed" || st.status === "closed" ? <NoDeviceStrip openDemo={props.shell.openDemo} /> : null}
            </>
          }
          empty={(overview) =>
            // -- WP WB4b -- Link's steps before a DSPi is open
            st.status === "pairing" ? (
              <PinEntry session={s} error={st.pinError} />
            ) : st.status === "choosing" ? (
              <HubChooser session={s} />
            ) : st.status === "opening" ? (
              <EmptyState icon="unplug" title="Connecting…" text="Waiting for the DSPi to answer." />
            ) : (
              <NoDeviceEmpty overview={overview} hub={s.source.kind === "hub"} onChoose={() => props.shell.onPicking(true)} />
            )
          }
        />
      )}
    </article>
  );
}

// -- WP WEBM1 --
/** What the page's shell gives a session's view. */
export interface ShellProps {
  /** The devices' page is on show in place of the session's. */
  picking: boolean;
  onPicking: (on: boolean) => void;
  /** That page. */
  connect: ComponentChildren;
  /** The shortcut list is open (the app menu opens it too). */
  shortcuts: boolean;
  onShortcuts: (on: boolean) => void;
  // -- WP WEBM4 --
  /** Where the device's view says what it can do now (the app menu's and the command palette's rows). */
  board?: ActionBoard;
  // -- WP REVEFIX --
  /** The top bar (Qt's TopBar), with the device's own parts beside its tab and before the bell. */
  top: (session?: { left: ComponentChildren; right: ComponentChildren }) => ComponentChildren;
  /** What the page has to say (another tab has this DSPi already), in the strip. */
  note?: ComponentChildren;
  /** Opens the demo, where the page has one (the no-device strip's Open Demo). */
  openDemo: (() => void) | null;
}

const NO_PAGES: ReadonlyMap<SectionId, string> = new Map();
const OVERVIEW_PAGES: Page[] = [{ id: "sender", label: "Sender" }];

/** The frame of an alink sender: its page, and the devices' page. */
function BareFrame(props: { shell: ShellProps; strip: ComponentChildren; children: ComponentChildren }) {
  const { shell } = props;
  const sections = sectionsFor(OVERVIEW_PAGES);
  const page = shell.picking ? CONNECT : "sender";
  return (
    <Frame bar={shell.top()} strip={props.strip} sections={sections} page={page} last={NO_PAGES} onPage={(id) => shell.onPicking(id === CONNECT)}>
      {shell.picking ? shell.connect : props.children}
    </Frame>
  );
}

// -- WP WB4b --
/** The hub's PIN; the banner above says where to find it (CG10's needs_pairing wording). */
function PinEntry(props: { session: Session; error: string }) {
  const [pin, setPin] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <form
      class="pin-entry"
      onSubmit={(e) => {
        e.preventDefault();
        if (!pin.trim() || busy) return;
        setBusy(true);
        void props.session.pair(pin.trim()).finally(() => {
          setBusy(false);
          setPin("");
        });
      }}
    >
      <label>
        PIN
        <input
          name="pin"
          inputMode="numeric"
          autoComplete="one-time-code"
          value={pin}
          onInput={(e) => setPin((e.target as HTMLInputElement).value)}
        />
      </label>
      <button type="submit" disabled={busy}>
        Pair
      </button>
      {props.error ? (
        <p class="error" role="alert">
          {props.error}
        </p>
      ) : null}
    </form>
  );
}

/**
 * The hub's DSPis (Link's choose_device step): open one in this session. -- WP UWN -- (UXP A15) rows you can tell
 * apart: the name, then the hub's location for it, its state and the distinguishing part of its serial whole; the
 * DSPis the hub remembers but does not have now greyed, with when it last saw them (its inventory, when it has one).
 */
function HubChooser(props: { session: Session }) {
  const st = useSessionState(props.session);
  const [inventory, setInventory] = useState<HubDeviceRecord[]>([]);
  useEffect(() => {
    let alive = true;
    // An upstream hub has no inventory: the Link list alone
    void props.session.core.call("hubInventory", undefined).then(
      (l) => alive && Array.isArray(l) && setInventory(l),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [props.session, st.devices.length]);
  const rows = chooserRows(st.devices, inventory);
  return (
    <ul class="hub-devices" aria-label={st.hubName ? `DSPis on ${st.hubName}` : "DSPis on the hub"}>
      {rows.map((r) => (
        <li key={r.serial} class={r.online ? "source" : "source source-offline"}>
          <span class="source-text">
            <span class="source-name" title={r.serial}>
              {r.name}
            </span>
            <span class="source-meta">{r.meta}</span>
          </span>
          {r.online ? (
            <button type="button" aria-label={`Open ${r.spoken}`} onClick={() => void props.session.choose(r.serial)}>
              Open
            </button>
          ) : null}
        </li>
      ))}
    </ul>
  );
}

// -- WP UWN -- (UXP A15)
/** The part of a serial that tells units apart, whole: its last two "-" parts ("ALINK-A"), else its last 8 characters. */
export function serialPart(serial: string): string {
  const parts = serial.split("-").filter(Boolean);
  return parts.length > 1 ? parts.slice(-2).join("-") : shortSerial(serial);
}

/** One row of the hub chooser: the name, what tells it apart (`meta`), and the whole for a screen reader. */
export interface ChooserRow {
  serial: string;
  name: string;
  online: boolean;
  meta: string;
  spoken: string;
}

/** The chooser's rows: the hub's Link list with its inventory's locations, then the remembered DSPis it lacks now. */
export function chooserRows(devices: readonly LinkDeviceRecord[], inventory: readonly HubDeviceRecord[]): ChooserRow[] {
  const row = (serial: string, name: string, state: string, location: string, lastSeen: string): ChooserRow => {
    const online = state === "online";
    const words = online ? "online" : state === "offline" || !state ? `offline${lastSeen ? `, last seen ${shortTime(lastSeen)}` : ""}` : state;
    const meta = [location, words, serialPart(serial)].filter(Boolean).join(" · ");
    const shown = name || "DSPi";
    return { serial, name: shown, online, meta, spoken: [shown, location, serialPart(serial)].filter(Boolean).join(", ") };
  };
  const inv = new Map(inventory.map((d) => [d.serial, d]));
  const out = devices.map((d) => row(d.serial, d.name, d.state, inv.get(d.serial)?.location ?? "", inv.get(d.serial)?.lastSeen ?? ""));
  for (const d of inventory) if (!devices.some((x) => x.serial === d.serial) && !d.online) out.push(row(d.serial, d.name, "offline", d.location, d.lastSeen));
  return out;
}

/** CG10's wording; a lost link always offers the way out (owner's rule). */
function LostLinkBanner(props: { phase: { phase: Parameters<typeof linkText>[0]; pinSource?: string }; onClose: () => void }) {
  const t = linkText(props.phase.phase, props.phase.pinSource);
  return (
    <div class={`banner banner-${t.phase}`} role="status" data-key={t.key}>
      <span>{t.text}</span>
      {t.action ? (
        <button type="button" data-key={t.action_key ?? ""} onClick={props.onClose}>
          {t.action}
        </button>
      ) : null}
    </div>
  );
}

/** Polls only while the page is visible: nothing runs for a hidden tab. */
function Meters(props: { session: Session; names: string[]; hidden?: boolean[] }) {
  const [levels, setLevels] = useState<number[]>([]);
  // -- WP P1WB2 -- (S8) not a fresh reading (Link): since when, greyed
  const [staleSince, setStaleSince] = useState<number | null>(null);
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      if (!alive) return;
      if (document.visibilityState === "visible" && props.session.state.status === "open") {
        try {
          const m = await props.session.core.call("meters", undefined);
          setLevels(m.peaks.map(peakDb));
          setStaleSince(m.stale ? Date.now() - (m.ageMs ?? 0) : null);
        } catch {
          // A failed read leaves the last levels; the strip shows the state.
        }
      }
      if (alive) timer = setTimeout(tick, 100);
    };
    void tick();
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
    };
  }, [props.session]);
  return (
    <>
    {staleSince !== null ? (
      <p class="note meters-stale" role="status" aria-label="Meters stale">
        Stale since {staleText(staleSince)}: no fresh reading from the DSPi.
      </p>
    ) : null}
    <ul class={staleSince !== null ? "meters stale" : "meters"} aria-label="Levels">
      {props.names.map((n, i) => {
        if (props.hidden?.[i]) return null; // -- WP INTE -- an unused output has no meter (owner 2026-11-24)
        const db = levels[i] ?? -90;
        const pct = Math.max(0, Math.min(100, ((db + 90) / 90) * 100));
        return (
          <li key={`${i}-${n}`} class="meter" title={n /* -- WP WEBM1 -- the dock has no room for the names */}>
            <span class="meter-name">{n}</span>
            <span class="meter-bar" role="meter" aria-valuemin={-90} aria-valuemax={0} aria-valuenow={Math.round(db)} aria-label={n}>
              <span style={{ width: `${pct}%`, "--level": `${pct}%` /* -- WP WEBM1 -- upright in the dock */ }} />
            </span>
          </li>
        );
      })}
    </ul>
    </>
  );
}

// -- WP P1WB2 --
/** When the meters' last fresh reading was: the time of day, and how long ago. */
export function staleText(since: number, now = Date.now()): string {
  const d = new Date(since);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  const ss = String(d.getSeconds()).padStart(2, "0");
  return `${hh}:${mm}:${ss} (${Math.max(0, Math.round((now - since) / 1000))} s ago)`;
}

// ---------------------------------------------------------------- About

export function About(props: { onClose: () => void; sourceUrl: string | null }) {
  const a = aboutData;
  return (
    <dialog open class="about" aria-label="About">
      <h2>
        About uConsole for DSPi {a.draft ? <span class="badge">Draft</span> : null}
      </h2>
      {a.credit_lines.map((l) => (
        <p key={l.id}>{l.text}</p>
      ))}
      <h3>Who made what</h3>
      <ul>
        {a.makers.map((m) => (
          <li key={m.id}>
            <a href={m.url} target="_blank" rel="noopener noreferrer">
              {m.name}
            </a>
            : {m.made.join(", ")}
          </li>
        ))}
      </ul>
      <h3>{a.links_title}</h3>
      <p class="note">{a.links_note}</p>
      <ul>
        {a.links.map((l) => (
          <li key={l.id}>
            <a href={l.url} target="_blank" rel="noopener noreferrer">
              {l.title}
            </a>{" "}
            <span class="note">{l.detail}</span>
          </li>
        ))}
      </ul>
      <h3>Credits</h3>
      <ul>
        {a.credits.map((c) => (
          <li key={c.id}>
            {c.url ? (
              <a href={c.url} target="_blank" rel="noopener noreferrer">
                {c.name}
              </a>
            ) : (
              c.name
            )}
            : {c.what}
          </li>
        ))}
      </ul>
      {/* -- WP LIC1 -- the project's licence and its section 7 terms, as the table words them */}
      <h3>{a.licence.title}</h3>
      {a.licence.copyright.map((c) => (
        <p key={c} class="note">
          {c}
        </p>
      ))}
      <p class="note">{a.licence.statement}</p>
      <h4>{a.licence.terms_title}</h4>
      <p class="note">{a.licence.terms_note}</p>
      <ol class="terms">
        {a.licence.terms.map((t) => (
          <li key={t.id}>
            <strong>{t.title}.</strong> {t.text}
          </li>
        ))}
      </ol>
      <p class="note">{a.licence.freedoms}</p>
      <p class="note">
        {a.licence.notices}{" "}
        <a href="THIRD-PARTY-NOTICES.txt" target="_blank" rel="noopener noreferrer">
          Third-party notices
        </a>
        {props.sourceUrl ? (
          <>
            {" · "}
            <a href={props.sourceUrl} target="_blank" rel="noopener noreferrer">
              Source code
            </a>
          </>
        ) : null}
      </p>
      <button type="button" onClick={props.onClose}>
        Close
      </button>
    </dialog>
  );
}
