// The EQ screen of one session (WP WB6b, the second WB6 slice): the channels
// with their curve-visibility pills (M58), the response graph with on-graph
// editing (M21), the band chip, and the chosen channel's band list (B5). It
// lives inside its session's view, as every screen does.
//
// The curves are the core's (`curves`), asked for only when the bands drawn
// change; the graph draws only then. Edits go through the session (`write`
// by registry path with the read-back), a drag's as fast as the device
// answers, the newest winning. A read-only session shows it all, editing off.
//
// -- WP WBCLIP -- The Channel menu copies the chosen channel's parameters and
// pastes them onto it (M45, screens/clipboard/model.ts); Ctrl/Cmd+C and +V do
// the same while this screen is shown.
//
// -- WP WEB3 -- (DL8 on the web) Drawn as Qt's Modern PEQ: the chosen channel's strip above the graph
// (its pill and name, routing, gain, delay, mute, limiter; an input's link and trim), the channels
// as a list at the left (INPUTS, OUTPUTS: name and pill; the dot shows its curve), the graph on the
// panel tone with no box, the band table under it; AutoEQ…, Channel ▾ and the spectrum in the
// page's tools. Narrow, the channels are one row of pills that scrolls sideways.
//
// -- WP WEB6 -- (REVE §3.1) Qt's Equaliser frame on PEQ and Crossover alike (EqualiserSection): the
// channel strip, the channel list and the inspector stay, the chosen channel kept across the tabs
// (per session, as Qt's one selection); Crossover puts its graph and table where PEQ has its own
// (CrossoverScreen.tsx). The page tools are Qt's: Headphone Profile… (an input's), Files ▾ (Import
// and Export Filters) and the Inspector's toggle; the channel's menu stays on its right click. The
// graph has Qt's toggle chips at its top right (RTA: the spectrum over it; All Curves); the PEQ/XO
// band tabs are gone (an output's crossover bands are on the Crossover tab, as Qt's).

import { memo } from "preact/compat";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "preact/hooks";
import type { DspState, FilterParams, OutputLabel } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { inputRows, liveInputs, outputRows, type ChannelRow } from "../main/model";
import { BandList } from "./BandList";
import { BandChip, retypeAll } from "./Chip";
import { useCurves, useGrid, type CurveRequest } from "./curves";
import { useBandEdits } from "./edits";
import { EqGraph } from "./EqGraph";
// -- WP WB6c -- the output's page: its settings card
import { OutputCard, type OutputLimits } from "../outputs/OutputCard";
// -- WP P1WB2 -- AutoEQ with its preview on the graph
import { AutoEqPanel } from "./AutoEq";
import type { AutoEqPreviewRecord } from "../../core/protocol";
// -- WP WB6G -- an input's page: its pair's link and its trim
import { InputHeader } from "../inputs/InputsScreen";
import { inputLinks } from "../inputs/model";
// -- WP WB6J -- the analyser on a channel's page: its bars (M22) and over the graph (M23)
import { RtaOverlay, SpectrumStrip, tapNames, useAnalyser } from "../analysis/SpectrumStrip";
import { TAP_INPUT, TAP_OUTPUT } from "../analysis/analyser";
import { identifyOutput } from "../analysis/siggen";
// -- WP WBCLIP -- the channel clipboard (M45): the chosen channel's menu, and the keys act on it
import { useClipboard } from "../clipboard/model";
// -- WP WEBM4 -- the channel's menu (the Channel button, a right click on a channel) is actions.json's
import { menuItems, type Handler } from "../../ui/menus/actions";
import { ActionMenuButton, isMenuKey, useContextMenu } from "../../ui/menus/ActionMenu";
import { RenameDialog } from "../main/Dialog"; // -- WP INTD --
// -- WP WEBM2 -- the inspector: the chosen band and channel
import { Inspect, useInspector } from "../../ui/inspector/Inspector";
import { BandPart, EqInspector } from "./EqInspector";
// -- WP WEB6 -- Qt's page tools and the Crossover tab in the same frame
import { useArrangement } from "../../ui/arrangement/Appearance";
import { CrossoverCentre } from "./XoverCentre";
import { OutcomeLine, useDesk } from "../snapshots/Bars";
import { download, type SnapshotDesk } from "../snapshots/model";
import { FilterPicker, type PendingFilters } from "../files/FilesCards";
import { FILTERS_FILE, exportFilters, failureWords, inspectFilters, inspectProblem } from "../files/model";
import type { Session } from "../../ui/session";
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
import { EmptyState } from "../../ui/design"; // -- WP WEB --
import { PageTools } from "../../ui/shell/Parts"; // -- WP WEB3 --
import { Pill } from "../../ui/Pill"; // -- WP WEB3 --
import "./eq.css"; // -- WP WEB3 --

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

