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
  maskTitle,
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
  type ParamDef,
} from "./model";
import { EmptyState, Switch } from "../../ui/design"; // -- WP WEB --, WEB2

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

  return (
    <section class="card fx-panel" id={effectPanelId(def.id)} role="tabpanel" aria-label={def.label}>
      <div class="card-head fx-head">
        <h3>{onInfo?.label ?? def.label}</h3>
        {/* -- WP WEB2 -- (REVD W3) the switch (Qt's AppSwitch), not an accent-filled "On" button */}
        <Switch class="fx-switch" label={def.label} on={on} disabled={ro || !onInfo} toggle={() => onInfo && void session.write(def.on.path, paramValue(onInfo, !on))} />
      </div>
      {onInfo?.plain ? <p class="fx-plain">{onInfo.plain}</p> : null}
      {def.split ? (
        <div class="fx-split" role="group" aria-label="View">
          <button type="button" aria-pressed={!advanced} onClick={() => setAdvanced(false)}>
            Basic
          </button>
          <button type="button" aria-pressed={advanced} onClick={() => setAdvanced(true)}>
            Advanced
          </button>
        </div>
      ) : null}
      <div class="fx-params">
        {params.map((p) => {
          const info = words.infos.get(p.path);
          if (!info) return null;
          return <ParamRow key={`${p.path}:${p.mask ?? ""}`} session={session} def={def} p={p} info={info} s={s} words={words.choices[p.path]} ro={ro} outputName={outputName} />;
        })}
      </div>
    </section>
  );
});

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

  if (p.mask) {
    const kind = p.mask;
    const mask = asNumber(value);
    const bits = maskBits(kind, s, props.outputName);
    const title = maskTitle(kind, info.label);
    return (
      <div class="fx-row fx-mask" role="group" aria-label={`${def.label} ${title}`} title={info.plain}>
        <span class="level-caption">{title}</span>
        <div class="fx-chips">
          {bits.map((b) => {
            const lit = (mask & (1 << b.bit)) !== 0;
            return (
              <button
                type="button"
                key={b.bit}
                class="fx-chip"
                aria-pressed={lit}
                title={b.long}
                disabled={disabled}
                onClick={() => void session.write(p.path, { mask: maskValue(kind, session.state.snapshot?.state ?? s, toggled(mask, b.bit)) })}
              >
                {b.short}
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
      // -- WP WEB2 -- (REVD W3) Solo is a toggle chip (Qt's ToggleChip); the other on/off parameters switches
      if (p.path === "sub.solo")
        return (
          <div class="fx-row fx-bool" title={info.plain}>
            <span class="level-caption" aria-hidden="true">
              {info.label}
            </span>
            <button type="button" class="tool-chip" aria-pressed={on} aria-label={`${def.label} ${info.label}`} disabled={disabled} onClick={() => void write(!on)}>
              {info.label}
            </button>
          </div>
        );
      return (
        <div class="fx-row fx-bool" title={info.plain}>
          <span class="level-caption" aria-hidden="true">
            {info.label}
          </span>
          <Switch label={`${def.label} ${info.label}`} on={on} disabled={disabled} toggle={() => void write(!on)} />
        </div>
      );
    }
    case "choice":
    case "int": {
      const options = choiceOptions(info, props.words);
      const v = Math.round(asNumber(value));
      const detail = options.find((o) => o.value === v)?.detail ?? "";
      return (
        <div class="fx-row fx-choice" title={info.plain}>
          <label>
            <span class="level-caption">{info.label}</span>
            <select aria-label={`${def.label} ${info.label}`} value={String(v)} disabled={disabled} onChange={(e) => void write(Number((e.target as HTMLSelectElement).value))}>
              {options.some((o) => o.value === v) ? null : <option value={String(v)}>{String(v)}</option>}
              {options.map((o) => (
                <option key={o.value} value={String(o.value)}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>
          {detail ? <span class="fx-detail">{detail}</span> : null}
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
        <LevelControl
          class="fx-row fx-float"
          label={info.label}
          name={`${def.label} ${info.label}`}
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
      );
    }
    default:
      return null;
  }
}
