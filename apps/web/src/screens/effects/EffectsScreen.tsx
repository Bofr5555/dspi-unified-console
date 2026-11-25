// The effects of one session (WP WB6h): crossfeed, loudness, the leveller,
// psychoacoustic bass, the tube modeller, the subharmonic synthesizer and the
// upmixer, each with its switch and its parameters, after the Console's
// effect windows. The list beside the panel shows which are on.
//
// An effect is shown only when the core's probe found its feature
// (`features`); its parameters' labels, ranges and kinds are the registry's
// (`describe`), the choices' words the core's (`effectChoices`), the values
// the core's mirror. Every change is a `write` of the registry path, read
// back; a slider's drag is one undo step (`setDragging`, through
// `LevelControl`). A frequency or feed change under a named crossfeed
// voicing makes it Custom from the values shown, in one undo step (P1XF).
// Leaving the subharmonic synthesizer switches its solo off, as the Console's
// window does. A read-only session shows it all, the controls off.
//
// Each effect is addressable: `effect` (or a `target` with `effect`, which
// the System view's blocks hand over once the session's `open` exists)
// opens the screen at it; its panel has the id `effects-<id>`.
//
// -- WP WEB3 -- Drawn as Qt's Modern effect panels (DL1), with Qt's words (texts.ts).
//
// -- WP WEBM3 -- The effects are the page's parts (MODLAY 2: one tab each, with its on/off dot),
// in the page head beside the title (ui/shell/Parts.tsx); the one on show is kept per session.

import type { ComponentChildren } from "preact";
import { memo } from "preact/compat";
import { useEffect, useRef, useState } from "preact/hooks";
import type { DspState, EffectCurveGraph, EffectGraphParams, EffectStartingPoint, OutputLabel, ParamInfo, PsybassSketch, SubharmGraph, TubeGraph } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { outputLabel } from "../main/model";
import { PageParts, usePart } from "../../ui/shell/Parts"; // -- WP WEBM3 --
import { LevelControl } from "../outputs/Level";
import {
  asBool,
  asNumber,
  availableEffects,
  choiceOptions,
  crossfeedWrites,
  effectPanelId,
  effectPaths,
  featureNames,
  isEffectId,
  isOn,
  maskBits,
  maskValue,
  paramValue,
  shownParams,
  stepFor,
  toggled,
  valueText,
  type ChoiceWords,
  type EffectDef,
  type EffectId,
  type Features,
  type MaskKind,
  type ParamDef,
} from "./model";
import { EmptyState, FormRow, Glyph, Section, Segments, Switch } from "../../ui/design"; // -- WP WEB -- -- WP WEB3 -- the rest
// -- WP WEB3 -- Qt's words and the page's own stylesheet
import {
  CROSSFEED_PRESET_TEXTS,
  EFFECT_INTROS,
  HARMONICS_TIP,
  LEVELLER_MASK_HINTS,
  LEVELLER_SPEED_TEXTS,
  PARAM_TEXTS,
  SELECTIVITY_PARAM_TEXTS,
  SELECTIVITY_TEXTS,
  TUBE_SHELF_TIP,
  UPMIX_ROUTING,
  headroomTip,
  liftText,
  maskLabel,
  paramCaption,
  paramTitle,
  tubeCaption,
} from "./texts";
// -- WP WEB6 -- Qt's columns, graphs, the tube's picture, the menus
import { CurveGraph, Legend, PsybassGraph, SubharmBands, TubeTransfer, useEffectGraph } from "./graphs";
import { ENDS, columnsOf, harmonicText, headroomText, maskPresets, offAt, placedPaths, upmixStatusText, type Block } from "./layout";
import { TUBE_GROUPS, TubePicture, pushPull, shortTubeName } from "./tube";
import { ActionMenuButton } from "../../ui/menus/ActionMenu";
import { ownRow } from "../../ui/menus/actions";
import "./effects.css";

export type EffectsScreenProps = ScreenProps & {
  /** Open at this effect. */
  effect?: EffectId;
};

/** What the registry says of each path, and the choices' words: read once per session. */
interface Words {
  infos: Map<string, ParamInfo>;
  choices: Record<string, ChoiceWords[]>;
}

function useWords(session: Session): Words | null {
  const [words, setWords] = useState<Words | null>(null);
  useEffect(() => {
    let alive = true;
    void (async () => {
      const infos = new Map<string, ParamInfo>();
      for (const path of effectPaths()) {
        try {
          infos.set(path, await session.core.call("describe", { path }));
        } catch {
          // A path this core does not know is not shown.
        }
      }
      const choices = await session.core.call("effectChoices", undefined).catch(() => ({}));
      if (alive) setWords({ infos, choices });
    })();
    return () => {
      alive = false;
    };
  }, [session]);
  return words;
}

