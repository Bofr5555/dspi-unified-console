// The EQ screen of one session (WP WB6b, the second WB6 slice): the channels
// with their curve-visibility pills (M58), the response graph with on-graph
// editing (M21), the band chip, and the chosen channel's band list (B5). It
// lives inside its session's view, as every screen does.
//
// The curves are the core's (`curves`), asked for only when the bands drawn
// change; the graph draws only then. Edits go through the session (`write`
// by registry path with the read-back), a drag's as fast as the device
// answers, the newest winning. A read-only session shows it all, editing off.

import { memo } from "preact/compat";
import { useCallback, useEffect, useMemo, useRef, useState } from "preact/hooks";
import type { DspState, FilterParams, OutputLabel } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { inputRows, liveInputs, outputRows, type ChannelRow } from "../main/model";
import { BandList } from "./BandList";
import { BandChip, retypeAll } from "./Chip";
import { useCurves, useGrid, type CurveRequest } from "./curves";
import { useBandEdits } from "./edits";
import { EqGraph } from "./EqGraph";
// -- WP WB6c -- the output's page: its settings card and crossover
import { CrossoverList } from "../outputs/Crossover";
import { OutputCard, type OutputLimits } from "../outputs/OutputCard";
// -- WP P1WB2 -- AutoEQ with its preview on the graph
import { AutoEqPanel } from "./AutoEq";
import type { AutoEqPreviewRecord } from "../../core/protocol";
import {
  DEFAULT_LIMITS,
  OFF_BAND,
  availableTypes,
  bypassSupported,
  channelColour,
  isGraphBand,
  setTypeNames,
  typeNamesLoaded,
  type Geometry,
  type Limits,
} from "./model";

interface EqChannel extends ChannelRow {
  output: boolean;
  /** Its colour on the graph. */
  colour: string;
}

// -- WP P1WB2 --
/** Whether a band is already as Clear All leaves it. */
function sameFlat(p: FilterParams): boolean {
  return p.filterType === OFF_BAND.filterType && p.freq === OFF_BAND.freq && p.gain === OFF_BAND.gain && p.q === OFF_BAND.q && !!p.bypass === !!OFF_BAND.bypass;
}

/** The PEQ bands the device takes on a wire channel. */
function peq(s: DspState, wire: number): FilterParams[] {
  return (s.filters?.[wire] ?? []).slice(0, s.maxBands || 10);
}

/** What a channel's curve is made of: its PEQ and, for an output, its crossover. */
function chain(s: DspState, c: EqChannel, bands: FilterParams[]): FilterParams[] {
  return c.output ? [...bands, ...(s.xover?.[c.wire] ?? [])] : bands;
}

/** The level shift drawn on a channel's curve: an output's gain, an input's trim. */
function offsetOf(s: DspState, c: EqChannel): number {
  return (c.output ? s.outputGainDb[c.index] : s.inputPreampDb[c.index]) ?? 0;
}

