// The console's root (WP WB5). Everything it knows about the page comes in
// through `ConsoleOptions`, so the same component runs standalone and inside a
// fleet server's page, any number of times on one page.

import { useCallback, useEffect, useMemo, useRef, useState } from "preact/hooks";
import { CoreClient } from "../core/client";
import type { DeviceTarget } from "../core/protocol";
import { pageEnv, type PageEnv } from "../fleet/env";
// -- WP WB4b --
import { hubTokensFor } from "../fleet/hub-tokens";
import { safeLocalStorage } from "../fleet/local";
import { sourceKey, type FleetBackend, type FleetUser, type Installation, type Source } from "../fleet/types";
import { About, DevicePicker, SessionView, type ShellProps } from "./components";
import { Session } from "./session";
// -- WP WEBM1 -- the Modern shell
import { alreadyOpenText, holderOf, serialOf } from "./shell/open-rule";
import { Rail } from "./shell/Rail";
import { sectionsFor, type Navigation } from "./shell/sections";
import { TopBar } from "./shell/TopBar";
// -- WP WB6b --
import { UnsavedWatch } from "./unsaved";
// -- WP P1WB1 -- offline files: Open File…, New from Template…
import { OpenFiles } from "../screens/snapshots/OpenFile";

export interface ConsoleOptions {
  backend: FleetBackend;
  /** Opens a core session. Default: a module Worker from `workerUrl`. */
  spawnCore: () => CoreClient;
  /** Heading. A fleet server may name its product here. */
  title: string;
  /** Link to this build's source (GPL). Shown in About when set. */
  sourceUrl: string | null;
  /** -- WP WB7 -- The serving hub's own page, linked from the header. */
  hubPage?: string | null;
  /** Every session read only, whatever the backend allows (a display page). */
  readOnly: boolean;
  /** What this page can reach; tests override it. */
  env?: PageEnv;
  // -- WP WB6b --
  /** Follows what is unsaved in the open sessions (and holds the leave prompt); `mount` makes one. */
  unsaved?: UnsavedWatch;
  // -- WP WEBM1 --
  /** Where the sections are (default: a rail). */
  navigation?: Navigation;
}