/** What the probe found on the open device, asked again when another device is open in the session. */
function useFeatures(session: Session, device: string): Features {
  const [features, setFeatures] = useState<{ device: string; found: Features }>({ device: "", found: null });
  useEffect(() => {
    let alive = true;
    session.core.call("features", { names: featureNames() }).then(
      (found) => alive && setFeatures({ device, found }),
      // A core without the call gates every probed effect off.
      () => alive && setFeatures({ device, found: {} }),
    );
    return () => {
      alive = false;
    };
  }, [session, device]);
  return features.device === device ? features.found : null;
}

export function EffectsScreen(props: EffectsScreenProps) {
  const { session } = props;
  const st = useSessionState(session);
  const snap = st.snapshot;
  const device = snap ? `${snap.device.serial}/${snap.device.firmware}` : "";
  const features = useFeatures(session, device);
  const words = useWords(session);
  const asked = props.effect ?? (props as { target?: { effect?: unknown } | null }).target?.effect;
  const s = snap?.state;
  // -- WP WEBM3 -- the effects shown, as the page's parts; a target opens its effect
  const list = s && features !== null && words !== null ? availableEffects(s, features) : [];
  const [chosen, setChosen] = usePart(session, "effects", list.map((e) => e.id));
  // A new target while the screen is shown (or as it opens) opens that effect.
  useEffect(() => {
    if (isEffectId(asked)) setChosen(asked);
  }, [asked]);

  // -- WP WEB -- empty states (DESIGN-LANGUAGE §16)
  if (!s || features === null || words === null) return <EmptyState icon="sparkles" title="Reading the effects…" text={s ? undefined : "This DSPi's effects are not known yet."} />;
  if (!list.length) return <EmptyState icon="sparkles" title="No effects" text="This DSPi has no effects this console can set." />;
  const current = list.find((e) => e.id === chosen) ?? list[0]!;
  const ro = session.readOnlyReason !== "";
  return (
    <section class="effects-screen" aria-label="Effects">
      <PageParts label="Effects" parts={list.map((e) => ({ id: e.id, label: e.label, on: isOn(e, s) }))} current={current.id} onPart={setChosen} />
      <EffectPanel key={current.id} session={session} def={current} s={s} features={features} words={words} ro={ro} labels={snap?.outputLabels} />
    </section>
  );
}

// -- WP WEB3 -- (DL1 on the web) An effect is the page, as Qt's Modern panels: the intro row (its icon,
// name and what it does, Basic/Advanced, Solo and the switch at the right), then two columns.
// -- WP WEB6 -- The columns as Qt places them (layout.ts): the graph and what the effect starts from
// at the left, the parameters and outputs at the right; a mask's "Presets ▾" and a graph's
// "Apply preset ▾" as Qt's menus; the tube's picture and type chips; the upmixer's status.
const EffectPanel = memo(function EffectPanel(props: {
  session: Session;
  def: EffectDef;
  s: DspState;
  features: Features;
  words: Words;
  ro: boolean;
  labels?: OutputLabel[];
}) {
  const { session, def, s, words, ro } = props;
  const [advanced, setAdvanced] = useState(false);
  const onInfo = words.infos.get(def.on.path);
  const on = isOn(def, s);
  const params = shownParams(def, s, props.features, !def.split || advanced);
  const outputName = (o: number) => outputLabel(s, props.labels, o);
  const intro = EFFECT_INTROS[def.id];

  // -- the Console's subharmonic window: leaving it switches the solo off
  const sessionRef = useRef(session);
  sessionRef.current = session;
  useEffect(() => {
    if (def.id !== "subharm") return;
    return () => {
      const sess = sessionRef.current;
      if (sess.state.status === "open" && !sess.readOnlyReason && sess.state.snapshot?.state.subharmSolo === true) void sess.write("sub.solo", { bool: false });
    };
  }, [def.id]);

  const solo = params.find((p) => p.path === "sub.solo");
  const soloInfo = solo ? words.infos.get(solo.path) : undefined;
  const soloOn = solo ? asBool(solo.read(s)) : false;
  const rest = params.filter((p) => p !== solo && words.infos.has(p.path));
  const value = (path: string) => asNumber(def.params.find((p) => p.path === path)?.read(s) ?? null);
  const has = (path: string) => rest.some((p) => p.path === path);

  const cols = columnsOf(def.id, def.split === true && advanced);
  const placed = placedPaths(cols);
  const unplaced = rest.filter((p) => !placed.has(p.path));
  const ctx: BlockContext = { session, def, s, words, ro, on, params: rest, value, has, outputName };
  const column = (blocks: Block[]) => blocks.map((b, i) => <BlockView key={`${b.kind}${i}`} ctx={ctx} block={b} />).filter(Boolean);
  const left = cols.left.filter((b) => blockShown(ctx, b));
  const right = cols.right.filter((b) => blockShown(ctx, b));
  if (unplaced.length) right.push({ kind: "section", title: "Parameters", paths: unplaced.map((p) => p.path) });

  return (
    <section class="fx-panel" id={effectPanelId(def.id)} role="tabpanel" aria-label={def.label}>
      <div class="fx-intro">
        <Glyph name={intro.icon} class="fx-icon" />
        <div class="fx-intro-text">
          <h3>{intro.title}</h3>
          <p class="fx-plain">{intro.subtitle}</p>
        </div>
        <div class="fx-intro-tools">
          {def.split ? (
            <Segments
              label={`${def.label} view`}
              options={[
                { value: "basic", text: "Basic" },
                { value: "advanced", text: "Advanced" },
              ]}
              value={advanced ? "advanced" : "basic"}
              choose={(v) => setAdvanced(v === "advanced")}
            />
          ) : null}
          {solo && soloInfo ? (
            // Qt's ToggleChip
            <button
              type="button"
              class="tool-chip"
              aria-pressed={soloOn}
              aria-label={`${def.label} ${paramTitle(solo.path, soloInfo.label)}`}
              title={soloInfo.plain}
              disabled={ro || !soloInfo.writable}
              onClick={() => void session.write(solo.path, paramValue(soloInfo, !soloOn))}
            >
              {paramTitle(solo.path, soloInfo.label)}
            </button>
          ) : null}
          <Switch label={def.label} on={on} disabled={ro || !onInfo} toggle={() => onInfo && void session.write(def.on.path, paramValue(onInfo, !on))} />
        </div>
      </div>
      <div class={`fx-cols${left.length ? "" : " fx-one"}`}>
        {left.length ? <div class="fx-col fx-side">{column(left)}</div> : null}
        <div class="fx-col fx-params">{column(right)}</div>
      </div>
    </section>
  );
});

