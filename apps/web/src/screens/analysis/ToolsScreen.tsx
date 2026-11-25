// WP WB6j: the Console's tool windows on the web, each a screen of the session (ids "generator",
// "analyser", "stats", "monitor", which WB6L's shortcuts open): the signal generator (M46), the
// Spectrum Analyser (M22), Stats for nerbs with Reset Watermarks (M53), the Interrupt Monitor (M54).
// Each part reads the core only while it is shown; every change is the core's call.

import { Fragment } from "preact";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import type { MonitorEntryRecord, SiggenCapsInfo, SiggenConfigRecord, SiggenStatusRecord, StatsRead, StatsTickRecord } from "../../core/tools-protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { outputRows } from "../main/model";
import { SpectrumPage } from "./SpectrumPage"; // -- WP WEB6 --
import {
  FLAG_DECORR,
  FLAG_RAW,
  FLAG_WALK,
  LEVEL_MAX,
  LEVEL_MIN,
  LIVE_APPLY_MS,
  SLIDER_MIN,
  STATUS_POLL_MS,
  TIMING_PATTERN,
  TIMING_SWEEP,
  initialDraft,
  playableMask,
  playing,
  startBlocker,
  transportDetail,
  transportTitle,
  typeOf,
  withLevel,
  withParam,
  withType,
} from "./siggen";
import { FAST_MS, MONITOR_POLL_MS, SLOW_MS, appendLines, copyText, eventCount, fillText, lineTime, mergeSlow, slowSections, starvationDelta } from "./stats";
import { EmptyState, FormRow, Glyph, Section, StatusChip, Switch, rangeFill } from "../../ui/design"; // -- WP WEB --
import { WAVE_BOX, wavePath } from "./waves"; // -- WP WEB4 --
import "./analysis.css"; // -- WP WEB4 --
import { readNumber } from "../../ui/number"; // -- WP WEB --

/** Signal Generator (M46). */
export function GeneratorScreen(props: ScreenProps) {
  return <SiggenPanel session={props.session} />;
}

/** The Spectrum Analyser on a screen of its own (M22): -- WP WEB6 -- Qt's Spectrum page. */
export function AnalyserScreen(props: ScreenProps) {
  const st = useSessionState(props.session);
  const snap = st.snapshot;
  if (!snap) return null;
  return <SpectrumPage session={props.session} s={snap.state} labels={snap.outputLabels} />;
}

/** Stats for nerbs (M53). */
export function StatsScreen(props: ScreenProps) {
  return <StatsPanel session={props.session} />;
}

/** Interrupt Monitor (M54). */
export function MonitorScreen(props: ScreenProps) {
  return <MonitorPanel session={props.session} />;
}

