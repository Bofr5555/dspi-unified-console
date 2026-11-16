// WP P3OV: one output's row of the Overview: its name, what is routed into it,
// its gain (typed, or a half-dB step with − and +) and mute edited inline
// through the session's writes (`out.gain`, `out.mute`: the core's guards,
// limits and undo, as on the output card; read only, nothing is editable), its
// delay, its limiter's threshold and gain reduction, its clip marker and its
// meter. A table row at desktop width; the stylesheet makes it a card at a
// phone's width (each cell says what it is there).

import { memo } from "preact/compat";
import type { Session } from "../../ui/session";
import { ValueField } from "../eq/Field";
import { compactGain, parseLevel } from "../outputs/model";
import { MeterBar } from "./Meters";
import { gainStep, gainTyped, grText, routingText, valueTexts, type MeterReading, type OverviewRow, type OverviewUnits } from "./model";

export const OutputRow = memo(function OutputRow(props: {
  session: Session;
  row: OverviewRow;
  units: OverviewUnits;
  meter: MeterReading;
  /** The limiters' gain reduction, as read; null when not read. */
  gr: number[] | null;
  stale: boolean;
  disabled: boolean;
}) {
  const { session, row, units, disabled } = props;
  const t = valueTexts(row, units);
  const who = `${row.descriptor} ${row.name}`;
  const range = units.gain.range;
  const setGain = (v: number) => void session.write("out.gain", { float: v }, [row.index]);
  const gr = grText(row, props.gr);
  return (
    <tr class={`ov-row${row.enabled ? "" : " ov-off"}`} role="row" aria-label={who} data-output={row.index} data-enabled={row.enabled}>
      <th scope="row" role="rowheader" class="ov-name" title={row.slot || undefined}>
        <span class="ov-kind">{row.descriptor}</span> <span class="ov-label">{row.name}</span>
        {row.enabled ? null : <span class="ov-state"> Disabled</span>}
      </th>
      <td role="cell" class="ov-routing" data-label="Routing" aria-label={`Routing of ${row.descriptor}`} title={routingText(row)}>
        {row.sources.length ? (
          row.sources.map((s) => (
            <span key={s.descriptor} class="ov-src">
              {s.name} <span class="ov-src-gain">{compactGain(s.gainDb)}</span>
              {s.invert ? <span class="ov-src-inv"> inv</span> : null}
            </span>
          ))
        ) : (
          <span class="ov-none">None</span>
        )}
      </td>
      <td role="cell" class="ov-gain" data-label={`Gain (${units.gain.unit})`}>
        <ValueField
          label={`${who} gain`}
          text={t.gain}
          parse={(x) => parseLevel(x, units.gain.unit.toLowerCase())}
          disabled={disabled}
          class="ov-gain-field"
          step={{ down: () => setGain(gainStep(row.gainDb, -1, range)), up: () => setGain(gainStep(row.gainDb, 1, range)) }}
          onCommit={(v) => setGain(gainTyped(v, range))}
        />
      </td>
      <td role="cell" class="ov-delay" data-label={`Delay (${units.delay.unit})`} aria-label={`${who} delay`}>
        {t.delay}
      </td>
      <td role="cell" class="ov-mute" data-label="Mute">
        <button
          type="button"
          class="toggle ov-mute-btn"
          aria-pressed={row.muted}
          aria-label={`${who} mute`}
          disabled={disabled}
          onClick={() => void session.write("out.mute", { bool: !row.muted }, [row.index])}
        >
          {row.muted ? "Muted" : "Mute"}
        </button>
      </td>
      <td role="cell" class="ov-limit" data-label={`Limiter (${units.threshold.unit})`} aria-label={`${who} limiter`}>
        {t.threshold}
      </td>
      <td role="cell" class="ov-gr" data-label="Reduction" aria-label={`${who} gain reduction`}>
        {gr}
      </td>
      <td role="cell" class="ov-clip" data-label="Clip" aria-label={`${who} clip`} data-clip={props.meter.clip}>
        {props.meter.clip ? <span class="ov-clip-on">Clip</span> : <span class="ov-clip-off">OK</span>}
      </td>
      <td role="cell" class="ov-level" data-label="Level">
        <MeterBar name={who} r={props.meter} stale={props.stale} />
      </td>
    </tr>
  );
});

