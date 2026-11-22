// -- WP WEBM2 -- An output's high and low pass (MODLAY 2): the Crossover tab's row and the
// inspector's Crossover part. Each pass is one choice (Off, or a family and slope) and its
// frequency, over the output's four crossover bands: a new pass takes the first band that is off,
// Off clears the band as the XO list's Clear All does. Writes are the XO list's: the core's
// `crossoverType`, and `eq.freq` on band 20 + n; every change is read back.

import type { FilterParams } from "../../core/protocol";
import type { Session } from "../../ui/session";
import { XOVER_BASE } from "../outputs/model";
import { freeBand, passChoices, passes, setPass, type Pass } from "./align";
import { ValueField } from "./Field";
import { clamp, freqText, parseFreq, roundFreq } from "./model";

/** What Off writes to the band (`clearCrossover`): off, 1 kHz. */
const CLEARED_FREQ = 1000;

const HP = passChoices(true);
const LP = passChoices(false);

export function PassFields(props: {
  session: Session;
  /** The output's wire channel. */
  wire: number;
  bands: readonly FilterParams[];
  freq: [number, number];
  disabled: boolean;
  /** "High pass" / "Low pass" are prefixed with it ("OUT1 Left high pass"); none in the inspector. */
  who?: string;
  /** How the parts are laid out: cells of a table row, or rows of the inspector. */
  as: "cells" | "rows";
}) {
  const { session, wire, bands, disabled } = props;
  const both = passes(bands);
  const full = freeBand(bands) === null;
  const choose = (highPass: boolean, type: number) => {
    const c = setPass(bands, highPass, type);
    if (!c) return;
    void session.change(
      "crossoverType",
      c.filterType === 0 ? { channel: wire, band: c.band, filterType: 0, freq: CLEARED_FREQ } : { channel: wire, band: c.band, filterType: c.filterType },
    );
  };
  const part = (highPass: boolean, pass: Pass | null) => {
    const kind = highPass ? "high pass" : "low pass";
    const label = props.who ? `${props.who} ${kind}` : kind[0]!.toUpperCase() + kind.slice(1);
    const choices = highPass ? HP : LP;
    // A pass the list does not offer (another family's odd order): shown as it is.
    const listed = !pass || choices.some((c) => c.value === pass.filterType);
    const select = (
      <select
        aria-label={label}
        disabled={disabled || (!pass && full)}
        title={!pass && full ? "All four crossover bands are in use" : undefined}
        value={String(pass?.filterType ?? 0)}
        onChange={(e) => choose(highPass, Number((e.target as HTMLSelectElement).value))}
      >
        {listed ? null : <option value={String(pass!.filterType)}>{`Type ${pass!.filterType}`}</option>}
        {choices.map((c) => (
          <option key={c.value} value={String(c.value)}>
            {c.label}
          </option>
        ))}
      </select>
    );
    const field = (
      <ValueField
        label={`${label} frequency`}
        text={pass ? freqText(pass.freq) : "–"}
        parse={parseFreq}
        disabled={disabled || !pass}
        onCommit={(v) => pass && void session.write("eq.freq", { float: roundFreq(clamp(v, props.freq)) }, [wire, XOVER_BASE + pass.band])}
      />
    );
    const off = pass?.bypass ? <span class="pass-bypassed">bypassed</span> : null;
    if (props.as === "cells")
      return (
        <>
          <td class="c-pass">
            {select}
            {off}
          </td>
          <td class="c-freq">{field}</td>
        </>
      );
    return (
      <>
        <div class="insp-row">
          <span class="insp-label">{label}</span>
          {select}
        </div>
        {pass ? (
          <div class="insp-row">
            <span class="insp-label">Frequency{off ? " (bypassed)" : ""}</span>
            {field}
          </div>
        ) : null}
      </>
    );
  };
  return (
    <>
      {part(true, both.hp)}
      {part(false, both.lp)}
    </>
  );
}