export function EqScreen(props: ScreenProps) {
  const { session, ask } = props;
  const st = useSessionState(session);
  const s = st.snapshot?.state;
  const ro = session.readOnlyReason !== "";

  // How many inputs USB streams, read once (the main window keeps it current).
  const [usbActive, setUsbActive] = useState<number | null>(null);
  useEffect(() => {
    let alive = true;
    session.core.call("meters", undefined).then(
      (m) => alive && setUsbActive(m.activeInputChannels),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [session]);

  // The registry's ranges, which `write` checks.
  const [limits, setLimits] = useState<Limits>(DEFAULT_LIMITS);
  useEffect(() => {
    let alive = true;
    void Promise.all(["eq.freq", "eq.q", "eq.gain"].map((path) => session.core.call("describe", { path }).catch(() => null))).then(([f, q, g]) => {
      const r = (i: { min?: number; max?: number } | null, d: [number, number]): [number, number] =>
        i?.min !== undefined && i.max !== undefined ? [i.min, i.max] : d;
      if (alive) setLimits({ freq: r(f!, DEFAULT_LIMITS.freq), q: r(q!, DEFAULT_LIMITS.q), gain: r(g!, DEFAULT_LIMITS.gain) });
    });
    return () => {
      alive = false;
    };
  }, [session]);

  // -- WP CG34 -- the filter types' names, the core's table, before any band is shown.
  const [named, setNamed] = useState(typeNamesLoaded);
  useEffect(() => {
    if (named) return;
    let alive = true;
    session.core.call("filterTypeNames", undefined).then(
      (table) => {
        setTypeNames(table);
        if (alive) setNamed(true);
      },
      // A core without the table (the tests' stub): "Type 1" rather than nothing.
      () => alive && setNamed(true),
    );
    return () => {
      alive = false;
    };
  }, [session]);

  // -- WP WB6c -- an output's gain range (the registry's) and longest delay (the device's)
  const [outLimits, setOutLimits] = useState<OutputLimits>(DEFAULT_OUTPUT_LIMITS);
  useEffect(() => {
    let alive = true;
    void Promise.all([session.core.call("describe", { path: "out.gain" }).catch(() => null), session.core.call("outputLimits", undefined).catch(() => null)]).then(([g, d]) => {
      if (!alive) return;
      setOutLimits({
        gain: g?.min !== undefined && g.max !== undefined ? [g.min, g.max] : DEFAULT_OUTPUT_LIMITS.gain,
        delay: d ? [0, d.maxDelayMs] : DEFAULT_OUTPUT_LIMITS.delay,
      });
    });
    return () => {
      alive = false;
    };
  }, [session]);

  if (!s || !Array.isArray(s.filters) || !named) return <p class="empty">This DSPi's filters are not known yet.</p>;
  // -- WP WB6D -- opened from the System view: at its channel and tab
  return <EqView session={session} ask={ask} s={s} ro={ro} usbActive={usbActive} limits={limits} outLimits={outLimits} labels={st.snapshot?.outputLabels} target={props.target} />;
}

// -- WP WB6c --
const DEFAULT_OUTPUT_LIMITS: OutputLimits = { gain: [-60, 20], delay: [0, 21] };

const EqView = memo(
  function EqView(props: {
    session: ScreenProps["session"];
    ask: ScreenProps["ask"];
    s: DspState;
    ro: boolean;
    usbActive: number | null;
    limits: Limits;
    /** -- BUG1 -- the outputs' labels, the snapshot's */
    labels?: OutputLabel[];
    outLimits: OutputLimits;
    /** -- WP WB6D -- where the System view opened this screen */
    target?: ScreenProps["target"];
  }) {
    const { session, ask, s, ro, limits } = props;
    const idBase = useMemo(() => `eq-${Math.random().toString(36).slice(2, 9)}`, []);

    // The channels, as the Console's sidebar lists them.
    const labels = props.labels;
    const channelsKey = JSON.stringify([s.channelNames, s.outputEnabled, s.numInputChannels, s.numOutputChannels, s.inputSource, s.i2sInputChannels, props.usbActive, labels]);
    const channels = useMemo<EqChannel[]>(() => {
      const ins = inputRows(s, liveInputs(s, props.usbActive)).map((r) => ({ ...r, output: false }));
      const outs = outputRows(s, labels).map((r) => ({ ...r, output: true }));
      return [...ins, ...outs].map((c) => ({ ...c, colour: channelColour(c.wire) }));
    }, [channelsKey]);

    // -- WP WB6D -- the System view's channel when it opened this screen (and it is listed)
    const [wire, setWire] = useState<number>(() => (channels.some((c) => c.wire === props.target?.wire) ? props.target!.wire! : (channels[0]?.wire ?? 0)));
    const chosen = channels.find((c) => c.wire === wire) ?? channels[0];
    const cw = chosen?.wire ?? 0;
    useEffect(() => {
      if (chosen && chosen.wire !== wire) setWire(chosen.wire);
    }, [chosen, wire]);

    // Which other channels' curves are shown (M58): this viewer's, in this view.
    const [hidden, setHidden] = useState<ReadonlySet<number>>(new Set());
    // -- WP WB6c -- an output's PEQ or XO bands (the Console's tab toggle), PEQ on every new channel
    // -- WP WB6D -- the System view's tab, on the channel it opened
    const opened = useRef(props.target?.wire === wire ? (props.target?.tab ?? null) : null);
    const [tab, setTab] = useState<"peq" | "xo">(opened.current ?? "peq");
    useEffect(() => {
      setTab(opened.current ?? "peq");
      opened.current = null;
    }, [cw]);

    // The chosen channel's bands from the device, the same array while they are the same.
    const deviceKey = JSON.stringify(peq(s, cw));
    const device = useMemo(() => peq(s, cw), [deviceKey, cw]);
    const edits = useBandEdits(session, cw, device, ro);

    const [sel, setSel] = useState<{ set: ReadonlySet<number>; anchor: number | null }>({ set: new Set(), anchor: null });
    useEffect(() => setSel({ set: new Set(), anchor: null }), [cw]);
    const select = useCallback((set: Set<number>, anchor: number | null) => setSel({ set, anchor: anchor !== null && set.has(anchor) ? anchor : ([...set][0] ?? null) }), []);
    // A band that went off (deleted elsewhere) is not chosen.
    useEffect(() => {
      const keep = [...sel.set].filter((b) => edits.shown[b] && isGraphBand(edits.shown[b]!));
      if (keep.length !== sel.set.size) select(new Set(keep), sel.anchor);
    }, [edits.shown]);

    // The curves: the chosen channel's (as shown, a drag included) first, then the others shown.
    const flat = s.bypass;
    const othersShown = channels.filter((c) => c.wire !== cw && !hidden.has(c.wire));
    const req = useMemo<CurveRequest>(
      () => ({
        chains: flat ? [[], ...othersShown.map(() => [])] : [chosen ? chain(s, chosen, edits.shown) : edits.shown, ...othersShown.map((c) => chain(s, c, peq(s, c.wire)))],
        bands: edits.shown,
      }),
      [JSON.stringify([edits.shown, flat, othersShown.map((c) => [c.wire, peq(s, c.wire), c.output ? s.xover?.[c.wire] : null]), chosen?.output ? s.xover?.[cw] : null])],
    );
    const key = JSON.stringify(req);
    const curves = useCurves(session, key, req);
    const freqs = useGrid(session);
    const othersKey = JSON.stringify([curves?.chains.slice(1), othersShown.map((c) => [c.colour, offsetOf(s, c)])]);
    // -- WP P1WB2 -- AutoEQ: open or not, and its preview (drawn dashed for the edited channel)
    const [autoeq, setAutoeq] = useState(false);
    const [aePreview, setAePreview] = useState<AutoEqPreviewRecord | null>(null);
    const others = useMemo(
      () => {
        const list: { curve: number[]; colour: string; offset: number; dashed?: boolean }[] = othersShown
          .map((c, i) => ({ curve: curves?.chains[i + 1] ?? [], colour: c.colour, offset: offsetOf(s, c) }))
          .filter((o) => o.curve.length);
        // The channel's level after the apply (its gain or preamp, unchanged when the level is not written).
        const mine = aePreview?.channels.find((c) => c.channel === cw);
        if (mine?.curve.length) list.push({ curve: mine.curve, colour: "#e8a33d", offset: mine.levelDb, dashed: true });
        return list;
      },
      [othersKey, aePreview, cw],
    );

    const freqInput = useRef<HTMLInputElement | null>(null);
    const freqRef = useCallback((el: HTMLInputElement | null) => {
      freqInput.current = el;
    }, []);
    const openFields = useCallback(
      (b: number) => {
        select(new Set([b]), b);
        // Once the chip shows that band.
        requestAnimationFrame(() => freqInput.current?.focus());
      },
      [select],
    );

    // -- WP WB6c -- the Linkwitz Transform on outputs
    const available = useMemo(() => availableTypes(s, chosen?.output ?? false), [s.formatVersion, s.fwMajor, s.fwMinor, s.fwPatch, chosen?.output]);
    const bypassOk = bypassSupported(s);
    const anchor = sel.anchor !== null && sel.set.has(sel.anchor) ? sel.anchor : null;
    const chipBand = anchor !== null && edits.shown[anchor] ? { b: anchor, p: edits.shown[anchor]! } : null;
    const chipG: Geometry = { width: 1, height: 1, minFreq: 20, maxFreq: 20000, dbTop: 25, dbBottom: -25 };
    const targets = () => new Map([...sel.set].map((b) => [b, edits.current(b)]));
    const choose = useCallback(
      (b: number, add: boolean) => {
        setSel((cur) => {
          if (!add) return { set: new Set([b]), anchor: b };
          const next = new Set(cur.set);
          if (next.has(b)) next.delete(b);
          else next.add(b);
          return { set: next, anchor: next.has(b) ? b : ([...next][0] ?? null) };
        });
      },
      [],
    );

    if (!chosen) return <p class="empty">This DSPi has no channels to show.</p>;
    const offset = offsetOf(s, chosen);
    // -- WP WB6c -- the crossover, where the firmware has one (outputs only)
    const xoBands = chosen.output ? (s.xover?.[cw] ?? []) : [];
    const showXo = tab === "xo" && xoBands.length > 0;

    return (
      <div class="eq-screen">
        <div class="eq-channels" role="group" aria-label="Channels">
          {channels.map((c) => (
            <span class="eq-channel" key={c.wire}>
              <button type="button" class="eq-pick" aria-pressed={c.wire === cw} onClick={() => setWire(c.wire)} title={c.detail ? `${c.name} (${c.detail})` : c.name}>
                <span class="swatch" style={{ background: c.colour }} aria-hidden="true" />
                <span class="eq-desc">{c.descriptor}</span> <span class="eq-name">{c.name}</span>
              </button>
              <button
                type="button"
                class="eq-eye"
                aria-label={`Show ${c.descriptor} ${c.name} on the graph`}
                aria-pressed={c.wire === cw || !hidden.has(c.wire)}
                disabled={c.wire === cw}
                onClick={() =>
                  setHidden((h) => {
                    const n = new Set(h);
                    if (n.has(c.wire)) n.delete(c.wire);
                    else n.add(c.wire);
                    return n;
                  })
                }
              >
                <span aria-hidden="true">{c.wire === cw || !hidden.has(c.wire) ? "●" : "○"}</span>
              </button>
            </span>
          ))}
        </div>
        {/* -- WP P1WB2 -- */}
        <div class="eq-tools">
          <button type="button" aria-expanded={autoeq} onClick={() => setAutoeq((v) => !v)}>
            AutoEQ…
          </button>
        </div>
        {autoeq ? <AutoEqPanel session={session} channels={channels} wire={cw} disabled={ro} onPreview={setAePreview} /> : null}
        {chosen.output ? (
          <OutputCard
            session={session}
            s={s}
            output={chosen.index}
            name={`${chosen.descriptor} ${chosen.name}`}
            inputs={channels.filter((c) => !c.output)}
            limits={props.outLimits}
            disabled={ro}
            labels={props.labels}
          />
        ) : null}
        <section class="card eq-graph-card" aria-label="Graph">
          <EqGraph
            idBase={idBase}
            channelName={`${chosen.descriptor} ${chosen.name}`}
            edits={edits}
            curves={curves}
            freqs={freqs}
            others={others}
            colour={chosen.colour}
            offset={flat ? 0 : offset}
            selection={sel.set}
            anchor={anchor}
            select={select}
            limits={limits}
            available={available}
            bypassOk={bypassOk}
            disabled={ro}
            openFields={openFields}
          />
          <BandChip
            band={chipBand}
            count={sel.set.size}
            available={available}
            bypassOk={bypassOk}
            limits={limits}
            g={chipG}
            disabled={ro}
            set={(p) => anchor !== null && edits.commit(new Map([[anchor, p]]))}
            retype={(t) => edits.commit(retypeAll(targets(), t))}
            bypass={(on) => edits.commit(new Map([...targets()].map(([b, p]) => [b, { ...p, bypass: on }])))}
            remove={() => {
              edits.commit(new Map([...sel.set].map((b) => [b, { ...OFF_BAND }])));
              setSel({ set: new Set(), anchor: null });
            }}
            freqRef={freqRef}
          />
        </section>
        {xoBands.length ? (
          <div class="band-tabs" role="tablist" aria-label="Bands">
            {(["peq", "xo"] as const).map((t) => (
              <button
                type="button"
                role="tab"
                key={t}
                aria-selected={(t === "xo") === showXo}
                title={(t === "xo") === showXo ? `Currently showing ${t.toUpperCase()} bands` : `Switch to ${t.toUpperCase()} bands`}
                onClick={() => setTab(t)}
              >
                {t.toUpperCase()}
              </button>
            ))}
          </div>
        ) : null}
        {showXo ? (
          <CrossoverList
            session={session}
            wire={cw}
            channelName={`${chosen.descriptor} ${chosen.name}`}
            bands={xoBands}
            freq={limits.freq}
            bypassOk={bypassOk}
            disabled={ro}
            ask={ask}
          />
        ) : (
        <BandList
          channelName={`${chosen.descriptor} ${chosen.name}`}
          bands={edits.shown}
          selection={sel.set}
          available={available}
          bypassOk={bypassOk}
          limits={limits}
          disabled={ro}
          ask={ask}
          choose={choose}
          commit={edits.commit}
          // -- WP P1WB2 -- one undo step: every band flat, as one group
          clearAll={() =>
            session.group("Clear All Bands", async () => {
              // The device's bands as they are now (another console may have changed them).
              await session.refresh();
              const now = peq(session.state.snapshot?.state ?? s, cw);
              for (let b = 0; b < now.length; b++) {
                if (!sameFlat(now[b]!)) await session.core.call("filter", { channel: cw, band: b, ...OFF_BAND }).catch((e) => session.fail(e));
              }
              await session.refresh();
            })
          }
        />
        )}
      </div>
    );
  },
  // The meters and the rest of the device's state do not reach it: only the EQ's own.
  (a, b) =>
    a.session === b.session &&
    a.ask === b.ask &&
    a.ro === b.ro &&
    a.usbActive === b.usbActive &&
    a.limits === b.limits &&
    a.outLimits === b.outLimits &&
    JSON.stringify(a.labels) === JSON.stringify(b.labels) &&
    eqKey(a.s) === eqKey(b.s),
);

function eqKey(s: DspState): string {
  return JSON.stringify([
    s.filters,
    s.xover,
    s.maxBands,
    s.bypass,
    s.channelNames,
    s.outputEnabled,
    s.outputGainDb,
    s.inputPreampDb,
    s.numInputChannels,
    s.numOutputChannels,
    s.inputSource,
    s.i2sInputChannels,
    s.formatVersion,
    s.fwMajor,
    s.fwMinor,
    s.fwPatch,
    // -- WP WB6c -- the output card's
    s.outputMuted,
    s.outputDelayMs,
    s.matrixRouting,
    s.matrixGain,
    s.matrixInvert,
    s.limiterEnabled,
    s.limiterLinkGroup,
    s.limiterThresholdDb,
    s.limiterReleaseMs,
  ]);
}
