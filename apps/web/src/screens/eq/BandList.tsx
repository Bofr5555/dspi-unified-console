// A channel's band list (WP WB6b, B5), after the Console's `FilterListView`:
// one row per band with its type, frequency, gain, Q and bypass, the row's
// number in the band's graph colour (a click chooses the band on the graph);
// Enable All, Bypass All and Clear All below, Clear asked first as on the
// Console. Rows keep one height whatever their type.
//
// -- WP WEB6 -- (REVE §3.1) Qt's table: the bypass disc first (filled in the band's colour while the
// band plays, a ring when bypassed or off; a click switches it), the number, the type as Qt's picker
// (a select drawn as its combo), FREQ, GAIN and Q (WP UWN, was WIDTH) with their units after the fields, an off
// band's values left out; Enable All and Bypass All at the left of the foot, Clear PEQ at the right.

import { memo } from "preact/compat";
import { useState } from "preact/hooks";
import type { FilterParams } from "../../core/protocol";
import type { Ask } from "../../ui/screens";
import { ValueField, useCompact } from "./Field";
// -- WP WB6c --
import { LinkwitzDialog } from "./Linkwitz";
import {
  OFF_BAND,
  T,
  bandColour,
  bareFreq,
  bareGain,
  clamp,
  isGraphBand,
  parseFreq,
  parseNumber,
  qText,
  retyped,
  shortFreq,
  shortGain,
  roundFreq,
  round2,
  round3,
  orderTitle,
  shapeOf,
  typeName,
  usesGain,
  usesQ,
  type Limits,
} from "./model";

// -- WP UWN -- (UXP A17, A10's list) one filter vocabulary on the page: the table says the shape as the
// inspector's shape grid does (Bell, Low Shelf, High Shelf, Low Cut, High Cut, Notch, All Pass) and the
// slope as its Slope list does; off, the Linkwitz Transform and an unknown type keep the core's names.
/** A band's type in the table's list: "Bell", "Low Cut 12 dB/oct", "All Pass 2nd Order". */
export function bandTypeName(t: number): string {
  const s = shapeOf(t);
  if (!s) return typeName(t);
  const two = (s.shape.types as Partial<Record<1 | 2, number>>)[1] !== undefined;
  return two ? `${s.shape.title} ${orderTitle(s.shape, s.order)}` : s.shape.title;
}