export function App(props: { options: ConsoleOptions }) {
  const { backend, spawnCore } = props.options;
  const env = useMemo(() => props.options.env ?? pageEnv(), [props.options.env]);
  const [installations, setInstallations] = useState<Installation[]>([]);
  const [user, setUser] = useState<FleetUser | null>(null);
  const [sessions, setSessions] = useState<Session[]>([]);
  // -- WP WEBM1 -- the open sessions as they are now; `sessions` follows at the next render
  const live = useRef<Session[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [aboutOpen, setAboutOpen] = useState(false);
  const [listError, setListError] = useState("");
  const [addHubError, setAddHubError] = useState("");
  // -- WP WEBM1 -- the devices' page in place of the session's; what the page has to say; a tab being closed; the shortcut list
  const [picking, setPicking] = useState(false);
  const [said, setSaid] = useState("");
  const [closing, setClosing] = useState<Session | null>(null);
  const [shortcuts, setShortcuts] = useState(false);
  // -- WP WB4b -- tokens per hub: the fleet's, or this page's storage
  const tokens = useMemo(() => hubTokensFor(backend, safeLocalStorage()), [backend]);
  const hubSessions = useRef(0);

  const reload = useCallback(async () => {
    try {
      setInstallations(await backend.installations());
      setListError("");
    } catch (e) {
      setListError(e instanceof Error ? e.message : String(e));
    }
  }, [backend]);

  useEffect(() => {
    void reload();
    void backend.user?.().then(setUser, () => setUser(null));
  }, [backend, reload]);

  const openKeys = useMemo(() => new Set(sessions.map((s) => s.id)), [sessions]);

  // -- WP WB6b -- what is unsaved, for the leave prompt and the host page
  const watch = useMemo(() => props.options.unsaved ?? new UnsavedWatch(true), [props.options.unsaved]);
  useEffect(() => watch.track(sessions), [watch, sessions]);
  useEffect(() => () => watch.track([]), [watch]);

  // -- WP WEBM1 --
  const show = (id: string) => {
    setActive(id);
    setPicking(false);
    setShortcuts(false);
    setSaid("");
  };

  /** A new session, in a tab of its own. `live` has it at once, so the next open sees it. */
  const add = (session: Session) => {
    live.current = [...live.current, session];
    setSessions(live.current);
    show(session.id);
  };

  /** Show the session that has this already (the same entry, or the same DSPi reached another way); whether one has. */
  const shownAlready = (id: string, source: Source): boolean => {
    if (live.current.some((s) => s.id === id)) {
      show(id);
      return true;
    }
    const holder = holderOf(live.current, serialOf(source));
    if (!holder) return false;
    show(holder.id);
    setSaid(alreadyOpenText(holder));
    return true;
  };

  const open = async (inst: Installation, source: Source) => {
    // -- WP WB4b -- a hub without a named DSPi: a new session each time, which
    // asks the hub for its list (one hub, several DSPis open at once)
    const chooser = source.kind === "hub" && !source.deviceId;
    const id = `${inst.id}/${sourceKey(source)}${chooser ? `~${++hubSessions.current}` : ""}`;
    if (shownAlready(id, source)) return;
    // -- WP WB4a -- a USB DSPi is its serial
    const deviceId =
      source.kind === "hub" ? (source.deviceId ?? "") : source.kind === "usb" && source.serial ? source.serial : source.kind;
    const access = props.options.readOnly ? "view" : await backend.access(inst.id, source, deviceId);
    // -- WP P1WB2 -- the fleet's pinned mode for this device, if it pins one
    const fleetMode = (await backend.pinnedMode?.(inst.id, source, deviceId).catch(() => null)) ?? "";
    // -- WP WEBM1 -- again: another open may have come first while the backend answered (nothing waits from here to `add`)
    if (shownAlready(id, source)) return;
    const session = new Session(id, inst.id, inst.name, source, access, spawnCore(), source.kind === "hub" ? tokens : null);
    session.fleetMode = fleetMode;
    session.refuseOpen = (serial) => {
      const holder = holderOf(live.current, serial, session);
      return holder ? alreadyOpenText(holder) : "";
    };
    // -- WP P2WB1 -- the hub screen's Open: another DSPi of the same hub, in a session of its own
    if (source.kind === "hub") session.openDevice = (serial) => void open(inst, { kind: "hub", url: source.url, deviceId: serial });
    add(session);
    await session.open(targetFor(source));
  };

  // -- WP P1WB1 -- a document opened with no device: a session of its own each time
  const documents = useRef(0);
  const openDocument = async (text: string, fileName: string, core?: CoreClient) => {
    const id = `files/document~${++documents.current}`;
    const source: Source = { kind: "document", fileName };
    const session = new Session(id, "files", "File", source, props.options.readOnly ? "view" : "control", core ?? spawnCore());
    add(session);
    await session.openDocument(text, fileName);
  };

  const close = async (s: Session) => {
    // -- WP WEBM1 -- the tab beside it is shown next
    const at = live.current.indexOf(s);
    const rest = live.current.filter((x) => x !== s);
    live.current = rest;
    setSessions(rest);
    setActive((cur) => (cur === s.id ? (rest[Math.min(Math.max(at, 0), rest.length - 1)]?.id ?? null) : cur));
    setShortcuts(false);
    await s.close();
  };

  // Close every session when the console goes away (unmount).
  useEffect(
    () => () => {
      for (const s of live.current) void s.close();
    },
    [],
  );

  const addHub = backend.addHub
    ? async (url: string, name: string) => {
        try {
          await backend.addHub!(null, url, name);
          setAddHubError("");
          await reload();
        } catch (e) {
          setAddHubError(e instanceof Error ? e.message : String(e));
        }
      }
    : null;

  const current = sessions.find((s) => s.id === active) ?? sessions[0] ?? null;

  // -- WP WEBM1 -- a tab's close button: its session is shown first, so its view can ask about unsaved changes
  useEffect(() => {
    if (!closing || current !== closing) return;
    setClosing(null);
    void closing.mayLeave().then((go) => (go ? close(closing) : undefined));
  }, [closing, current]);

  const connect = (
    <div class="devices-page">
      {current ? null : <p class="empty">Choose a DSPi to connect to.</p>}
      {listError ? (
        <p class="error" role="alert">
          {listError}
        </p>
      ) : null}
      {props.options.hubPage ? (
        <p>
          <a class="hub-page" href={props.options.hubPage}>
            Hub page
          </a>
        </p>
      ) : null}
      <div class="devices-columns">
        <DevicePicker
          installations={installations}
          env={env}
          openKeys={openKeys}
          onOpen={(i, s) => void open(i, s)}
          onAddHub={addHub}
          addHubError={addHubError}
          hubDevices={backend.hubDevices ? (url) => backend.hubDevices!(url) : null}
        />
        {/* -- WP P1WB1 -- */}
        {props.options.readOnly ? null : (
          <OpenFiles spawnCore={spawnCore} onOpen={(text, name, core) => void openDocument(text, name, core)} />
        )}
      </div>
    </div>
  );
  const onPicking = useCallback((on: boolean) => {
    setPicking(on);
    if (on) setSaid("");
  }, []);
  const shell: ShellProps = { picking, onPicking, connect, shortcuts, onShortcuts: setShortcuts };
  const status = useOpenStatus(current);

  return (
    <div class="dspi-web" data-nav={props.options.navigation ?? "rail"}>
      <TopBar
        title={props.options.title}
        user={user?.name ?? ""}
        sessions={sessions}
        current={current}
        picking={picking}
        onChoose={(s) => show(s.id)}
        onClose={(s) => {
          show(s.id);
          setClosing(s);
        }}
        onAdd={() => onPicking(true)}
        onShortcuts={status === "open" && current?.state.protocol?.deviceKind !== "alinkSender" ? () => setShortcuts(true) : null}
        onAbout={() => setAboutOpen(true)}
      />
      {said ? (
        <p class="shell-note" role="status">
          <span>{said}</span>
          <button type="button" class="dismiss" aria-label="Dismiss" onClick={() => setSaid("")}>
            ×
          </button>
        </p>
      ) : null}
      <main class="sessions">
        {current ? (
          <SessionView key={current.id} session={current} onClose={() => void close(current)} shell={shell} />
        ) : (
          <div class="shell">
            <div class="shell-body">
              <Rail sections={sectionsFor([])} current="devices" onSection={() => undefined} />
              <div class="shell-main">
                <div class="page-head">
                  <h2 class="page-title">Devices</h2>
                </div>
                <div class="page">{connect}</div>
              </div>
            </div>
          </div>
        )}
      </main>
      {aboutOpen ? <About sourceUrl={props.options.sourceUrl} onClose={() => setAboutOpen(false)} /> : null}
    </div>
  );
}

// -- WP WEBM1 --
/** The session's status as it changes, null for none. */
function useOpenStatus(session: Session | null): Session["state"]["status"] | null {
  const [, seen] = useState(session?.state.status ?? null);
  useEffect(() => {
    const follow = () => seen(session?.state.status ?? null);
    follow();
    return session?.watch(follow);
  }, [session]);
  return session?.state.status ?? null;
}

function targetFor(source: Source): DeviceTarget {
  switch (source.kind) {
    case "demo":
      return { kind: "demo" };
    case "usb":
      // -- WP WB4a --
      return source.serial ? { kind: "usb", serial: source.serial } : { kind: "usb" };
    case "hub":
      // -- WP WB4b --
      return { kind: "hub", url: source.url, deviceId: source.deviceId ?? "" };
    case "document":
      // -- WP P1WB1 -- opened by `openDocument`, never from the picker
      return { kind: "demo" };
  }
}