/** Run `fn` every `ms` while mounted and the page is seen, first at once. */
function usePoll(fn: () => Promise<void>, ms: number, on: boolean, deps: unknown[]): void {
  useEffect(() => {
    if (!on) return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const run = async () => {
      if (!alive) return;
      if (document.visibilityState !== "hidden") await fn().catch(() => undefined);
      if (alive) timer = setTimeout(run, ms);
    };
    void run();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [on, ms, ...deps]);
}

// ------------------------------------------------------------ signal generator

// -- WP WEB -- the generator's decimal fields: text read as Qt's fields read it ("−10,5 dBFS");
// Enter commits as leaving does; what is not a number puts the value back.
const DECIMAL = {
  type: "text",
  inputMode: "decimal",
  autoComplete: "off",
  onKeyDown: (e: KeyboardEvent) => {
    if (e.key === "Enter") (e.target as HTMLInputElement).blur();
  },
} as const;

function typed(e: Event, was: number, units: string[]): number | null {
  const el = e.target as HTMLInputElement;
  const v = readNumber(el.value, units);
  if (v === null) el.value = String(was);
  return v;
}

function SiggenPanel(props: { session: Session }) {
  const { session } = props;
  const st = useSessionState(session);
  const open = st.status === "open";
  const s = st.snapshot?.state;
  const ro = session.readOnlyReason;
  const [caps, setCaps] = useState<SiggenCapsInfo | null>(null);
  const [draft, setDraft] = useState<SiggenConfigRecord>(initialDraft);
  const [status, setStatus] = useState<SiggenStatusRecord | null>(null);
  const live = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const busy = useRef(false);

  useEffect(() => {
    if (!open) return;
    let alive = true;
    void session.core.call("siggenCaps", undefined).then(
      (c) => alive && setCaps(c),
      () => alive && setCaps(null),
    );
    // An Identify may have started it: read what it does once now.
    void session.core.call("siggenStatus", undefined).then(
      (x) => alive && setStatus(x),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [session, open]);

  const isPlaying = playing(status);
  usePoll(
    async () => {
      setStatus(await session.core.call("siggenStatus", undefined));
    },
    STATUS_POLL_MS,
    open && isPlaying,
    [session],
  );
  useEffect(() => () => clearTimeout(live.current), []);

  const outputs = s ? (caps?.outputChannels || s.numOutputChannels) : 0;
  const rows = useMemo(() => (s ? outputRows(s, st.snapshot?.outputLabels) : []), [s, st.snapshot?.outputLabels]);
  const nameOf = (o: number) => {
    const r = rows.find((x) => x.index === o);
    // -- WP WEB6 -- (REVE D2) the core's label alone, as Qt's chips ("Out 1 · L"), not "OUT1 Out 1 · L"
    return r?.name || `Out ${o + 1}`;
  };
  const playable = playableMask(caps, outputs);
  const type = typeOf(caps, draft.signalType);
  const blocker = startBlocker(open, caps, draft, ro);
  const disabled = !open || !!ro || caps?.supported === false;

  const send = async (cfg: SiggenConfigRecord, start: boolean) => {
    if (busy.current) return;
    busy.current = true;
    try {
      await session.core.call("siggenSet", { config: cfg });
      if (start) await session.core.call("siggenControl", { action: "start" });
      setStatus(await session.core.call("siggenStatus", undefined));
    } catch (e) {
      session.fail(e);
    } finally {
      busy.current = false;
    }
  };
  /** A change of the draft; while it plays, sent once the person pauses (the generator restarts with a fade). */
  const edit = (d: SiggenConfigRecord) => {
    setDraft(d);
    if (!isPlaying) return;
    clearTimeout(live.current);
    live.current = setTimeout(() => {
      if (!startBlocker(open, caps, d, session.readOnlyReason)) void send(d, false);
    }, LIVE_APPLY_MS);
  };
  const stop = async (now: boolean) => {
    clearTimeout(live.current);
    try {
      await session.core.call("siggenControl", { action: now ? "stopNow" : "stop" });
      setStatus(await session.core.call("siggenStatus", undefined));
    } catch (e) {
      session.fail(e);
    }
  };

  if (!s) return null;
  if (caps && !caps.supported) return <EmptyState icon="chart-line" title="No signal generator" text="This DSPi's firmware has no signal generator." />; // -- WP WEB --
  const timing = type?.timing ?? 0;
  // -- WP WEB4 -- Qt's Modern Signal Generator: the intro row with the state's chip, two columns
  // (Signal and Outputs at the left; Level, Parameters, Timing and Options at the right), the
  // waveform tiles, the outputs as chips, the options as switches, and the transport at the foot
  // with Start the one primary.
  const params = type ? type.params.map((p, i) => ({ p, i })).filter((x) => x.p.semantic !== 0) : [];
  const timed = timing === TIMING_SWEEP || timing === TIMING_PATTERN;
  return (
    <section class="siggen" aria-label="Signal Generator">
      <div class="sg-intro">
        <Glyph name="audio-lines" class="sg-intro-icon" />
        <div class="sg-intro-text">
          <span class="sg-intro-title">Signal Generator</span>
          <span class="sg-caption">Onboard test and measurement signals</span>
        </div>
        <StatusChip kind={isPlaying ? "ok" : "idle"}>{isPlaying ? "Playing" : "Idle"}</StatusChip>
      </div>
      <div class="sg-columns">
        <div class="sg-col">
          <Section label="Signal">
            <div class="siggen-types sg-tiles" role="radiogroup" aria-label="Signal">
              {(caps?.types ?? []).filter((t) => t.offered).map((t) => (
                <button
                  key={t.id}
                  type="button"
                  role="radio"
                  class="sg-tile"
                  aria-checked={t.id === draft.signalType}
                  title={t.blurb}
                  disabled={disabled}
                  onClick={() => edit(withType(draft, t))}
                >
                  <svg viewBox={`0 0 ${WAVE_BOX.width} ${WAVE_BOX.height}`} aria-hidden="true" focusable="false">
                    <path d={wavePath(t.tile)} />
                  </svg>
                  <span>{t.tile}</span>
                </button>
              ))}
            </div>
            {type ? <p class="siggen-blurb sg-caption">{type.name}: {type.blurb}</p> : null}
          </Section>
          <Section
            label="Outputs"
            tools={
              <>
                <button type="button" class="sg-quiet" disabled={disabled} onClick={() => edit({ ...draft, channelMask: playable })}>
                  All
                </button>
                <button type="button" class="sg-quiet" disabled={disabled} onClick={() => edit({ ...draft, channelMask: 0 })}>
                  None
                </button>
              </>
            }
          >
            <div class="siggen-outputs sg-outs" role="group" aria-label="Outputs">
              {Array.from({ length: Math.min(16, outputs) }, (_, o) => {
                const bit = 1 << o;
                if (!(playable & bit)) return null;
                const on = (draft.channelMask & bit) !== 0;
                return (
                  <button key={o} type="button" class="chip" aria-pressed={on} disabled={disabled} onClick={() => edit({ ...draft, channelMask: draft.channelMask ^ bit })}>
                    {nameOf(o)}
                  </button>
                );
              })}
            </div>
            <p class="sg-caption">Click to choose; the generator plays on the chosen outputs together, or one at a time with Walk.</p>
          </Section>
        </div>
        <div class="sg-col sg-right">
          <Section
            label="Level"
            tools={
              <span class="sg-value">
                <input
                  {...DECIMAL}
                  class="siggen-number"
                  value={draft.levelDb}
                  disabled={disabled}
                  aria-label="Level in dBFS"
                  onChange={(e) => {
                    const v = typed(e, draft.levelDb, ["dBFS", "dB"]);
                    if (v !== null) edit(withLevel(draft, v));
                  }}
                />
                <span class="sg-unit">dBFS</span>
              </span>
            }
          >
            <input
              type="range"
              class="sg-slider"
              min={SLIDER_MIN}
              max={LEVEL_MAX}
              step={0.5}
              value={Math.max(SLIDER_MIN, draft.levelDb)}
              style={rangeFill(draft.levelDb, SLIDER_MIN, LEVEL_MAX)}
              disabled={disabled}
              aria-label="Level"
              onInput={(e) => edit(withLevel(draft, Number((e.target as HTMLInputElement).value)))}
            />
            <p class="sg-caption">Peak level in dBFS. Output trim, master volume and mute still apply downstream.</p>
          </Section>
          {type && params.length ? (
            <Section label="Parameters">
              <div class="siggen-params">
                {params.map(({ p, i }) => (
                  <FormRow key={i} title={p.label}>
                    {p.semantic === 6 ? (
                      <select aria-label={p.label} disabled={disabled} value={draft.p[i] ?? 0} onChange={(e) => edit(withParam(draft, type, i, Number((e.target as HTMLSelectElement).value)))}>
                        {Array.from({ length: Math.round(p.max - p.min) + 1 }, (_, k) => (
                          <option key={k} value={p.min + k}>
                            Pattern {p.min + k + 1}
                          </option>
                        ))}
                      </select>
                    ) : (
                      <input
                        {...DECIMAL}
                        class="siggen-number"
                        value={draft.p[i] ?? p.def}
                        disabled={disabled}
                        aria-label={p.label}
                        onChange={(e) => {
                          const v = typed(e, draft.p[i] ?? p.def, p.unit ? [p.unit] : []);
                          if (v !== null) edit(withParam(draft, type, i, v));
                        }}
                      />
                    )}
                    <span class="sg-unit">{p.unit}</span>
                  </FormRow>
                ))}
              </div>
            </Section>
          ) : null}
          {timed ? (
            <Section label="Timing">
              <FormRow title={timing === TIMING_SWEEP ? "Length" : "Duration"} caption={timing === TIMING_SWEEP ? undefined : "0 = play until stopped"}>
                <input
                  {...DECIMAL}
                  class="siggen-number"
                  value={draft.durationMs / 1000}
                  disabled={disabled}
                  aria-label={timing === TIMING_SWEEP ? "Sweep length in seconds" : "Duration in seconds"}
                  onChange={(e) => {
                    const v = typed(e, draft.durationMs / 1000, ["s"]);
                    if (v !== null) edit({ ...draft, durationMs: Math.round(Math.min(600, Math.max(0, v)) * 1000) });
                  }}
                />
                <span class="sg-unit">s</span>
              </FormRow>
              {timing === TIMING_SWEEP ? (
                <FormRow title="Repeat" caption="0 = once">
                  <input
                    type="number"
                    class="siggen-number"
                    min={0}
                    max={1000}
                    step={1}
                    value={draft.repeat}
                    disabled={disabled}
                    aria-label="Repeat count"
                    onChange={(e) => {
                      const v = Math.round(Number((e.target as HTMLInputElement).value));
                      if (Number.isFinite(v)) edit({ ...draft, repeat: Math.min(1000, Math.max(0, v)) });
                    }}
                  />
                  <span class="sg-unit" />
                </FormRow>
              ) : null}
            </Section>
          ) : null}
          <Section label="Options">
            <div class="siggen-options" role="group" aria-label="Options">
              {[
                { flag: FLAG_RAW, label: "Bypass output EQ (RAW)", tip: "Skips crossover and PEQ on the selected outputs. Trim, master volume, mute and delay still apply." },
                { flag: FLAG_WALK, label: "Walk outputs one at a time", tip: "Plays the selected outputs sequentially instead of together." },
                { flag: FLAG_DECORR, label: "Decorrelated", tip: "A different noise on each output." },
              ].map((o) => (
                <FormRow key={o.flag} title={o.label} caption={o.tip}>
                  <Switch label={o.label} on={(draft.flags & o.flag) !== 0} disabled={disabled} toggle={() => edit({ ...draft, flags: draft.flags & o.flag ? draft.flags & ~o.flag : draft.flags | o.flag })} />
                </FormRow>
              ))}
            </div>
          </Section>
        </div>
      </div>
      <div class="siggen-transport sg-foot" role="group" aria-label="Transport">
        <div>
          <strong class="siggen-title" aria-live="polite">
            {transportTitle(caps, draft, status, blocker)}
          </strong>
          <div class="siggen-detail">{transportDetail(status, nameOf)}</div>
        </div>
        {isPlaying ? (
          <>
            <button type="button" disabled={!open || !!ro} onClick={() => void stop(false)}>
              Stop
            </button>
            <button type="button" disabled={!open || !!ro} onClick={() => void stop(true)} title="Cut at once, no fade">
              Stop Now
            </button>
          </>
        ) : (
          <button type="button" class="primary" disabled={!!blocker} onClick={() => void send(draft, true)}>
            Start
          </button>
        )}
      </div>
    </section>
  );
}

// ------------------------------------------------------------- stats for nerbs

function Dot(props: { band?: string }) {
  return props.band ? <span class={`stat-dot ${props.band}`} aria-hidden="true" /> : null;
}

function StatsPanel(props: { session: Session }) {
  const { session } = props;
  const st = useSessionState(session);
  const open = st.status === "open";
  const [slow, setSlow] = useState<StatsRead | null>(null);
  const [fast, setFast] = useState<StatsTickRecord | null>(null);
  const [error, setError] = useState("");
  const starv = useRef<{ last: number | null; at: number | null; before: number | null }>({ last: null, at: null, before: null });

  usePoll(
    async () => {
      try {
        const r = await session.core.call("statsRead", undefined);
        const total = r.counters.starvationTotal ?? null;
        if (typeof total === "number") {
          const d = starvationDelta(starv.current.last, total);
          if (d > 0) starv.current = { ...starv.current, before: starv.current.at, at: Date.now() };
          starv.current.last = total;
        }
        setSlow((prev) => mergeSlow(prev, r));
        setError("");
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    SLOW_MS,
    open,
    [session],
  );
  usePoll(
    async () => {
      setFast(await session.core.call("statsTick", undefined));
    },
    FAST_MS,
    open,
    [session],
  );

  const d = st.snapshot?.device;
  const reset = async () => {
    try {
      await session.core.call("statsResetWatermarks", undefined);
    } catch (e) {
      session.fail(e);
    }
  };
  const b = fast?.buffers;
  return (
    <section class="stats" aria-label="Stats for nerbs">
      <div class="stats-head">
        <span class="sg-caption">Updated every 2 seconds</span>
        <button type="button" disabled={!open || !!session.readOnlyReason} onClick={() => void reset()}>
          Reset Watermarks
        </button>
      </div>
      {error ? <p class="error">{error}</p> : null}
      <div class="stats-grid">
        {d ? (
          <section class="card" aria-label="Device Information">
            <h3>Device Information</h3>
            <dl class="stat-rows">
              <dt>Firmware</dt>
              <dd>{d.firmware}</dd>
              <dt>Platform</dt>
              <dd>{d.platform}</dd>
              <dt>Serial</dt>
              <dd>{d.serial || "-"}</dd>
            </dl>
          </section>
        ) : null}
        {b ? (
          <section class="card" aria-label="Buffer Fill Levels">
            <h3>Buffer Fill Levels</h3>
            <dl class="stat-rows">
              <dt>Audio Streaming</dt>
              <dd>
                <Dot band={b.streaming ? "good" : "off"} />
                {b.streaming ? "Yes" : "No"}
              </dd>
              {(fast?.fills ?? []).map((f, i) => (
                <Fragment key={`o${i}`}>
                  <dt>Output {i + 1}</dt>
                  <dd data-band={f.band}>
                    <Dot band={f.band} />
                    {fillText(f)}
                  </dd>
                </Fragment>
              ))}
              {b.spdif.slice(0, b.numSpdif).map((x, i) => (
                <Fragment key={`s${i}`}>
                  <dt>S/PDIF {i + 1}</dt>
                  <dd>
                    {x.fillPct}% ({x.minPct}–{x.maxPct}%)
                  </dd>
                </Fragment>
              ))}
              <dt>PDM Active</dt>
              <dd>{b.pdmActive ? "Yes" : "No"}</dd>
              {b.pdmActive ? (
                <>
                  <dt>DMA Buffer</dt>
                  <dd>
                    {b.pdm.dmaFillPct}% ({b.pdm.dmaMinPct}–{b.pdm.dmaMaxPct}%)
                  </dd>
                  <dt>Ring Buffer</dt>
                  <dd>
                    {b.pdm.ringFillPct}% ({b.pdm.ringMinPct}–{b.pdm.ringMaxPct}%)
                  </dd>
                </>
              ) : null}
            </dl>
          </section>
        ) : null}
        {slow
          ? slowSections(slow).map((sec) => (
              <section class="card" key={sec.title} aria-label={sec.title}>
                <h3>{sec.title}</h3>
                <dl class="stat-rows">
                  {sec.rows.map((r) => (
                    <Fragment key={r.label}>
                      <dt>{r.label}</dt>
                      <dd>
                        <Dot band={r.band} />
                        {r.text}
                      </dd>
                    </Fragment>
                  ))}
                  {sec.title === "S/PDIF DMA Starvation" && starv.current.at ? (
                    <>
                      <dt>Time since last event</dt>
                      <dd>{Math.round((Date.now() - starv.current.at) / 1000)} s</dd>
                    </>
                  ) : null}
                </dl>
                {sec.note ? <p class="muted">{sec.note}</p> : null}
              </section>
            ))
          : null}
      </div>
    </section>
  );
}

// ------------------------------------------------------------ interrupt monitor

function MonitorPanel(props: { session: Session }) {
  const { session } = props;
  const st = useSessionState(session);
  const open = st.status === "open";
  const [lines, setLines] = useState<MonitorEntryRecord[]>([]);
  const [paused, setPaused] = useState(false);
  const last = useRef(0);
  const list = useRef<HTMLOListElement>(null);

  usePoll(
    async () => {
      const b = await session.core.call("monitorRead", { afterId: last.current });
      last.current = b.lastId;
      if (b.entries.length) setLines((l) => appendLines(l, b.entries));
    },
    MONITOR_POLL_MS,
    open && !paused,
    [session],
  );
  useEffect(() => {
    const el = list.current;
    if (el && !paused) el.scrollTop = el.scrollHeight;
  }, [lines.length, paused]);

  const clear = async () => {
    try {
      await session.core.call("monitorClear", undefined);
    } catch {
      // The page's copy is cleared all the same.
    }
    setLines([]);
  };
  const copy = () => {
    void navigator.clipboard?.writeText(copyText(lines)).catch(() => undefined);
  };
  return (
    // -- WP WEB4 -- no card: the tools and the state in one row at the top, the lines in mono, an empty state while there are none
    <section class="monitor" aria-label="Interrupt Monitor">
      <div class="monitor-tools">
        <button type="button" aria-pressed={paused} onClick={() => setPaused((p) => !p)}>
          {paused ? "Resume" : "Pause"}
        </button>
        <button type="button" onClick={() => void clear()}>
          Clear
        </button>
        <button type="button" title="Copies the lines shown, oldest first" disabled={!lines.length} onClick={copy}>
          Copy
        </button>
        <StatusChip kind={open && !paused ? "ok" : "idle"} role="status">
          {!open ? "Inactive" : paused ? "Paused" : "Listening"} · {eventCount(lines.length)}
        </StatusChip>
      </div>
      {/* the empty state's look without its status role: the chip above is this page's status */}
      {lines.length ? null : (
        <div class="empty-state">
          <Glyph name="activity" class="empty-icon" />
          <p class="empty-title">No notifications yet</p>
          <p class="empty-text">The device's notifications are listed here as they come, oldest first.</p>
        </div>
      )}
      <ol class="monitor-lines" ref={list} aria-label="Notification lines" hidden={!lines.length}>
        {lines.map((l) => (
          <li key={l.id}>
            <time>{lineTime(l.timeMs)}</time> <code>{l.text}</code>
          </li>
        ))}
      </ol>
    </section>
  );
}
