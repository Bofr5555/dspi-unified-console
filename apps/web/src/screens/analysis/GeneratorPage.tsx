// -- WP WEB7 -- The Signal Generator as Qt Modern's SiggenPanel (REVE §3.1): the waveform tiles five
// to a row, every output the generator can play as a chip three to a row (the core's label; a click
// steps off → on → inverted ø → off; an output the matrix leaves off is dimmed), the level, the
// parameters, the timing for every signal (Sine's Duration too) as Qt's rows, the options Qt shows
// for the type, the transport at the foot. The generator plays only on Start; the rules are
// siggen.ts's (WB6j) and siggen-page.ts's.

import { useEffect, useRef, useState } from "preact/hooks";
import type { SiggenCapsInfo, SiggenConfigRecord, SiggenStatusRecord } from "../../core/tools-protocol";
import { useSessionState } from "../../ui/components";
import { EmptyState, FormRow, Glyph, StatusChip, Switch, rangeFill } from "../../ui/design";
import { readNumber } from "../../ui/number";
import type { ScreenProps } from "../../ui/screens";
import { outputLabel } from "../main/model";
import {
  FLAG_DECORR,
  FLAG_RAW,
  FLAG_WALK,
  LEVEL_MAX,
  LIVE_APPLY_MS,
  SLIDER_MIN,
  STATUS_POLL_MS,
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
import { CHANNEL_ID, PINK, WHITE, idleDetail, noOutputs, stepOutput, timingRows, withTiming } from "./siggen-page";
import { WAVE_BOX, wavePath } from "./waves";
import "./siggen-page.css";

// The generator's decimal fields: text read as Qt's fields read it ("−10,5 dBFS"); Enter commits as
// leaving does; what is not a number puts the value back (as ToolsScreen's).
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

function Heading(props: { text: string; tools?: preact.ComponentChildren }) {
  return (
    <div class="sgq-head">
      <h4 class="section-label">{props.text}</h4>
      {props.tools ? <div class="sgq-tools">{props.tools}</div> : null}
    </div>
  );
}

export function GeneratorPage(props: ScreenProps) {
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
  useEffect(() => {
    if (!open || !isPlaying) return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const run = async () => {
      if (!alive) return;
      if (document.visibilityState !== "hidden") await session.core.call("siggenStatus", undefined).then((x) => alive && setStatus(x), () => undefined);
      if (alive) timer = setTimeout(run, STATUS_POLL_MS);
    };
    void run();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [session, open, isPlaying]);
  useEffect(() => () => clearTimeout(live.current), []);

  const outputs = s ? caps?.outputChannels || s.numOutputChannels : 0;
  // -- (REVE D2) the core's label alone, for every output
  const nameOf = (o: number) => (s ? outputLabel(s, st.snapshot?.outputLabels, o) : `Output ${o + 1}`);
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
  if (caps && !caps.supported) return <EmptyState icon="chart-line" title="Signal generator not available" text="The connected firmware does not include the onboard test signal generator. Update the DSPi firmware to use this tool." />;
  const params = type ? type.params.map((p, i) => ({ p, i })).filter((x) => x.p.semantic !== 0) : [];
  const rows = timingRows(type, draft);
  const sweep = type?.timing === TIMING_SWEEP;
  const channelId = draft.signalType === CHANNEL_ID;
  const noise = draft.signalType === WHITE || draft.signalType === PINK;
  const timingValue = (k: (typeof rows)[number]["kind"]) => (k === "duration" ? draft.durationMs / 1000 : k === "repeat" ? draft.repeat : draft.gapMs);
  return (
    <section class="siggen sgq" aria-label="Signal Generator">
      <div class="sg-intro">
        <Glyph name="audio-lines" class="sg-intro-icon" />
        <div class="sg-intro-text">
          <span class="sg-intro-title">Signal Generator</span>
          <span class="sg-caption">Onboard test and measurement signals</span>
        </div>
        <StatusChip kind={isPlaying ? "ok" : "idle"}>{isPlaying ? "Playing" : "Idle"}</StatusChip>
      </div>
      <div class="sgq-columns">
        <div class="sgq-col">
          <section class="sgq-sec" aria-label="Signal">
            <Heading text="Signal" />
            <div class="siggen-types sgq-tiles" role="radiogroup" aria-label="Signal">
              {(caps?.types ?? []).filter((t) => t.offered).map((t) => (
                <button key={t.id} type="button" role="radio" class="sg-tile" aria-checked={t.id === draft.signalType} title={t.blurb} disabled={disabled} onClick={() => edit(withType(draft, t))}>
                  <svg viewBox={`0 0 ${WAVE_BOX.width} ${WAVE_BOX.height}`} aria-hidden="true" focusable="false">
                    <path d={wavePath(t.tile)} />
                  </svg>
                  <span>{t.tile}</span>
                </button>
              ))}
            </div>
            {type ? <p class="siggen-blurb sg-caption">{type.blurb}</p> : null}
          </section>
          <section class="sgq-sec" aria-label="Outputs">
            <Heading
              text="Outputs"
              tools={
                <>
                  <button type="button" disabled={disabled} onClick={() => edit({ ...draft, channelMask: playable })}>
                    All
                  </button>
                  <button type="button" disabled={disabled} onClick={() => edit(noOutputs(draft))}>
                    None
                  </button>
                </>
              }
            />
            <div class="siggen-outputs sgq-outs" role="group" aria-label="Outputs">
              {Array.from({ length: Math.min(16, outputs) }, (_, o) => {
                const bit = 1 << o;
                if (!(playable & bit)) return null;
                const on = (draft.channelMask & bit) !== 0;
                const inv = on && (draft.invertMask & bit) !== 0;
                const matrixOff = s.outputEnabled[o] === false;
                const walking = isPlaying && status?.activeChannel === o;
                return (
                  <button
                    key={o}
                    type="button"
                    class={`sgq-out${matrixOff ? " matrix-off" : ""}${walking ? " walking" : ""}`}
                    aria-pressed={on}
                    data-inverted={String(inv)}
                    title={matrixOff ? "Disabled in the matrix mixer: it stays silent" : "Click: on → inverted → off"}
                    disabled={disabled}
                    onClick={() => edit(stepOutput(draft, o))}
                  >
                    {nameOf(o)}
                    {inv ? <span class="sgq-inv"> ø<span class="visually-hidden"> inverted</span></span> : null}
                  </button>
                );
              })}
            </div>
            <p class="sg-caption">Click to select, click again to invert polarity (ø). Dimmed outputs are disabled in the matrix mixer and stay silent.</p>
          </section>
        </div>
        <div class="sgq-col sgq-right">
          <section class="sgq-sec" aria-label="Level">
            <Heading
              text="Level"
              tools={
                <span class="sg-value">
                  <input
                    {...DECIMAL}
                    class="siggen-number"
                    value={draft.levelDb.toFixed(1)}
                    disabled={disabled}
                    aria-label="Level in dBFS"
                    onChange={(e) => {
                      const v = typed(e, draft.levelDb, ["dBFS", "dB"]);
                      if (v !== null) edit(withLevel(draft, v));
                    }}
                  />
                  <span class="sg-unit">dB</span>
                </span>
              }
            />
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
          </section>
          {type && params.length ? (
            <section class="sgq-sec" aria-label="Parameters">
              <Heading text="Parameters" />
              <div class="siggen-params sgq-rows">
                {params.map(({ p, i }) => (
                  <div key={i} class="sgq-row">
                    <span class="sgq-label">{p.label}</span>
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
                  </div>
                ))}
              </div>
            </section>
          ) : null}
          {rows.length ? (
            <section class="sgq-sec" aria-label="Timing">
              <Heading text="Timing" />
              {rows.map((r) => (
                <FormRow key={r.kind} title={r.label} caption={r.caption || undefined}>
                  <input
                    {...DECIMAL}
                    class="siggen-number"
                    value={timingValue(r.kind)}
                    disabled={disabled}
                    aria-label={r.kind === "duration" ? (sweep ? "Sweep length in seconds" : "Duration in seconds") : r.kind === "repeat" ? "Repeat count" : "Gap in milliseconds"}
                    onChange={(e) => {
                      const v = typed(e, timingValue(r.kind), r.unit ? [r.unit] : []);
                      if (v !== null) edit(withTiming(draft, r.kind, v, sweep));
                    }}
                  />
                  <span class="sg-unit">{r.unit}</span>
                </FormRow>
              ))}
            </section>
          ) : null}
          <section class="sgq-sec sgq-rule" aria-label="Options">
            <Heading text="Options" />
            <div class="siggen-options" role="group" aria-label="Options">
              <FormRow title="Bypass output EQ (RAW)" caption="Skips crossover and PEQ on the selected outputs. Trim, master volume, mute and delay still apply.">
                <Switch label="Bypass output EQ (RAW)" on={(draft.flags & FLAG_RAW) !== 0} disabled={disabled} toggle={() => edit({ ...draft, flags: draft.flags ^ FLAG_RAW })} />
              </FormRow>
              {noise ? (
                <FormRow title="Decorrelate channels" caption="Independent noise per output instead of one copied signal.">
                  <Switch label="Decorrelate channels" on={(draft.flags & FLAG_DECORR) !== 0} disabled={disabled} toggle={() => edit({ ...draft, flags: draft.flags ^ FLAG_DECORR })} />
                </FormRow>
              ) : null}
              <FormRow title="Walk outputs one at a time" caption={channelId ? "Channel ID always walks: each output plays its channel number as counted blips." : "Plays the selected outputs sequentially instead of together."}>
                <Switch label="Walk outputs one at a time" on={channelId || (draft.flags & FLAG_WALK) !== 0} disabled={disabled || channelId} toggle={() => edit({ ...draft, flags: draft.flags ^ FLAG_WALK })} />
              </FormRow>
            </div>
          </section>
        </div>
      </div>
      <div class="siggen-transport sg-foot" role="group" aria-label="Transport">
        <div>
          <strong class="siggen-title" aria-live="polite">
            {transportTitle(caps, draft, status, blocker)}
          </strong>
          <div class="siggen-detail">{isPlaying ? transportDetail(status, nameOf) : blocker ? "" : idleDetail(draft)}</div>
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
