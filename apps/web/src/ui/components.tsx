// The skeleton's UI (WP WB5): device picker, connection strip, lost-link banner,
// a volume control with read-back, meters, and About. Later WPs (WB6 slices) add
// screens beside these; they share the session model and never reach the core
// except through it.

import { useEffect, useState } from "preact/hooks";
import type { Installation, Source } from "../fleet/types";
import { sourceKey } from "../fleet/types";
import type { PageEnv, UsbChoice } from "../fleet/env";
import { shortSerial } from "../core/usb-ids";
import { peakDb } from "../core/protocol";
import type { Session } from "./session";
import { aboutData, linkText } from "./texts";

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
        : src.deviceId
          ? `${src.name || src.deviceId} · hub ${hostOf(src.url)}`
          : `Hub ${hostOf(src.url)}`;
  const avail = src.kind === "demo" ? { ok: true, reason: "" } : src.kind === "usb" ? env.usb : env.link;
  return (
    <li class="source">
      <span class="source-name" title={src.kind === "usb" && src.serial ? src.serial : undefined}>
        {label}
      </span>
      {avail.ok ? (
        <button type="button" disabled={props.open} onClick={props.onOpen}>
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
        <button type="button" onClick={() => void choose()}>
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

/** A tab's name: where the device is, then what it is called once it answers. */
export function TabLabel(props: { session: Session }) {
  const st = useSessionState(props.session);
  return (
    <>
      {props.session.installationName} · {st.snapshot?.device.name ?? "Connecting"}
    </>
  );
}

export function SessionView(props: { session: Session; onClose: () => void }) {
  const s = props.session;
  const st = useSessionState(s);
  const snap = st.snapshot;
  const ro = s.readOnlyReason;
  return (
    <article class="session" aria-label={snap?.device.name ?? "DSPi"} data-status={st.status}>
      <header class="strip">
        <span class={`dot dot-${st.status}`} aria-hidden="true" />
        <strong class="device-name">{snap?.device.name ?? "Connecting"}</strong>
        {snap ? (
          <span class="device-meta">
            {snap.device.platform} · firmware {snap.device.firmware}
            {snap.device.serial ? ` · ${snap.device.serial}` : ""}
          </span>
        ) : null}
        {ro ? (
          <span class="badge read-only" title={ro}>
            Read only
          </span>
        ) : null}
        <button type="button" class="close" onClick={props.onClose}>
          Disconnect
        </button>
      </header>
      {st.link && st.link.phase !== "bound" ? <LostLinkBanner phase={st.link} onClose={props.onClose} /> : null}
      {/* -- WP WB4b -- Link's steps before a DSPi is open */}
      {st.status === "pairing" ? <PinEntry session={s} error={st.pinError} /> : null}
      {st.status === "choosing" ? <HubChooser session={s} /> : null}
      {ro ? <p class="note read-only-reason">{ro}</p> : null}
      {st.error ? (
        <p class="error" role="alert">
          {st.error}
        </p>
      ) : null}
      {snap && st.status === "open" ? (
        <div class="controls">
          <Volume session={s} disabled={!!ro} />
          <Meters session={s} names={snap.state.channelNames.slice(0, snap.state.numChannels)} />
        </div>
      ) : null}
    </article>
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

/** The hub's DSPis (Link's choose_device step): open one in this session. */
function HubChooser(props: { session: Session }) {
  const st = useSessionState(props.session);
  return (
    <ul class="hub-devices" aria-label={st.hubName ? `DSPis on ${st.hubName}` : "DSPis on the hub"}>
      {st.devices.map((d) => {
        const online = d.state === "online";
        return (
          <li key={d.serial} class="source">
            <span class="source-name" title={d.serial}>
              {d.name || "DSPi"} · {shortSerial(d.serial)}
            </span>
            {online ? (
              <button type="button" onClick={() => void props.session.choose(d.serial)}>
                Open
              </button>
            ) : (
              <span class="unavailable">{d.state}</span>
            )}
          </li>
        );
      })}
    </ul>
  );
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

function Volume(props: { session: Session; disabled: boolean }) {
  const { snapshot, params } = props.session.state;
  const db = snapshot!.state.userVolumeDb;
  const range = params["vol.user"];
  const [draft, setDraft] = useState<number | null>(null);
  const shown = draft ?? db;
  return (
    <div class="volume">
      <label>
        Volume
        <input
          type="range"
          min={range?.min ?? -60}
          max={range?.max ?? 0}
          step={0.5}
          value={shown}
          disabled={props.disabled}
          onInput={(e) => setDraft(Number((e.target as HTMLInputElement).value))}
          onChange={(e) => {
            const v = Number((e.target as HTMLInputElement).value);
            void props.session.write("vol.user", { float: v }).then(() => setDraft(null));
          }}
        />
      </label>
      <output class="volume-value" aria-live="polite">
        {db.toFixed(1)} dB
      </output>
    </div>
  );
}

/** Polls only while the page is visible: nothing runs for a hidden tab. */
function Meters(props: { session: Session; names: string[] }) {
  const [levels, setLevels] = useState<number[]>([]);
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      if (!alive) return;
      if (document.visibilityState === "visible" && props.session.state.status === "open") {
        try {
          setLevels((await props.session.core.call("meters", undefined)).peaks.map(peakDb));
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
    <ul class="meters" aria-label="Levels">
      {props.names.map((n, i) => {
        const db = levels[i] ?? -90;
        const pct = Math.max(0, Math.min(100, ((db + 90) / 90) * 100));
        return (
          <li key={`${i}-${n}`} class="meter">
            <span class="meter-name">{n}</span>
            <span class="meter-bar" role="meter" aria-valuemin={-90} aria-valuemax={0} aria-valuenow={Math.round(db)} aria-label={n}>
              <span style={{ width: `${pct}%` }} />
            </span>
          </li>
        );
      })}
    </ul>
  );
}

// ---------------------------------------------------------------- About

export function About(props: { onClose: () => void; sourceUrl: string | null }) {
  const a = aboutData;
  return (
    <dialog open class="about" aria-label="About">
      <h2>
        About DSPi Console {a.draft ? <span class="badge">Draft</span> : null}
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
      <p class="note">
        Licence: GNU GPL, version 3 or later.{" "}
        {props.sourceUrl ? (
          <a href={props.sourceUrl} target="_blank" rel="noopener noreferrer">
            Source code
          </a>
        ) : null}
      </p>
      <button type="button" onClick={props.onClose}>
        Close
      </button>
    </dialog>
  );
}