export const BandList = memo(function BandList(props: {
  channelName: string;
  bands: FilterParams[];
  selection: ReadonlySet<number>;
  available: number[];
  bypassOk: boolean;
  limits: Limits;
  disabled: boolean;
  ask: Ask;
  choose: (b: number, add: boolean) => void;
  commit: (changes: Map<number, FilterParams>) => void;
  // -- WP P1WB2 -- Clear All as one undo step (P1UG); without it, the bands through `commit`
  clearAll?: () => Promise<void>;
  /** -- WP WEB6 -- the foot's Clear ("Clear PEQ", a linked pair's "Clear 1/2 PEQ"). */
  clearText?: string;
}) {
  const { bands, limits, disabled } = props;
  // On a phone the values are short and their units in the headings; wide, the units after the fields.
  const compact = useCompact();
  const fText = compact ? shortFreq : bareFreq;
  const gText = compact ? shortGain : bareGain;
  const unit = (u: string) => (compact ? undefined : u);
  const set = (b: number, p: FilterParams) => props.commit(new Map([[b, p]]));
  // -- WP WB6c -- the Linkwitz Transform panel of a band
  const [lt, setLt] = useState<number | null>(null);
  const actionable = bands.map((p, b) => (p.filterType !== T.flat ? b : -1)).filter((b) => b >= 0);
  const allBypassed = actionable.every((b) => bands[b]!.bypass);
  const allEnabled = actionable.every((b) => !bands[b]!.bypass);
  const bypassAll = (on: boolean) => props.commit(new Map(actionable.map((b) => [b, { ...bands[b]!, bypass: on }])));
  const clearAll = async () => {
    const a = await props.ask({
      title: "Clear All Bands?",
      body: <p>Every band in this list will be reset to its default (flat) state. One Undo puts them all back.</p>,
      buttons: [
        { label: "Clear All", value: "clear", danger: true },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (a !== "clear") return;
    if (props.clearAll) await props.clearAll();
    else props.commit(new Map(bands.map((_, b) => [b, { ...OFF_BAND }])));
  };

  return (
    <section class="band-list eq-bands eq-table" aria-label={`Bands of ${props.channelName}`}> {/* -- WP WEB3 -- §9, no card */}
      <table>
        <thead>
          <tr>
            {props.bypassOk ? (
              <th scope="col" class="c-on">
                <span class="visually-hidden">Plays</span>
              </th>
            ) : null}
            <th scope="col" class="c-n">
              #
            </th>
            <th scope="col" class="c-type">
              Type
            </th>
            <th scope="col" class="c-freq">
              {compact ? "Hz" : "Freq"}
            </th>
            <th scope="col" class="c-gain">
              {compact ? "dB" : "Gain"}
            </th>
            <th scope="col" class="c-q">
              Q{/* -- WP UWN -- (UXP A17) "Q", as the inspector and the field's own name (was Width) */}
            </th>
          </tr>
        </thead>
        <tbody>
          {bands.map((p, b) => {
            const on = p.filterType !== T.flat;
            const locked = p.filterType === T.linkwitz || !isGraphBand(p);
            const n = `Band ${b + 1}`;
            const types = props.available.includes(p.filterType) ? props.available : [...props.available, p.filterType];
            return (
              <tr key={b} class={`${props.selection.has(b) ? "chosen" : ""}${on ? "" : " band-off"}${p.bypass ? " band-bypassed" : ""}`.trim() || undefined}>
                {props.bypassOk ? (
                  <td class="c-on">
                    {/* -- WP WEB6 -- Qt's bypass disc: "Band N plays" */}
                    <button
                      type="button"
                      role="checkbox"
                      class="band-disc"
                      aria-label={`${n} plays`}
                      aria-checked={on && !p.bypass}
                      title={!on ? undefined : p.bypass ? "Re-enable this band" : "Bypass this band"}
                      disabled={disabled || !on}
                      style={{ "--band": bandColour(b) }}
                      onClick={() => set(b, { ...p, bypass: !p.bypass })}
                    />
                  </td>
                ) : null}
                <td class="c-n">
                  <button
                    type="button"
                    class="band-n"
                    aria-label={n}
                    aria-pressed={props.selection.has(b)}
                    disabled={!on}
                    onClick={(e) => props.choose(b, e.shiftKey || e.ctrlKey || e.metaKey)}
                  >
                    {props.bypassOk ? null : <span class="swatch" style={on ? { background: bandColour(b) } : undefined} aria-hidden="true" />}
                    {b + 1}
                  </button>
                </td>
                <td class="c-type">
                  <select
                    class="combo"
                    aria-label={`${n} type`}
                    disabled={disabled}
                    value={String(p.filterType)}
                    onChange={(e) => {
                      const t = Number((e.target as HTMLSelectElement).value);
                      set(b, t === T.flat ? { ...OFF_BAND, freq: p.freq } : retyped(p, t));
                    }}
                  >
                    {types.map((t) => (
                      <option key={t} value={t}>
                        {bandTypeName(t) /* -- WP UWN -- */}
                      </option>
                    ))}
                  </select>
                </td>
                <td class="c-freq">
                  {on ? (
                    <ValueField
                      label={`${n} frequency`}
                      text={fText(p.freq)}
                      unit={unit("Hz")}
                      parse={parseFreq}
                      disabled={disabled || p.filterType === T.linkwitz}
                      onCommit={(v) => set(b, { ...p, freq: roundFreq(clamp(v, limits.freq)) })}
                    />
                  ) : null}
                </td>
                <td class="c-gain">
                  {p.filterType === T.linkwitz ? (
                    <button
                      type="button"
                      class="lt-edit"
                      aria-label={`${n} Linkwitz Transform`}
                      title="Edit Linkwitz Transform parameters (f0, Q0, fp, Qp)"
                      onClick={() => setLt(b)}
                    >
                      f0, Q0, fp, Qp…
                    </button>
                  ) : on && usesGain(p.filterType) ? (
                    <ValueField
                      label={`${n} gain`}
                      text={gText(p.gain)}
                      unit={unit("dB")}
                      parse={(t) => parseNumber(t, "db")}
                      disabled={disabled}
                      onCommit={(v) => set(b, { ...p, gain: round2(clamp(v, limits.gain)) })}
                    />
                  ) : null}
                </td>
                <td class="c-q">
                  {on && usesQ(p.filterType) ? (
                    <ValueField
                      label={`${n} Q`}
                      text={qText(p.q)}
                      unit={unit("Q")}
                      parse={(t) => parseNumber(t, "")}
                      disabled={disabled || locked}
                      onCommit={(v) => set(b, { ...p, q: round3(clamp(v, limits.q)) })}
                    />
                  ) : null}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <div class="band-list-foot">
        {props.bypassOk ? (
          <span class="split" role="group" aria-label="Bypass">
            <button type="button" disabled={disabled || allEnabled} onClick={() => bypassAll(false)}>
              Enable All
            </button>
            <button type="button" disabled={disabled || allBypassed} onClick={() => bypassAll(true)}>
              Bypass All
            </button>
          </span>
        ) : null}
        <button type="button" class="band-clear" disabled={disabled || !actionable.length} onClick={() => void clearAll()}>
          {props.clearText ?? "Clear All"}
        </button>
      </div>
      {lt !== null && bands[lt]?.filterType === T.linkwitz ? (
        <LinkwitzDialog band={lt} p={bands[lt]!} disabled={disabled} apply={(p) => set(lt, p)} onClose={() => setLt(null)} />
      ) : null}
    </section>
  );
});