/** What a block of a column draws from. */
interface BlockContext {
  session: Session;
  def: EffectDef;
  s: DspState;
  words: Words;
  ro: boolean;
  /** The effect is on. */
  on: boolean;
  /** The parameters this device shows here. */
  params: ParamDef[];
  /** A parameter's value in the mirror (0 when it holds none). */
  value: (path: string) => number;
  /** Whether a parameter is shown. */
  has: (path: string) => boolean;
  outputName: (o: number) => string;
}

/** The parameters of a section, in the section's order; depth and hold only while a selectivity mode is chosen. */
function sectionItems(ctx: BlockContext, paths: string[]): ParamDef[] {
  const out = paths.flatMap((path) => ctx.params.filter((p) => p.path === path));
  if (ctx.def.id === "subharm" && ctx.value("sub.select") === 0) return out.filter((p) => p.path !== "sub.depth" && p.path !== "sub.hold");
  if (ctx.def.id === "tube" && ctx.value("tube.xfmr") < 0.5) return out.filter((p) => p.path !== "tube.damping" && p.path !== "tube.resonance");
  return out;
}

function blockShown(ctx: BlockContext, b: Block): boolean {
  if (b.kind === "section") return sectionItems(ctx, b.paths).length > 0;
  if (b.kind === "tube-types") return ctx.has("tube.type");
  return true;
}

function BlockView(props: { ctx: BlockContext; block: Block }) {
  const { ctx, block } = props;
  switch (block.kind) {
    case "graph":
      return <GraphBlock ctx={ctx} title={block.title} points={block.points} />;
    case "section":
      return <SectionBlock ctx={ctx} title={block.title} paths={block.paths} />;
    case "status":
      return <UpmixStatusBlock ctx={ctx} />;
    case "routing":
      return (
        <Section label="Routing" class="fx-group">
          <p class="fx-help">{UPMIX_ROUTING}</p>
        </Section>
      );
    case "picture":
      return <TubeTile ctx={ctx} />;
    case "tube-types":
      return <TubeTypes ctx={ctx} />;
  }
}

/** A mask's "Presets ▾" (Qt's menu of masks at the section label's right). */
function MaskPresetsMenu(props: { ctx: BlockContext; p: ParamDef }) {
  const { ctx, p } = props;
  const info = ctx.words.infos.get(p.path);
  const presets = maskPresets(p.path, ctx.s);
  if (!info || !presets.length) return null;
  return (
    <ActionMenuButton
      label="Presets"
      title={`${ctx.def.label} ${maskLabel(p.mask!)} presets`}
      disabled={ctx.ro || !info.writable}
      items={() => presets.map((m, i) => ownRow(`fx.${p.path}.preset.${i}`, m.label, () => void ctx.session.write(p.path, paramValue(info, m.value))))}
    />
  );
}

