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
import { About, DevicePicker, SessionView, TabLabel } from "./components";
import { Session } from "./session";
// -- WP WB6b --
import { UnsavedWatch } from "./unsaved";

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
}

export function App(props: { options: ConsoleOptions }) {
  const { backend, spawnCore } = props.options;
  const env = useMemo(() => props.options.env ?? pageEnv(), [props.options.env]);
  const [installations, setInstallations] = useState<Installation[]>([]);
  const [user, setUser] = useState<FleetUser | null>(null);
  const [sessions, setSessions] = useState<Session[]>([]);
  const live = useRef<Session[]>([]);
  live.current = sessions;
  const [active, setActive] = useState<string | null>(null);
  const [aboutOpen, setAboutOpen] = useState(false);
  const [listError, setListError] = useState("");
  const [addHubError, setAddHubError] = useState("");
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

  const open = async (inst: Installation, source: Source) => {
    // -- WP WB4b -- a hub without a named DSPi: a new session each time, which
    // asks the hub for its list (one hub, several DSPis open at once)
    const chooser = source.kind === "hub" && !source.deviceId;
    const id = `${inst.id}/${sourceKey(source)}${chooser ? `~${++hubSessions.current}` : ""}`;
    if (openKeys.has(id)) {
      setActive(id);
      return;
    }
    // -- WP WB4a -- a USB DSPi is its serial
    const deviceId =
      source.kind === "hub" ? (source.deviceId ?? "") : source.kind === "usb" && source.serial ? source.serial : source.kind;
    const access = props.options.readOnly ? "view" : await backend.access(inst.id, source, deviceId);
    const session = new Session(id, inst.id, inst.name, source, access, spawnCore(), source.kind === "hub" ? tokens : null);
    setSessions((list) => [...list, session]);
    setActive(id);
    await session.open(targetFor(source));
  };

  const close = async (s: Session) => {
    setSessions((list) => list.filter((x) => x !== s));
    setActive((cur) => (cur === s.id ? null : cur));
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

  return (
    <div class="dspi-web">
      <header class="top">
        <h1>{props.options.title}</h1>
        {user ? <span class="user">{user.name}</span> : null}
        {props.options.hubPage ? (
          <a class="hub-page" href={props.options.hubPage}>
            Hub page
          </a>
        ) : null}
        <button type="button" class="about-button" onClick={() => setAboutOpen(true)}>
          About
        </button>
      </header>
      <div class="body">
        {listError ? (
          <p class="error" role="alert">
            {listError}
          </p>
        ) : null}
        <DevicePicker
          installations={installations}
          env={env}
          openKeys={openKeys}
          onOpen={(i, s) => void open(i, s)}
          onAddHub={addHub}
          addHubError={addHubError}
          hubDevices={backend.hubDevices ? (url) => backend.hubDevices!(url) : null}
        />
        <main class="sessions">
          {sessions.length > 1 ? (
            <div class="tabs" role="tablist" aria-label="Open DSPi">
              {sessions.map((s) => (
                <button
                  type="button"
                  role="tab"
                  key={s.id}
                  aria-selected={s === current}
                  onClick={() => setActive(s.id)}
                >
                  <TabLabel session={s} />
                </button>
              ))}
            </div>
          ) : null}
          {current ? (
            <SessionView key={current.id} session={current} onClose={() => void close(current)} />
          ) : (
            <p class="empty">Choose a DSPi to connect to.</p>
          )}
        </main>
      </div>
      {aboutOpen ? <About sourceUrl={props.options.sourceUrl} onClose={() => setAboutOpen(false)} /> : null}
    </div>
  );
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
  }
}
