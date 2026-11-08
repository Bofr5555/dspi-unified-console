// An output's crossover (WP WB6c, B8 and M50), after the Console's XO tab
// (`crossoverRowContent`, Components.swift): four bands, each with its bypass,
// family (Off, Linkwitz-Riley, Butterworth, Bessel), pass (Low Pass, High
// Pass), slope and frequency; Enable All / Bypass All (Bypass All asks first,
// with the Console's warning) and Clear All below. The names a screen reader
// hears are the core's ("LR4 High Pass", `filterTypeNames`).
//
// The type goes through the core's `crossoverType` (the registry's `eq.type`
// takes PEQ types only); frequency and bypass are `write`s of `eq.freq` and
// `eq.bypass` on band 20 + n. Every change is read back.

import { memo } from "preact/compat";
import type { FilterParams } from "../../core/protocol";
import type { Ask } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { ValueField, useCompact } from "../eq/Field";
import { clamp, freqText, parseFreq, roundFreq, shortFreq, typeName } from "../eq/model";
import { FAMILIES, FAMILY_ORDERS, XOVER_BASE, decodeXover, encodeXover, slopeText, withFamily } from "./model";

/** On a phone the choices in short, as the Console's short labels ("LR4 LP"). */
const SHORT_FAMILIES = ["Off", "LR", "BW", "BES"];

/** What the Console writes to clear a band (`clearCrossover`): off, 1 kHz. */
const CLEARED_FREQ = 1000;

export const CrossoverList = memo(function CrossoverList(props: {
  session: Session;
  /** The output's wire channel. */
  wire: number;
  channelName: string;
  bands: FilterParams[];
  freq: [number, number];
  bypassOk: boolean;
  disabled: boolean;
  ask: Ask;
}) {
  const { session, wire, bands, disabled } = props;
  const compact = useCompact();
  const setType = (b: number, filterType: number, freq?: number) =>
    void session.change("crossoverType", freq === undefined ? { channel: wire, band: b, filterType } : { channel: wire, band: b, filterType, freq });
  const armed = bands.map((p, b) => (p.filterType !== 0 ? b : -1)).filter((b) => b >= 0);
  const allBypassed = armed.every((b) => bands[b]!.bypass);
  const allEnabled = armed.every((b) => !bands[b]!.bypass);
  const bypassAll = (on: boolean) =>
    void session.writeAll(armed.filter((b) => bands[b]!.bypass !== on).map((b) => ({ path: "eq.bypass", value: { bool: on }, indices: [wire, XOVER_BASE + b] })));
  const askBypass = async () => {
    const a = await props.ask({
      title: "Bypass this output's crossovers?",
      body: (
        <p>
          This sends full-range audio to this output with no crossover protection, which can damage unprotected drivers such as tweeters. Continue only if you
          are sure.
        </p>
      ),
      buttons: [
        { label: "Bypass All", value: "bypass", danger: true },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (a === "bypass") bypassAll(true);
  };
  const clearAll = async () => {
    const a = await props.ask({
      title: "Clear All Bands?",
      body: <p>Every band in this list will be reset to its default (flat) state. This cannot be undone.</p>,
      buttons: [
        { label: "Clear All", value: "clear", danger: true },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (a !== "clear") return;
    for (let b = 0; b < bands.length; b++) {
      const p = bands[b]!;
      if (p.filterType !== 0 || p.freq !== CLEARED_FREQ) await session.change("crossoverType", { channel: wire, band: b, filterType: 0, freq: CLEARED_FREQ });
      if (p.bypass) await session.write("eq.bypass", { bool: false }, [wire, XOVER_BASE + b]);
    }
  };

  return (
    <section class="card band-list xover-list" aria-label={`Crossover of ${props.channelName}`}>
      <table>
        <thead>
          <tr>
            {props.bypassOk ? (
              <th scope="col" class="c-byp">
                <span class="visually-hidden">Bypass</span>
              </th>
            ) : null}
            <th scope="col" class="c-n">
              #
            </th>
            <th scope="col">Family</th>
            <th scope="col">Type</th>
            <th scope="col">Slope</th>
            <th scope="col" class="c-freq">
              Freq
            </th>
          </tr>
        </thead>
        <tbody>
          {bands.map((p, b) => {
            const x = decodeXover(p.filterType);
            const on = x.family !== 0;
            const n = `Crossover ${b + 1}`;
            return (
              <tr key={b} class={!on || p.bypass ? "xover-off" : undefined} title={on ? typeName(p.filterType) : undefined}>
                {props.bypassOk ? (
                  <td class="c-byp">
                    <input
                      type="checkbox"
                      aria-label={`${n} active`}
                      checked={on && !p.bypass}
                      disabled={disabled || !on}
                      onChange={(e) => void session.write("eq.bypass", { bool: !(e.target as HTMLInputElement).checked }, [wire, XOVER_BASE + b])}
                    />
                  </td>
                ) : null}
                <td class="c-n">{b + 1}</td>
                <td>
                  <select
                    aria-label={`${n} family`}
                    disabled={disabled}
                    value={String(x.family)}
                    onChange={(e) => setType(b, withFamily(p.filterType, Number((e.target as HTMLSelectElement).value)))}
                  >
                    {FAMILIES.map((f, i) => (
                      <option key={i} value={i}>
                        {compact ? SHORT_FAMILIES[i] : f}
                      </option>
                    ))}
                  </select>
                </td>
                <td>
                  <select
                    aria-label={`${n} type`}
                    disabled={disabled || !on}
                    value={x.highPass ? "hp" : "lp"}
                    onChange={(e) => setType(b, encodeXover({ ...x, highPass: (e.target as HTMLSelectElement).value === "hp" }))}
                  >
                    {on ? (
                      <>
                        <option value="lp">{compact ? "LP" : "Low Pass"}</option>
                        <option value="hp">{compact ? "HP" : "High Pass"}</option>
                      </>
                    ) : (
                      <option value="lp">–</option>
                    )}
                  </select>
                </td>
                <td>
                  <select
                    aria-label={`${n} slope`}
                    disabled={disabled || !on}
                    value={String(x.order)}
                    onChange={(e) => setType(b, encodeXover({ ...x, order: Number((e.target as HTMLSelectElement).value) }))}
                  >
                    {on ? (
                      (FAMILY_ORDERS[x.family] ?? []).map((o) => (
                        <option key={o} value={o}>
                          {compact ? slopeText(o).replace("/oct", "") : slopeText(o)}
                        </option>
                      ))
                    ) : (
                      <option value={String(x.order)}>–</option>
                    )}
                  </select>
                </td>
                <td class="c-freq">
                  <ValueField
                    label={`${n} frequency`}
                    text={on ? (compact ? shortFreq(p.freq) : freqText(p.freq)) : "–"}
                    parse={parseFreq}
                    disabled={disabled || !on}
                    onCommit={(v) => void session.write("eq.freq", { float: roundFreq(clamp(v, props.freq)) }, [wire, XOVER_BASE + b])}
                  />
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
            <button type="button" disabled={disabled || allBypassed} onClick={() => void askBypass()}>
              Bypass All
            </button>
          </span>
        ) : null}
        <button type="button" disabled={disabled || !armed.length} onClick={() => void clearAll()}>
          Clear All
        </button>
      </div>
    </section>
  );
});