function SectionBlock(props: { ctx: BlockContext; title: string; paths: string[] }) {
  const { ctx } = props;
  const items = sectionItems(ctx, props.paths);
  const row = (p: ParamDef) => (
    <ParamRow
      key={`${p.path}:${p.mask ?? ""}`}
      ctx={ctx}
      p={p}
      info={ctx.words.infos.get(p.path)!}
      words={ctx.words.choices[p.path]}
      section={props.title}
    />
  );
  // The tube's output stage: its switch beside the label, the caption under it (Qt's)
  if (ctx.def.id === "tube" && props.title === "Output stage") {
    const xfmr = items.find((p) => p.path === "tube.xfmr");
    const info = xfmr ? ctx.words.infos.get(xfmr.path) : undefined;
    const lit = ctx.value("tube.xfmr") >= 0.5;
    return (
      <Section
        label="Output stage"
        class="fx-group"
        tools={info ? <Switch label="Tube Output stage" on={lit} disabled={ctx.ro || !info.writable} toggle={() => void ctx.session.write("tube.xfmr", paramValue(info, !lit))} /> : undefined}
      >
        <p class="fx-help fx-under-label">{PARAM_TEXTS["tube.xfmr"]?.caption}</p>
        {items.filter((p) => p !== xfmr).map(row)}
      </Section>
    );
  }
  if (!props.title) return <div class="fx-group fx-untitled">{items.map(row)}</div>;
  const masks = items.filter((p) => p.mask);
  const tools =
    masks.length && new Set(masks.map((p) => p.path)).size === 1 ? (
      <MaskPresetsMenu ctx={ctx} p={masks[0]!} />
    ) : ctx.def.id === "tube" && props.title === "Character" ? (
      <span class="fx-help">{tubeTypeName(ctx) ? `from ${tubeTypeName(ctx)}` : "custom"}</span>
    ) : undefined;
  return (
    <Section label={props.title} class="fx-group" tools={tools}>
      {items.map(row)}
    </Section>
  );
}

/** The chosen tube type's name (null for Custom). */
function tubeTypeName(ctx: BlockContext): string | null {
  const t = Math.round(ctx.value("tube.type"));
  if (t <= 0) return null;
  return ctx.words.choices["tube.type"]?.find((c) => c.value === t)?.label ?? null;
}

/** An effect's "Apply preset ▾": its starting points, each written as one undo step. */
function StartingPointsMenu(props: { ctx: BlockContext; effect: "psybass" | "subharm" | "tube" }) {
  const { ctx } = props;
  const [points, setPoints] = useState<EffectStartingPoint[]>([]);
  useEffect(() => {
    let alive = true;
    ctx.session.core.call("effectStartingPoints", { effect: props.effect }).then(
      (p) => alive && setPoints(p),
      () => alive && setPoints([]),
    );
    return () => {
      alive = false;
    };
  }, [ctx.session, props.effect]);
  if (!points.length) return null;
  const apply = (p: EffectStartingPoint) => {
    const writes = p.writes.flatMap(([path, v]) => {
      const info = ctx.words.infos.get(path);
      return info ? [{ path, value: paramValue(info, v) }] : [];
    });
    void ctx.session.group(`${ctx.def.label}, ${p.name}`, () => ctx.session.writeAll(writes));
  };
  return (
    <ActionMenuButton
      label="Apply preset"
      title={`${ctx.def.label} starting points`}
      disabled={ctx.ro}
      items={() => points.map((p, i) => ownRow(`fx.${props.effect}.start.${i}`, `${p.name} - ${p.detail}`, () => apply(p)))}
    />
  );
}

