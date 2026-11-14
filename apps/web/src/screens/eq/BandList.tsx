// A channel's band list (WP WB6b, B5), after the Console's `FilterListView`:
// one row per band with its type, frequency, gain, Q and bypass, the row's
// number in the band's graph colour (a click chooses the band on the graph);
// Enable All, Bypass All and Clear All below, Clear asked first as on the
// Console. Rows keep one height whatever their type.

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
  clamp,
  freqText,
  gainText,
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
  typeName,
  usesGain,
  usesQ,
  type Limits,
} from "./model";

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
}) {
  const { bands, limits, disabled } = props;
  // On a phone the values are short and their units in the headings.
  const compact = useCompact();
  const fText = compact ? shortFreq : freqText;
  const gText = compact ? shortGain : gainText;
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
    <section class="card band-list" aria-label={`Bands of ${props.channelName}`}>
      <table>
        <thead>
          <tr>
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
              Q
            </th>
            {props.bypassOk ? (
              <th scope="col" class="c-byp">
                <span class="visually-hidden">Bypass</span>
                <span aria-hidden="true">Byp</span>
              </th>
            ) : null}
          </tr>
        </thead>
        <tbody>
          {bands.map((p, b) => {
            const on = p.filterType !== T.flat;
            const locked = p.filterType === T.linkwitz || !isGraphBand(p);
            const n = `Band ${b + 1}`;
            const types = props.available.includes(p.filterType) ? props.available : [...props.available, p.filterType];
            return (
              <tr key={b} class={props.selection.has(b) ? "chosen" : undefined}>
                <td class="c-n">
                  <button
                    type="button"
                    class="band-n"
                    aria-label={n}
                    aria-pressed={props.selection.has(b)}
                    disabled={!on}
                    onClick={(e) => props.choose(b, e.shiftKey || e.ctrlKey || e.metaKey)}
                  >
                    <span class="swatch" style={on ? { background: bandColour(b) } : undefined} aria-hidden="true" />
                    {b + 1}
                  </button>
                </td>
                <td class="c-type">
                  <select
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
                        {typeName(t)}
                      </option>
                    ))}
                  </select>
                </td>
                <td class="c-freq">
                  <ValueField
                    label={`${n} frequency`}
                    text={on ? fText(p.freq) : "–"}
                    parse={parseFreq}
                    disabled={disabled || !on || p.filterType === T.linkwitz}
                    onCommit={(v) => set(b, { ...p, freq: roundFreq(clamp(v, limits.freq)) })}
                  />
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
                  ) : (
                  <ValueField
                    label={`${n} gain`}
                    text={on && usesGain(p.filterType) ? gText(p.gain) : "–"}
                    parse={(t) => parseNumber(t, "db")}
                    disabled={disabled || !usesGain(p.filterType)}
                    onCommit={(v) => set(b, { ...p, gain: round2(clamp(v, limits.gain)) })}
                  />
                  )}
                </td>
                <td class="c-q">
                  <ValueField
                    label={`${n} Q`}
                    text={on && usesQ(p.filterType) ? qText(p.q) : "–"}
                    parse={(t) => parseNumber(t, "")}
                    disabled={disabled || !usesQ(p.filterType) || locked}
                    onCommit={(v) => set(b, { ...p, q: round3(clamp(v, limits.q)) })}
                  />
                </td>
                {props.bypassOk ? (
                  <td class="c-byp">
                    <input
                      type="checkbox"
                      aria-label={`${n} bypass`}
                      checked={p.bypass}
                      disabled={disabled || !on}
                      onChange={(e) => set(b, { ...p, bypass: (e.target as HTMLInputElement).checked })}
                    />
                  </td>
                ) : null}
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
        <button type="button" disabled={disabled || !actionable.length} onClick={() => void clearAll()}>
          Clear All
        </button>
      </div>
      {lt !== null && bands[lt]?.filterType === T.linkwitz ? (
        <LinkwitzDialog band={lt} p={bands[lt]!} disabled={disabled} apply={(p) => set(lt, p)} onClose={() => setLt(null)} />
      ) : null}
    </section>
  );
});