// -- WP WEB6 -- the channel chosen on the Equaliser's tabs, per session (Qt's one selection)
const chosenWire = new WeakMap<Session, number>();

export function EqScreen(props: ScreenProps & { eqTab?: "peq" | "crossover" }) {
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

  if (!s || !Array.isArray(s.filters) || !named) return <EmptyState icon="chart-line" title="Reading the filters…" text="This DSPi's filters are not known yet." />; // -- WP WEB --
  // -- WP WB6D -- opened from the System view: at its channel and tab
  return (
    <EqView
      session={session}
      ask={ask}
      s={s}
      ro={ro}
      usbActive={usbActive}
      limits={limits}
      outLimits={outLimits}
      labels={st.snapshot?.outputLabels}
      target={props.target}
      open={props.open}
      eqTab={props.eqTab ?? "peq"}
      desk={props.desk}
    />
  );
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
    /** -- WP WEBM2 -- another page of the session (the inspector's Edit routing) */
    open?: ScreenProps["open"];
    /** -- WP WEB6 -- the tab: PEQ, or Crossover in the same frame */
    eqTab: "peq" | "crossover";
    /** -- WP WEB6 -- Files ▾'s filter files */
    desk?: SnapshotDesk;
  }) {
    const { session, ask, s, ro, limits } = props;
    const mode = props.eqTab;
    const idBase = useMemo(() => `eq-${Math.random().toString(36).slice(2, 9)}`, []);
    // -- WP WEBM2 -- the band bar goes to the inspector when it is beside the page (and not on touch)
    const insp = useInspector();

    // The channels, as the Console's sidebar lists them.
    const labels = props.labels;
    const channelsKey = JSON.stringify([s.channelNames, s.outputEnabled, s.numInputChannels, s.numOutputChannels, s.inputSource, s.i2sInputChannels, props.usbActive, labels]);
    const channels = useMemo<EqChannel[]>(() => {
      const ins = inputRows(s, liveInputs(s, props.usbActive)).map((r) => ({ ...r, output: false }));
      const outs = outputRows(s, labels).map((r) => ({ ...r, output: true }));
      return [...ins, ...outs].map((c) => ({ ...c, colour: channelColour(s, c.wire) }));
    }, [channelsKey]);

    // -- WP WB6D -- the System view's channel when it opened this screen (and it is listed)
    // -- WP WEB6 -- else the one chosen on the other tab
    const [wire, setWire] = useState<number>(() => {
      if (channels.some((c) => c.wire === props.target?.wire)) return props.target!.wire!;
      const kept = chosenWire.get(session);
      return kept !== undefined && channels.some((c) => c.wire === kept) ? kept : (channels[0]?.wire ?? 0);
    });
    const chosen = channels.find((c) => c.wire === wire) ?? channels[0];
    const cw = chosen?.wire ?? 0;
    useEffect(() => {
      if (chosen && chosen.wire !== wire) setWire(chosen.wire);
    }, [chosen, wire]);
    useEffect(() => {
      chosenWire.set(session, cw);
    }, [session, cw]);
    // -- WP WEB6 -- the System view's "Crossover" block opens PEQ at "xo": the crossover is its own tab now
    useEffect(() => {
      if (mode === "peq" && props.target?.tab === "xo" && props.open) props.open({ screen: "xover", wire: cw });
    }, []);
    // -- WP WBCLIP -- the channel Ctrl/Cmd+C and +V act on, while this screen is shown
    // (a layout effect: set as the chosen channel shows, before a key can arrive)
    const board = useClipboard(session);
    useLayoutEffect(() => {
      board.selected = cw;
      return () => {
        if (board.selected === cw) board.selected = null;
      };
    }, [board, cw]);

    // Which other channels' curves are shown (M58): this viewer's, in this view.
    const [hidden, setHidden] = useState<ReadonlySet<number>>(new Set());
    // -- WP WEB6 -- the graph's chips: the spectrum over it (the analyser's choice), every curve
    const analyser = useAnalyser(session);
    const { inspector, showInspector } = useArrangement();

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
    // -- WP INTD -- the channel menu's Rename… (the Overview's level cards that renamed inputs are gone)
    const [renaming, setRenaming] = useState<EqChannel | null>(null);
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

    const ctx = useContextMenu(); // -- WP WEBM4 -- a channel's menu
    // -- WP WEB6 -- Files ▾: a filter file chosen (its channels asked next), why one was refused
    const fileInput = useRef<HTMLInputElement>(null);
    const [pendingFilters, setPendingFilters] = useState<PendingFilters | null>(null);
    const [filesProblem, setFilesProblem] = useState("");
    const [filesUsed, setFilesUsed] = useState(false);
    if (!chosen) return <EmptyState icon="chart-line" title="No channels" text="This DSPi has no channels to show." />; // -- WP WEB --
    // -- WP WEBM2 -- the band bar: under the graph, or in the inspector with the shape grid
    const chip = (
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
    );
    const offset = offsetOf(s, chosen);
    // -- WP WEB6 -- a linked input pair's Clear says so ("Clear 1/2 PEQ"), as Qt's
    const partner = chosen.output ? -1 : inputLinks(session).partner(cw);
    const clearText = partner >= 0 ? `Clear ${Math.min(cw, partner) + 1}/${Math.max(cw, partner) + 1} PEQ` : "Clear PEQ";
    const desk = props.desk;
    const chooseFilterFile = async (input: HTMLInputElement) => {
      const f = input.files?.[0];
      input.value = "";
      if (!f) return;
      setFilesUsed(true);
      try {
        const text = await f.text();
        const inspect = await inspectFilters(session, text);
        const why = inspectProblem(inspect);
        setFilesProblem(why ? `${f.name}: ${why}` : "");
        if (!why) setPendingFilters({ text, name: f.name, inspect });
      } catch (e) {
        setFilesProblem(`${f.name}: ${failureWords(e)}`);
      }
    };
    /** Files ▾ (Qt's MenuTable place "files"): Import Filters…, Export Filters…, the Files page's own. */
    const filesItems = () =>
      menuItems("files", {
        "file.importFilters": {
          run: () => fileInput.current?.click(),
          enabled: !ro && !!desk && !session.document,
          reason: session.readOnlyReason || undefined,
        },
        "file.exportFilters": {
          run: () => {
            if (!desk) return;
            setFilesUsed(true);
            void exportFilters(desk).then((t) => t && download(t, FILTERS_FILE));
          },
          enabled: !!desk && !session.document,
        },
      });
    const allShown = channels.every((c) => c.wire === cw || !hidden.has(c.wire));
    const inspectorShown = inspector !== "hidden";

    // -- WP WEBM4 -- one undo step: every band flat, as one group (the band list's Clear All, and the channel menu's Clear PEQ…)
    const clearAll = () =>
      session.group("Clear All Bands", async () => {
        // The device's bands as they are now (another console may have changed them).
        await session.refresh();
        // -- WP WB6G -- a linked input pair: both sides, in the same step
        const partner = chosen.output ? -1 : inputLinks(session).partner(cw);
        for (const ch of partner >= 0 ? [cw, partner] : [cw]) {
          const now = peq(session.state.snapshot?.state ?? s, ch);
          for (let b = 0; b < now.length; b++) {
            if (!sameFlat(now[b]!)) await session.core.call("filter", { channel: ch, band: b, ...OFF_BAND }).catch((e) => session.fail(e));
          }
        }
        await session.refresh();
      });
    /** A channel's menu (LAY3b's `channel`): the rows this channel has here. */
    const channelItems = (c: EqChannel) => {
      const h: Record<string, Handler> = {
        "channel.rename": { run: () => setRenaming(c), enabled: !ro, reason: session.readOnlyReason || undefined }, // -- WP INTD --
        "channel.copy": { run: () => void board.copy(c.wire) },
        // -- WP WBCLIP2 -- naming the source when it is another device's
        "channel.paste": { run: () => void board.paste(c.wire), text: board.pasteLabel, enabled: !ro && board.hasClip },
        "channel.headphone": {
          run: () => {
            setWire(c.wire);
            setAutoeq(true);
          },
        },
      };
      if (c.output) {
        h["channel.identify"] = { run: () => void identifyOutput(session, c.index), enabled: !ro, reason: "Play counted blips on this output alone" };
        if (props.open) h["channel.routing"] = { run: () => props.open!({ screen: "matrix" }) };
      }
      // The band list's Clear All, on the channel on show
      if (c.wire === cw && !ro)
        h["channel.clear"] = {
          danger: true,
          run: () =>
            void ask({
              title: "Clear All Bands?",
              body: <p>Every band of {c.name} will be reset to its default (flat) state. One Undo puts them all back.</p>,
              buttons: [
                { label: "Clear All", value: "clear", danger: true },
                { label: "Cancel", value: "cancel" },
              ],
            }).then((a) => (a === "clear" ? clearAll() : undefined)),
        };
      return menuItems("channel", h);
    };

    return (
      <div class={`eq-screen${mode === "crossover" ? " eq-mode-xo" : ""}`}>
        <div class="eq-body">
        <div class="eq-channels" role="group" aria-label="Channels">
          {channels.map((c, i) => (
            <span class={`eq-channel${c.output ? " out" : " in"}`} key={c.wire}>
              {/* -- WP WEB3 -- the list's section labels (wide only) */}
              {i === 0 || channels[i - 1]!.output !== c.output ? (
                <span class="section-label eq-list-label" aria-hidden="true">
                  {c.output ? "Outputs" : "Inputs"}
                </span>
              ) : null}
              <button
                type="button"
                class="eq-pick"
                aria-pressed={c.wire === cw}
                onClick={() => setWire(c.wire)}
                title={c.detail ? `${c.name} (${c.detail})` : c.name}
                // -- WP WEBM4 -- its menu: right click, the menu key, Shift+F10
                onContextMenu={(e) => ctx.open(e, `${c.descriptor} ${c.name}`, channelItems(c))}
                onKeyDown={(e) => isMenuKey(e) && ctx.open(e, `${c.descriptor} ${c.name}`, channelItems(c))}
              >
                <span class="eq-name">{c.name}</span>
                <Pill text={c.descriptor} colour={c.colour} ring={c.wire !== cw} class="eq-desc" />
              </button>
              <button
                type="button"
                class="eq-eye"
                aria-label={`Show ${c.descriptor} ${c.name} on the graph`}
                aria-pressed={c.wire === cw || !hidden.has(c.wire)}
                disabled={c.wire === cw}
                style={{ "--ch": c.colour }}
                onClick={() =>
                  setHidden((h) => {
                    const n = new Set(h);
                    if (n.has(c.wire)) n.delete(c.wire);
                    else n.add(c.wire);
                    return n;
                  })
                }
              >
                <span aria-hidden="true" class="eq-eye-dot" />
              </button>
            </span>
          ))}
        </div>
        {ctx.menu /* -- WP WEBM4 -- */}
        {renaming ? (
          // -- WP INTD -- as the matrix's column menu renames an output
          <RenameDialog
            title={`Rename ${renaming.name}`}
            label="Name"
            initial={renaming.stored ?? renaming.name}
            onCancel={() => setRenaming(null)}
            onSave={(name) => {
              const wire = renaming.wire;
              setRenaming(null);
              if (name.trim()) void session.write("ch.name", { text: name }, [wire]);
            }}
          />
        ) : null}
        {/* -- WP P1WB2 -- */}
        {/* -- WP WEB6 -- Qt's page tools (EqualiserTools): Headphone Profile… on an input's page, Files ▾,
            the inspector's toggle. The channel's menu (Copy/Paste, Identify, Clear PEQ…) is its right click. */}
        <PageTools label="Equaliser tools">
        <div class="eq-tools">
          {!chosen.output && mode === "peq" ? (
            <button type="button" aria-expanded={autoeq} onClick={() => setAutoeq((v) => !v)}>
              Headphone Profile…
            </button>
          ) : null}
          <ActionMenuButton label="Files" items={filesItems} />
          <button
            type="button"
            class="eq-inspector-tool"
            aria-expanded={inspectorShown}
            title={inspectorShown ? "Hide the inspector to give the page the full width" : "Show the inspector"}
            onClick={() => showInspector(inspectorShown ? "hidden" : "shown")}
          >
            <span aria-hidden="true">{inspectorShown ? "›" : "‹"}</span> Inspector
          </button>
        </div>
        </PageTools>
        <div class="eq-main">
        {chosen.output ? (
          <OutputCard
            session={session}
            s={s}
            output={chosen.index}
            name={`${chosen.descriptor} ${chosen.name}`}
            tag={chosen.descriptor}
            title={chosen.name}
            colour={chosen.colour}
            inputs={channels.filter((c) => !c.output)}
            limits={props.outLimits}
            disabled={ro}
            labels={props.labels}
          />
        ) : (
          // -- WP WB6G -- an input's page: its pair's link and its trim (the Console's InputChannelHeader)
          <InputHeader session={session} ask={ask} s={s} input={chosen.index} name={`${chosen.descriptor} ${chosen.name}`} title={chosen.name} colour={chosen.colour} disabled={ro} />
        )}
        {/* -- WP WEB6 -- Files ▾: the chosen filter file's channels, and what came of it */}
        <input ref={fileInput} type="file" hidden accept=".txt,text/plain" aria-label="Filter file to import" onChange={(e) => void chooseFilterFile(e.target as HTMLInputElement)} />
        {filesProblem ? (
          <p class="error" role="alert">
            {filesProblem}
          </p>
        ) : null}
        {desk && pendingFilters ? <FilterPicker desk={desk} pending={pendingFilters} onDone={() => setPendingFilters(null)} /> : null}
        {desk && filesUsed ? <FilesOutcome desk={desk} /> : null}
        {mode === "crossover" ? (
          <CrossoverCentre session={session} s={s} freq={limits.freq} labels={props.labels} chosen={chosen.output ? cw : -1} choose={setWire} ask={ask} />
        ) : (
        <>
        {autoeq ? <AutoEqPanel session={session} channels={channels} wire={cw} disabled={ro} onPreview={setAePreview} /> : null}
        <section class="eq-graph-card" aria-label="Graph">
          {/* -- WP WB6J -- the spectrum over the graph, in one box with it */}
          <div class="rta-host">
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
          <RtaOverlay session={session} want={{ tap: chosen.output ? TAP_OUTPUT : TAP_INPUT, mask: 1 << chosen.index }} names={tapNames(s, props.labels)} />
          {/* -- WP WEB6 -- Qt's toggle chips in the graph's top right corner (EqualiserGraphToggles) */}
          <div class="eq-graph-chips" role="group" aria-label="Graph options">
            <button
              type="button"
              class="graph-chip"
              aria-pressed={analyser.choice.graph}
              title="Show the spectrum on the graph"
              onClick={() => analyser.setChoice({ graph: !analyser.choice.graph })}
            >
              RTA
            </button>
            <button
              type="button"
              class="graph-chip"
              aria-pressed={allShown}
              title="Show every channel's curve"
              onClick={() => setHidden(allShown ? new Set(channels.filter((c) => c.wire !== cw).map((c) => c.wire)) : new Set())}
            >
              All Curves
            </button>
          </div>
          </div>
          {insp.band ? null : chip}
        </section>
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
          clearAll={clearAll}
          clearText={clearText}
        />
        </>
        )}
        {/* -- WP WEBM2 -- the inspector: the chosen band (beside the page) and the chosen channel */}
        <Inspect>
          <EqInspector
            session={session}
            s={s}
            channel={chosen}
            labels={props.labels}
            bands={edits.shown}
            freq={limits.freq}
            disabled={ro}
            choose={setWire}
            openRouting={() => props.open?.({ screen: "matrix" })}
            headphones={() => {
              if (mode === "crossover") props.open?.({ screen: "eq", wire: cw });
              setAutoeq(true);
            }}
            setBand={(b, p) => edits.commit(new Map([[b, p]]))}
            band={
              // -- WP WEB6 -- Qt's band part (BandInspector): the sentence with no band chosen
              mode === "peq" && insp.band ? (
                <BandPart
                  band={chipBand}
                  count={sel.set.size}
                  available={available}
                  bypassOk={bypassOk}
                  limits={limits}
                  disabled={ro}
                  set={(p) => anchor !== null && edits.commit(new Map([[anchor, p]]))}
                  retype={(t) => edits.commit(retypeAll(targets(), t))}
                  bypass={(on) => edits.commit(new Map([...targets()].map(([b, p]) => [b, { ...p, bypass: on }])))}
                  reset={() => {
                    edits.commit(new Map([...sel.set].map((b) => [b, { ...OFF_BAND }])));
                    setSel({ set: new Set(), anchor: null });
                  }}
                  freqRef={freqRef}
                />
              ) : null
            }
          />
        </Inspect>
        {/* -- WP WB6J -- this channel's bars, below its bands */}
        {mode === "peq" ? (
          <SpectrumStrip session={session} names={tapNames(s, props.labels)} want={{ tap: chosen.output ? TAP_OUTPUT : TAP_INPUT, mask: 1 << chosen.index }} />
        ) : null}
        </div>
        </div>
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
    a.eqTab === b.eqTab &&
    a.desk === b.desk &&
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

/** -- WP WEB6 -- What Files ▾'s import or export came to (the desk's outcome), on this page. */
function FilesOutcome(props: { desk: SnapshotDesk }) {
  const st = useDesk(props.desk);
  return <OutcomeLine desk={props.desk} outcome={st.outcome} />;
}