/** The effect's graph, under its section label; the subharmonic's headroom and the tube's harmonics under it. */
function GraphBlock(props: { ctx: BlockContext; title: string; points?: "psybass" | "subharm" | "tube" }) {
  const { ctx } = props;
  const { session, s, value, on } = ctx;
  const tools = props.points ? <StartingPointsMenu ctx={ctx} effect={props.points} /> : undefined;
  let body: ComponentChildren = null;
  let under: ComponentChildren = null;
  switch (ctx.def.id) {
    case "crossfeed":
      body = <CrossfeedGraph session={session} on={on} freq={Number(s.crossfeedFreq ?? 700)} feed={Number(s.crossfeedFeed ?? 4.5)} />;
      break;
    case "loudness":
      body = <LoudnessGraph session={session} on={on} refSpl={Number(s.loudnessRefSpl ?? 83)} intensity={Number(s.loudnessIntensity ?? 100)} />;
      break;
    case "psybass":
      body = <PsybassBlock session={session} on={on} cutoff={Number(s.psybassCutoffHz ?? 80)} harmonics={Number(s.psybassHarmonicsDb ?? 0)} original={Number(s.psybassOriginalDb ?? 0)} />;
      break;
    case "subharm": {
      const extended = ctx.has("sub.top");
      const p = {
        kind: "subharm" as const,
        low: value("sub.low"),
        high: value("sub.high"),
        top: extended ? value("sub.top") : -30,
        boost: value("sub.boost"),
        ceiling: extended && ctx.has("sub.ceiling") ? value("sub.ceiling") : 0,
      };
      body = <SubharmBlock session={session} on={on} params={p} />;
      under = <Headroom session={session} on={on} keyOf={JSON.stringify([on, p, value("sub.mask")])} />;
      break;
    }
    case "tube": {
      const p = {
        kind: "tube" as const,
        drive: value("tube.drive"),
        bias: value("tube.bias"),
        asym: value("tube.asym"),
        hardness: value("tube.hardness"),
        mix: value("tube.mix"),
        trim: value("tube.trim"),
      };
      return <TubeGraphBlock ctx={ctx} title={props.title} params={p} tools={tools} />;
    }
    default:
      return null;
  }
  return (
    <Section label={props.title} class="fx-group fx-graph-group" tools={tools}>
      {body}
      {under}
    </Section>
  );
}

function CrossfeedGraph(props: { session: Session; on: boolean; freq: number; feed: number }) {
  const g = useEffectGraph<EffectCurveGraph>(props.session, { kind: "crossfeed", freq: props.freq, feed: props.feed });
  return (
    <CurveGraph
      label="Frequency Response"
      height={200}
      active={props.on}
      graph={g}
      colours={{ direct: "var(--accent)", crossfeed: "var(--warn)" }}
      tools={
        <Legend
          items={[
            { colour: "var(--accent)", label: "Direct" },
            { colour: "var(--warn)", label: "Crossfeed" },
          ]}
        />
      }
    />
  );
}

function LoudnessGraph(props: { session: Session; on: boolean; refSpl: number; intensity: number }) {
  const g = useEffectGraph<EffectCurveGraph>(props.session, { kind: "loudness", refSpl: props.refSpl, intensity: props.intensity });
  return <CurveGraph label="Compensation Curve" height={260} active={props.on} graph={g} colours={{ curve: "var(--accent)" }} tools={<span class="fx-badge">Curve at -40dB</span>} />;
}

function PsybassBlock(props: { session: Session; on: boolean; cutoff: number; harmonics: number; original: number }) {
  const g = useEffectGraph<PsybassSketch>(props.session, { kind: "psybass", cutoff: props.cutoff, harmonics: props.harmonics, original: props.original });
  return <PsybassGraph height={200} active={props.on} sketch={g} />;
}

function SubharmBlock(props: { session: Session; on: boolean; params: EffectGraphParams }) {
  const g = useEffectGraph<SubharmGraph>(props.session, props.params);
  return <SubharmBands height={200} active={props.on} graph={g} />;
}

