// The skeleton's UI (WP WB5): device picker, connection strip, lost-link banner,
// a volume control with read-back, meters, and About. Later WPs (WB6 slices) add
// screens beside these; they share the session model and never reach the core
// except through it.

import { useEffect, useState } from "preact/hooks";
import type { Installation, Source } from "../fleet/types";
import { sourceKey } from "../fleet/types";
import type { PageEnv } from "../fleet/env";
import type { Session } from "./session";
import { aboutData, linkText } from "./texts";

export function useSessionState(s: Session) {
  const [, bump] = useState(0);
  useEffect(() => s.watch(() => bump((n) => n + 1)), [s]);
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
}) {
  return (
    <nav class="picker" aria-label="DSPi devices">
      {props.installations.map((inst) => (
        <section key={inst.id} class="installation" aria-label={inst.name}>
          <h2>{inst.name}</h2>
          <ul>
            {inst.sources.map((src) => (
              <SourceRow
                key={sourceKey(src)}
                src={src}
                env={props.env}
                open={props.openKeys.has(`${inst.id}/${sourceKey(src)}`)}
                onOpen={() => props.onOpen(inst, src)}
              />
            ))}
          </ul>
          {inst.editable && props.onAddHub ? <AddHub onAdd={props.onAddHub} error={props.addHubError} /> : null}
        </section>
      ))}
    </nav>
  );
}

function SourceRow(props: { src: Source; env: PageEnv; open: boolean; onOpen: () => void }) {
  const { src, env } = props;
  const label = src.kind === "demo" ? "Demo DSPi" : src.kind === "usb" ? "DSPi on USB" : `Hub ${hostOf(src.url)}`;
  const avail = src.kind === "demo" ? { ok: true, reason: "" } : src.kind === "usb" ? env.usb : env.link;
  return (
    <li class="source">
      <span class="source-name">{label}</span>
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
      {props.session.installationName} · {st.snapshot?.name ?? "Connecting"}
    </>
  );
}

export function SessionView(props: { session: Session; onClose: () => void }) {
  const s = props.session;
  const st = useSessionState(s);
  const snap = st.snapshot;
  const ro = s.readOnlyReason;
  return (
    <article class="session" aria-label={snap?.name ?? "DSPi"} data-status={st.status}>
      <header class="strip">
        <span class={`dot dot-${st.status}`} aria-hidden="true" />
        <strong class="device-name">{snap?.name ?? "Connecting"}</strong>
        {snap ? (
          <span class="device-meta">
            {snap.platform} · firmware {snap.firmware} · {snap.serial}
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
      {ro ? <p class="note read-only-reason">{ro}</p> : null}
      {st.error ? (
        <p class="error" role="alert">
          {st.error}
        </p>
      ) : null}
      {snap && st.status === "open" ? (
        <div class="controls">
          <Volume session={s} disabled={!!ro} />
          <Meters session={s} names={snap.channels.map((c) => c.name)} />
        </div>
      ) : null}
    </article>
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
  const snap = props.session.state.snapshot!;
  const [draft, setDraft] = useState<number | null>(null);
  const shown = draft ?? snap.volume.db;
  return (
    <div class="volume">
      <label>
        Volume
        <input
          type="range"
          min={snap.volume.minDb}
          max={snap.volume.maxDb}
          step={0.5}
          value={shown}
          disabled={props.disabled}
          onInput={(e) => setDraft(Number((e.target as HTMLInputElement).value))}
          onChange={(e) => {
            const v = Number((e.target as HTMLInputElement).value);
            void props.session.write("vol.user", v).then(() => setDraft(null));
          }}
        />
      </label>
      <output class="volume-value" aria-live="polite">
        {snap.volume.db.toFixed(1)} dB
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
          setLevels(await props.session.core.call("meters", undefined));
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
          <li key={n} class="meter">
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
