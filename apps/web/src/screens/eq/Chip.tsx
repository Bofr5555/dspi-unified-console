// The band chip (WP WB6b, M21), after the Console's `PeqBandHUD`: the chosen
// band's shape and slope, bypass, and its frequency, gain and Q to read and
// type; Q also steps (− and +), the touch control beside the pinch. On the web it sits under the graph at a fixed height (shown
// with nothing chosen too), so choosing a band never moves the page and a
// finger can reach every value. Shape, slope and bypass apply to every
// chosen band, values to the one shown, as on the Console.
//
// -- WP WEBM2 -- The same bar sits in the Equaliser's inspector when it is beside the page; there
// each value has its name shown above it (`chip-field`'s label, only drawn there).

import { memo } from "preact/compat";
import type { FilterParams } from "../../core/protocol";
import { ValueField, useCompact } from "./Field";
import {
  SHAPES,
  bandColour,
  clamp,
  freqText,
  gainText,
  nudge,
  orderTitle,
  parseFreq,
  parseNumber,
  qText,
  retyped,
  roundFreq,
  round2,
  round3,
  shapeOf,
  typeName,
  usesGain,
  usesQ,
  type Geometry,
  type Limits,
} from "./model";

export const BandChip = memo(function BandChip(props: {
  /** The band shown, and its number; null with nothing chosen. */
  band: { b: number; p: FilterParams } | null;
  /** How many bands are chosen. */
  count: number;
  available: number[];
  bypassOk: boolean;
  limits: Limits;
  g: Geometry;
  disabled: boolean;
  /** One edit of the band shown. */
  set: (p: FilterParams) => void;
  /** The chosen bands to another type, or bypassed. */
  retype: (t: number) => void;
  bypass: (on: boolean) => void;
  remove: () => void;
  freqRef: (el: HTMLInputElement | null) => void;
}) {
  // -- WP WEB6 -- the bar is under the graph only now (the inspector has its own band part): Q
  // keeps its − and + there, the touch control beside the pinch
  const { band, limits, g } = props;
  // On a phone the slopes in short: "12 dB", "2nd".
  const compact = useCompact();
  const slopeName = (shape: Parameters<typeof orderTitle>[0], o: 1 | 2) => (compact ? orderTitle(shape, o).replace("/oct", "").replace(" Order", "") : orderTitle(shape, o));
  const p = band?.p ?? null;
  const off = props.disabled || !p;
  const s = p ? shapeOf(p.filterType) : null;
  const types = s?.shape.types as Partial<Record<1 | 2, number>> | undefined;
  const slopes = s && types?.[1] !== undefined && types[2] !== undefined && props.available.includes(types[1]) && props.available.includes(types[2]);
  const step = (key: "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown", alt = false) => () => {
    if (!p) return;
    const n = nudge(p, key, alt, false, g, limits);
    if (n) props.set(n);
  };

  return (
    <div class="band-chip" role="group" aria-label={band ? `Band ${band.b + 1}` : "No band chosen"}>
      <div class="chip-row">
        <span class="chip-band">
          <span class="swatch" style={band ? { background: bandColour(band.b) } : undefined} aria-hidden="true" />
          {band ? (
            <>
              <span class="chip-word">Band </span>
              {band.b + 1}
              {props.count > 1 ? <span class="chip-word"> of {props.count}</span> : null}
            </>
          ) : (
            "–"
          )}
        </span>
        <select
          aria-label="Shape"
          disabled={off || !s}
          value={s ? s.shape.id : ""}
          onChange={(e) => {
            const shape = SHAPES.find((x) => x.id === (e.target as HTMLSelectElement).value);
            if (!shape || !p) return;
            const t = (shape.types as Partial<Record<1 | 2, number>>)[s?.order ?? 2] ?? shape.types[2];
            if (t !== undefined && props.available.includes(t)) props.retype(t);
          }}
        >
          {!s ? <option value="">{p ? typeName(p.filterType) : "Shape"}</option> : null}
          {SHAPES.filter((x) => props.available.includes(x.types[2])).map((x) => (
            <option key={x.id} value={x.id}>
              {x.title}
            </option>
          ))}
        </select>
        <select
          aria-label="Slope"
          disabled={off || !slopes}
          value={s ? String(s.order) : "2"}
          onChange={(e) => {
            const o = Number((e.target as HTMLSelectElement).value) as 1 | 2;
            const t = types?.[o];
            if (t !== undefined) props.retype(t);
          }}
        >
          {s && slopes ? (
            ([2, 1] as const).map((o) => (
              <option key={o} value={o}>
                {slopeName(s.shape, o)}
              </option>
            ))
          ) : (
            <option value="2">{s ? slopeName(s.shape, 2) : "Slope"}</option>
          )}
        </select>
        {props.bypassOk ? (
          <button type="button" class="toggle" aria-pressed={p?.bypass ?? false} disabled={off} onClick={() => p && props.bypass(!p.bypass)}>
            Bypass
          </button>
        ) : null}
        <button type="button" class="chip-delete" aria-label="Delete" disabled={off} onClick={props.remove}>
          <span class="chip-word">Delete</span>
          <span class="chip-x" aria-hidden="true">
            ✕
          </span>
        </button>
      </div>
      <div class="chip-row chip-values">
        <span class="chip-field" data-label="Frequency">
          <ValueField
            label="Frequency"
            text={p ? freqText(p.freq) : "–"}
            parse={parseFreq}
            disabled={off}
            inputRef={props.freqRef}
            onCommit={(v) => p && props.set({ ...p, freq: roundFreq(clamp(v, limits.freq)) })}
          />
        </span>
        <span class="chip-field" data-label="Gain">
          <ValueField
            label="Gain"
            text={p && usesGain(p.filterType) ? gainText(p.gain) : "–"}
            parse={(t) => parseNumber(t, "db")}
            disabled={off || !p || !usesGain(p.filterType)}
            onCommit={(v) => p && props.set({ ...p, gain: round2(clamp(v, limits.gain)) })}
          />
        </span>
        <span class="chip-field" data-label="Q">
          <ValueField
            label="Q"
            text={p && usesQ(p.filterType) ? qText(p.q) : "–"}
            parse={(t) => parseNumber(t, "")}
            disabled={off || !p || !usesQ(p.filterType)}
            onCommit={(v) => p && props.set({ ...p, q: round3(clamp(v, limits.q)) })}
            step={{ down: step("ArrowDown", true), up: step("ArrowUp", true) }}
          />
        </span>
      </div>
    </div>
  );
});

/** The chosen bands to type `t` (the chip's shape and slope, the list's type). */
export function retypeAll(bands: Map<number, FilterParams>, t: number): Map<number, FilterParams> {
  return new Map([...bands].map(([b, p]) => [b, retyped(p, t)]));
}