/** The subharmonic's headroom cost, the device's (`subharmHeadroom`), read again after a change that moves it. */
function Headroom(props: { session: Session; on: boolean; keyOf: string }) {
  const [db, setDb] = useState<number | null>(null);
  useEffect(() => {
    let alive = true;
    // After the change has reached the device
    const t = setTimeout(() => {
      props.session.core.call("subharmHeadroom", undefined).then(
        (v) => alive && setDb(Number.isFinite(v) ? v : null),
        () => alive && setDb(null),
      );
    }, 150);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [props.session, props.keyOf]);
  const warn = db !== null && db > 0;
  return (
    <div class="fx-headroom" title={headroomTip(db)} data-testid="fx-headroom">
      <span class="section-label">Headroom cost</span>
      <span class={`fx-headroom-value${warn ? " warn" : ""}`}>{headroomText(db)}</span>
    </div>
  );
}

/** The tube's transfer curve with "Apply preset ▾", the harmonics at full scale under it. */
function TubeGraphBlock(props: { ctx: BlockContext; title: string; params: EffectGraphParams; tools?: ComponentChildren }) {
  const g = useEffectGraph<TubeGraph>(props.ctx.session, props.params);
  return (
    <Section label={props.title} class="fx-group fx-graph-group" tools={props.tools}>
      <TubeTransfer height={200} active={props.ctx.on} graph={g} />
      <div class="fx-headroom" title={HARMONICS_TIP} data-testid="fx-harmonics">
        <span class="section-label">At full scale</span>
        <span class="fx-harmonic">
          <span class="fx-help">2nd</span> {g ? harmonicText(g.second) : "–"}
        </span>
        <span class="fx-harmonic">
          <span class="fx-help">3rd</span> {g ? harmonicText(g.third) : "–"}
        </span>
      </div>
    </Section>
  );
}

/** The tube's lift at its damping, under the damping (Qt's). */
function LiftLine(props: { session: Session; damping: number }) {
  const g = useEffectGraph<TubeGraph>(props.session, { kind: "tube", drive: 0, bias: 0, asym: 0, hardness: 0, mix: 0, trim: 0, damping: props.damping });
  return g?.lift ? <p class="fx-help" data-testid="fx-lift">{liftText(g.lift)}</p> : null;
}

/** The upmixer's live status, when the core answers `upmixStatus`. */
interface UpmixLive {
  active: boolean;
  parkedReason: number;
  correlation: number;
  centerGain: number;
  lsGain: number;
  rsGain: number;
}

function UpmixStatusBlock(props: { ctx: BlockContext }) {
  const { session, on } = props.ctx;
  const [live, setLive] = useState<UpmixLive | null>(null);
  useEffect(() => {
    if (!on) {
      setLive(null);
      return;
    }
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    // An older core without `upmixStatus`: the line says only what the mirror knows
    const poll = () => {
      session.core.call("upmixStatus", undefined).then(
        (v) => {
          if (!alive) return;
          setLive(v);
          timer = setTimeout(poll, 500);
        },
        () => alive && setLive(null),
      );
    };
    poll();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [session, on]);
  const text = upmixStatusText(on, live);
  // -- WP WEB6 -- Qt's gauges while it runs: the correlation, the centre's gain unless the centre is
  // off, the surrounds' while they are on
  const centreOff = Math.round(props.ctx.value("up.center_mode")) === 2;
  const surroundOn = Math.round(props.ctx.value("up.surround_mode")) > 0;
  const pct = (v: number) => `${Math.round(v * 100)}%`;
  const corr = live?.correlation ?? 0;
  return (
    <Section label="Status" class="fx-group">
      <p class="fx-status" role="status">
        <span class={`status-dot ${live?.active ? "ok" : "warn"}`} aria-hidden="true" />
        {text}
      </p>
      {live?.active ? (
        <div class="fx-gauges" aria-label="Upmixer gauges" role="group">
          <Gauge label="Correlation" level={(corr + 1) / 2} figure={`${corr >= 0 ? "+" : ""}${corr.toFixed(2)}`} tone="accent" />
          {centreOff ? null : <Gauge label="Centre gain" level={live.centerGain} figure={pct(live.centerGain)} tone="ok" />}
          {surroundOn ? <Gauge label="Ls gain" level={live.lsGain} figure={pct(live.lsGain)} tone="warn" /> : null}
          {surroundOn ? <Gauge label="Rs gain" level={live.rsGain} figure={pct(live.rsGain)} tone="bad" /> : null}
        </div>
      ) : null}
    </Section>
  );
}

/** -- WP WEB6 -- One live gauge (Qt's): its name, a bar, the figure. */
function Gauge(props: { label: string; level: number; figure: string; tone: "accent" | "ok" | "warn" | "bad" }) {
  const level = Math.min(1, Math.max(0, Number.isFinite(props.level) ? props.level : 0));
  return (
    <div class="fx-gauge">
      <span class="fx-gauge-name">{props.label}</span>
      <span class={`fx-gauge-bar fx-gauge-${props.tone}`} role="meter" aria-label={props.label} aria-valuemin={0} aria-valuemax={1} aria-valuenow={level}>
        <span style={{ width: `${level * 100}%` }} />
      </span>
      <span class="fx-gauge-figure">{props.figure}</span>
    </div>
  );
}

/** The tube on show (Qt's showcase): its picture, its name and what it is. */
function TubeTile(props: { ctx: BlockContext }) {
  const { session, value, on } = props.ctx;
  const type = Math.round(value("tube.type"));
  const [family, setFamily] = useState(0);
  useEffect(() => {
    let alive = true;
    session.core.call("tubeFamily", { type: Math.max(0, type) }).then(
      (f) => alive && setFamily(f),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [session, type]);
  const row = type > 0 ? props.ctx.words.choices["tube.type"]?.find((c) => c.value === type) : undefined;
  return (
    <div class={`fx-tube-tile${on ? " on" : ""}`} data-testid="fx-tube-tile">
      <TubePicture family={family} on={on} />
      <p class="fx-tube-name">{row?.label ?? "Custom"}</p>
      <p class="fx-help fx-tube-caption">{tubeCaption(row?.detail ?? null, pushPull(type), value("tube.xfmr") >= 0.5, false)}</p>
    </div>
  );
}

/** The tube types as Qt's shelf: chips grouped by kind, the short name on the chip, the whole in its tooltip. */
function TubeTypes(props: { ctx: BlockContext }) {
  const { ctx } = props;
  const info = ctx.words.infos.get("tube.type");
  const rows = ctx.words.choices["tube.type"] ?? [];
  const type = Math.round(ctx.value("tube.type"));
  if (!info) return null;
  return (
    <Section label="Tube" class="fx-group fx-tube-types">
      <div title={TUBE_SHELF_TIP}>
        {TUBE_GROUPS.map((g) => (
          <div key={g.title} class="fx-tube-group">
            <span class="fx-help">{g.title}</span>
            <div class="fx-chips" role="radiogroup" aria-label={`Tube ${g.title}`}>
              {rows
                .filter((r) => r.value >= g.from && r.value <= g.to)
                .map((r) => (
                  <button
                    type="button"
                    key={r.value}
                    role="radio"
                    class="fx-chip fx-chip-type"
                    aria-checked={r.value === type}
                    title={`${r.label}: ${r.detail}`}
                    disabled={ctx.ro || !info.writable}
                    onClick={() => void ctx.session.write("tube.type", paramValue(info, r.value))}
                  >
                    {shortTubeName(r.label)}
                  </button>
                ))}
            </div>
          </div>
        ))}
      </div>
    </Section>
  );
}

/** Choices up to this many are segments (Speed, Selectivity, the upmixer's engines), more a picker. */
const SEGMENTS_UP_TO = 3;

/** A mask chip's words: the output's number (Qt's chips), a pair "1: Out 1 / Out 2", an input its number. */
function chipText(kind: MaskKind, bit: number): string {
  return kind === "pairs" ? `${bit + 1}: Out ${2 * bit + 1} / Out ${2 * bit + 2}` : String(bit + 1);
}

/** A choice shown with its segments under its title, the chosen one's words under them (Qt's Speed, Selectivity). */
const STACKED = new Set(["lev.speed", "sub.select"]);

function ParamRow(props: { ctx: BlockContext; p: ParamDef; info: ParamInfo; words?: ChoiceWords[]; section: string }) {
  const { ctx, p, info } = props;
  const { session, def, s, ro } = ctx;
  const value = p.read(s);
  const disabled = ro || !info.writable;
  const write = (v: number | boolean) => session.write(p.path, paramValue(info, v));
  const title = paramTitle(p.path, info.label);
  let caption = paramCaption(p.path, info.plain);
  if (p.path === "sub.depth") caption = SELECTIVITY_PARAM_TEXTS.depth;
  if (p.path === "sub.hold") caption = ctx.value("sub.select") === 1 ? SELECTIVITY_PARAM_TEXTS.holdPercussive : SELECTIVITY_PARAM_TEXTS.holdSustained;
  const name = `${def.label} ${title}`;

  if (p.mask) {
    const kind = p.mask;
    const mask = asNumber(value);
    const bits = maskBits(kind, s, ctx.outputName);
    const label = maskLabel(kind);
    const named = kind === "detector" || kind === "apply";
    return (
      <div class="fx-row fx-mask">
        {named ? (
          <span class="fx-title">
            {label} <span class="fx-hint">{LEVELLER_MASK_HINTS[kind]}</span>
          </span>
        ) : null}
        {caption && !named ? <p class="fx-help">{caption}</p> : null}
        {/* Qt's ChipGroup: the channel's number on the chip, its name in the tooltip */}
        <div class="fx-chips" role="group" aria-label={`${def.label} ${label}`}>
          {bits.map((b) => {
            const lit = (mask & (1 << b.bit)) !== 0;
            return (
              <button
                type="button"
                key={b.bit}
                class={`fx-chip${kind === "pairs" ? " fx-chip-pair" : ""}`}
                aria-pressed={lit}
                aria-label={b.short}
                title={b.long}
                disabled={disabled}
                onClick={() => void session.write(p.path, { mask: maskValue(kind, session.state.snapshot?.state ?? s, toggled(mask, b.bit)) })}
              >
                {chipText(kind, b.bit)}
              </button>
            );
          })}
        </div>
      </div>
    );
  }

  switch (info.kind) {
    case "bool": {
      const on = asBool(value);
      return (
        <FormRow class="fx-row fx-bool" title={title} caption={caption || undefined}>
          <Switch label={name} on={on} disabled={disabled} toggle={() => void write(!on)} />
        </FormRow>
      );
    }
    case "choice":
    case "int": {
      const words = choiceOptions(info, props.words);
      // Qt lists Off first (the upmixer's centre keeps Off as its value 2)
      const off = words.find((o) => o.label === "Off");
      const options = off ? [off, ...words.filter((o) => o !== off)] : words;
      const v = Math.round(asNumber(value));
      const known = options.some((o) => o.value === v);
      // The crossfeed's presets: Qt's radio list of the voicings with what each is
      if (p.path === "cf.preset")
        return (
          <div class="fx-row fx-presets" role="radiogroup" aria-label={name}>
            {options.map((o) => {
              const qt = CROSSFEED_PRESET_TEXTS[o.value];
              return (
                <label key={o.value} class="fx-preset">
                  <input type="radio" name={`${def.id}-preset`} value={String(o.value)} checked={o.value === v} disabled={disabled} onChange={() => void write(o.value)} />
                  <span class="fx-preset-text">
                    <span class="fx-preset-name">{qt?.name ?? o.label}</span>
                    <span class="fx-preset-detail">{qt?.detail ?? o.detail}</span>
                  </span>
                </label>
              );
            })}
          </div>
        );
      const detail =
        p.path === "lev.speed"
          ? (LEVELLER_SPEED_TEXTS[v] ?? "")
          : p.path === "sub.select"
            ? (SELECTIVITY_TEXTS[v] ?? "")
            : p.path === "tube.type"
              ? tubeCaption(v > 0 ? (options.find((o) => o.value === v)?.detail ?? null) : null, pushPull(v), ctx.value("tube.xfmr") >= 0.5, true)
              : (options.find((o) => o.value === v)?.detail ?? "");
      const control =
        options.length <= SEGMENTS_UP_TO && known ? (
          <Segments label={name} options={options.map((o) => ({ value: o.value, text: o.label, title: o.detail || undefined }))} value={v} choose={(n) => void write(n)} disabled={disabled} />
        ) : (
          <select aria-label={name} value={String(v)} disabled={disabled} onChange={(e) => void write(Number((e.target as HTMLSelectElement).value))}>
            {known ? null : <option value={String(v)}>{String(v)}</option>}
            {options.map((o) => (
              <option key={o.value} value={String(o.value)}>
                {o.label}
              </option>
            ))}
          </select>
        );
      const engine = p.path === "up.center_mode" || p.path === "up.surround_mode";
      if (STACKED.has(p.path))
        return (
          <div class="fx-row fx-choice fx-stacked">
            {title !== props.section ? <span class="fx-title">{title}</span> : null}
            {control}
            {detail || caption ? <p class="fx-help">{detail || caption}</p> : null}
          </div>
        );
      return (
        <div class={`fx-row fx-choice${engine ? " fx-engine" : ""}`}>
          <div class="fx-choice-head">
            <span class="fx-title">{title}</span>
            {control}
          </div>
          {detail || (caption && !engine) ? <p class="fx-help">{detail || caption}</p> : null}
          {engine && p.path === "up.surround_mode" && caption ? <p class="fx-help">{caption}</p> : null}
        </div>
      );
    }
    case "float": {
      const range: [number, number] = [info.min ?? 0, info.max ?? 1];
      const step = stepFor(info);
      const crossfeed = p.path === "cf.freq" || p.path === "cf.feed";
      const off = offAt(p.path);
      const text = (v: number) => (off === "floor" && v <= range[0] + 1e-6) || (off === "top" && v >= range[1] - 1e-6) ? "Off" : valueText(v, info, step);
      const writeFloat = async (v: number): Promise<boolean> => {
        if (!crossfeed) return write(v);
        // -- P1XF -- under a named voicing: Custom from the values shown, one undo step
        const now = session.state.snapshot?.state ?? s;
        const writes = crossfeedWrites(now, p.path as "cf.freq" | "cf.feed", v);
        if (writes.length === 1) return write(v);
        return session.group(`${def.label}, custom`, () => session.writeAll(writes));
      };
      const ends = ENDS[p.path];
      return (
        <div class="fx-row fx-float">
          <LevelControl
            label={title}
            name={name}
            unit={(info.unit ?? "").replace(/\s+/g, "")}
            value={asNumber(value)}
            range={range}
            slider={range}
            step={step}
            page={10}
            text={text}
            write={writeFloat}
            disabled={disabled}
            session={session}
          />
          {ends ? (
            <div class="fx-ends" aria-hidden="true">
              <span>{ends[0]}</span>
              <span>{ends[1]}</span>
            </div>
          ) : null}
          {caption ? <p class="fx-help">{caption}</p> : null}
          {p.path === "tube.damping" ? <LiftLine session={session} damping={asNumber(value)} /> : null}
        </div>
      );
    }
    default:
      return null;
  }
}
