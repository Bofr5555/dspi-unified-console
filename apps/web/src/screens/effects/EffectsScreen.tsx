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

import { memo } from "preact/compat";
import { useEffect, useRef, useState } from "preact/hooks";
import type { DspState, OutputLabel, ParamInfo } from "../../core/protocol";
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
import { EFFECT_INTROS, PARAM_TEXTS, UPMIX_ROUTING, groupBySection, maskLabel, maskSection, paramCaption, paramTitle } from "./texts";
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
// name and what it does, Basic/Advanced, Solo and the switch at the right), then two columns: what
// it acts on and its preset at the left, its parameters in their sections at the right. Every
// parameter is its title with its value field at the right, its slider, and its caption under it.
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
  const side = rest.filter(isSide);
  const main = rest.filter((p) => !isSide(p));
  const row = (p: ParamDef) => (
    <ParamRow key={`${p.path}:${p.mask ?? ""}`} session={session} def={def} p={p} info={words.infos.get(p.path)!} s={s} words={words.choices[p.path]} ro={ro} outputName={outputName} />
  );
  // The side's masks under their section (the leveller's two rows in one), the rest each its own
  const sideGroups = groupBySection(side, (p) => (p.mask ? maskSection(p.mask) : PARAM_TEXTS[p.path]?.section ?? paramTitle(p.path, words.infos.get(p.path)!.label)));
  const mainGroups = groupBySection(main, (p) => PARAM_TEXTS[p.path]?.section);
  const allNone = (p: ParamDef | undefined) => {
    if (!p?.mask || ro) return undefined;
    const kind = p.mask;
    const bits = maskBits(kind, s, outputName);
    const all = bits.reduce((m, b) => m | (1 << b.bit), 0);
    const set = (m: number) => void session.write(p.path, { mask: maskValue(kind, session.state.snapshot?.state ?? s, m) });
    return (
      <>
        <button type="button" class="quiet" aria-label={`${def.label} ${maskLabel(kind)} all`} onClick={() => set(all)}>
          All
        </button>
        <button type="button" class="quiet" aria-label={`${def.label} ${maskLabel(kind)} none`} onClick={() => set(0)}>
          None
        </button>
      </>
    );
  };

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
      <div class={`fx-cols${side.length || def.id === "upmix" ? "" : " fx-one"}`}>
        {side.length || def.id === "upmix" ? (
          <div class="fx-col fx-side">
            {sideGroups.map((g) => (
              <Section key={g.section} label={g.section} class="fx-group" tools={allNone(g.items.filter((p) => p.mask).length === 1 ? g.items.find((p) => p.mask) : undefined)}>
                {g.items.map(row)}
              </Section>
            ))}
            {def.id === "upmix" ? (
              <Section label="Routing" class="fx-group">
                <p class="fx-help">{UPMIX_ROUTING}</p>
              </Section>
            ) : null}
          </div>
        ) : null}
        <div class="fx-col fx-params">
          {mainGroups.map((g) => (
            <Section key={g.section} label={g.section} class="fx-group">
              {g.items.map(row)}
            </Section>
          ))}
        </div>
      </div>
    </section>
  );
});

/** Whether a parameter goes in the left column: what the effect acts on, the crossfeed's preset, the upmixer's engines. */
function isSide(p: ParamDef): boolean {
  if (p.mask || p.path === "cf.preset") return true;
  const section = PARAM_TEXTS[p.path]?.section;
  return section === "Outputs" || section === "Engines";
}

/** Choices up to this many are segments (Speed, Selectivity, the upmixer's engines), more a picker. */
const SEGMENTS_UP_TO = 3;

/** A mask chip's words: the output's number (Qt's chips), a pair "1: Out 1 / Out 2", an input its number. */
function chipText(kind: MaskKind, bit: number): string {
  return kind === "pairs" ? `${bit + 1}: Out ${2 * bit + 1} / Out ${2 * bit + 2}` : String(bit + 1);
}

function ParamRow(props: {
  session: Session;
  def: EffectDef;
  p: ParamDef;
  info: ParamInfo;
  s: DspState;
  words?: ChoiceWords[];
  ro: boolean;
  outputName: (o: number) => string;
}) {
  const { session, def, p, info, s, ro } = props;
  const value = p.read(s);
  const disabled = ro || !info.writable;
  const write = (v: number | boolean) => session.write(p.path, paramValue(info, v));
  const title = paramTitle(p.path, info.label);
  const caption = paramCaption(p.path, info.plain);
  const name = `${def.label} ${title}`;

  if (p.mask) {
    const kind = p.mask;
    const mask = asNumber(value);
    const bits = maskBits(kind, s, props.outputName);
    const label = maskLabel(kind);
    const named = kind === "detector" || kind === "apply";
    return (
      <div class="fx-row fx-mask">
        {named ? <span class="fx-title">{label}</span> : null}
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
        {caption && !named ? <p class="fx-help">{caption}</p> : null}
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
      const options = choiceOptions(info, props.words);
      const v = Math.round(asNumber(value));
      const known = options.some((o) => o.value === v);
      const detail = options.find((o) => o.value === v)?.detail ?? "";
      // The crossfeed's presets: a list of the voicings with what each is (Qt's radio list)
      if (p.path === "cf.preset")
        return (
          <div class="fx-row fx-presets" role="radiogroup" aria-label={name}>
            {options.map((o) => (
              <label key={o.value} class="fx-preset">
                <input type="radio" name={`${def.id}-preset`} value={String(o.value)} checked={o.value === v} disabled={disabled} onChange={() => void write(o.value)} />
                <span class="fx-preset-text">
                  <span class="fx-preset-name">{o.label}</span>
                  {o.detail ? <span class="fx-preset-detail">{o.detail}</span> : null}
                </span>
              </label>
            ))}
          </div>
        );
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
      return (
        <div class="fx-row fx-choice">
          <div class="fx-choice-head">
            <span class="fx-title">{title}</span>
            {control}
          </div>
          {detail || caption ? <p class="fx-help">{detail || caption}</p> : null}
        </div>
      );
    }
    case "float": {
      const range: [number, number] = [info.min ?? 0, info.max ?? 1];
      const step = stepFor(info);
      const crossfeed = p.path === "cf.freq" || p.path === "cf.feed";
      const writeFloat = async (v: number): Promise<boolean> => {
        if (!crossfeed) return write(v);
        // -- P1XF -- under a named voicing: Custom from the values shown, one undo step
        const now = session.state.snapshot?.state ?? s;
        const writes = crossfeedWrites(now, p.path as "cf.freq" | "cf.feed", v);
        if (writes.length === 1) return write(v);
        return session.group(`${def.label}, custom`, () => session.writeAll(writes));
      };
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
            text={(v) => valueText(v, info, step)}
            write={writeFloat}
            disabled={disabled}
            session={session}
          />
          {caption ? <p class="fx-help">{caption}</p> : null}
        </div>
      );
    }
    default:
      return null;
  }
}
