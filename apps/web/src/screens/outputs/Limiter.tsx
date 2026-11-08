// An output's limiter (WP WB6c, B12 and M52), after the Console's
// `OutputLimiterView`: on the output card a toggle (the Console's click) and a
// settings button (its secondary click), which opens the settings: on and
// off, threshold, release, the link group with its summary, "Copy to all
// outputs" and the "All outputs" actions, in the Console's words. The link
// groups are the core's: a write to one member moves the group, and the
// read-back shows it.

import { Dialog } from "../main/Dialog";
import { Menu } from "../main/Menu";
import type { DspState } from "../../core/protocol";
import type { Session, Write } from "../../ui/session";
import { LevelControl } from "./Level";
import { LINK_GROUPS, levelText, linkSummary, stereoPairGroups } from "./model";

/** `limit.*` to every output at once. */
const ALL = 255;
const THRESHOLD: [number, number] = [-30, 0];
const RELEASE: [number, number] = [10, 1000];

export function LimiterButtons(props: { session: Session; s: DspState; output: number; name: string; disabled: boolean; open: () => void }) {
  const { session, s, output } = props;
  const on = s.limiterEnabled[output] ?? false;
  const ceiling = s.limiterThresholdDb[output] ?? -1;
  return (
    <span class="limiter-buttons" role="group" aria-label="Output limiter">
      <button
        type="button"
        class="toggle limiter-toggle"
        aria-pressed={on}
        aria-label={`Limiter ${props.name}`}
        title={
          on
            ? `Output limiter on, ceiling ${ceiling.toFixed(1)} dBFS. Click to switch off, right-click for settings.`
            : "Output limiter off. Click to switch on, right-click for settings."
        }
        disabled={props.disabled}
        onClick={() => void session.write("limit.on", { bool: !on }, [output])}
        onContextMenu={(e) => {
          e.preventDefault();
          props.open();
        }}
      >
        Limiter
      </button>
      <button type="button" class="limiter-more" aria-label={`Limiter settings, ${props.name}`} aria-haspopup="dialog" onClick={props.open}>
        <span aria-hidden="true">…</span>
      </button>
    </span>
  );
}

export function LimiterDialog(props: { session: Session; s: DspState; output: number; names: string[]; disabled: boolean; onClose: () => void }) {
  const { session, s, output, disabled } = props;
  const on = s.limiterEnabled[output] ?? false;
  const group = s.limiterLinkGroup[output] ?? 0;
  const name = props.names[output] ?? `Output ${output + 1}`;
  const others = group ? props.names.filter((_, o) => o !== output && (s.limiterLinkGroup[o] ?? 0) === group) : [];
  const off = disabled || !on;
  const writeAll = (w: Write[]) => void session.writeAll(w);
  return (
    <Dialog title="Output Limiter" onCancel={props.onClose} buttons={[{ label: "Done", onClick: props.onClose }]}>
      <div class="limiter">
        <p class="limiter-name">{name}</p>
        <label
          class="limiter-switch"
          title="A test signal from the Signal Generator is limited like any other signal. Switch this output's limiter off for an unaltered full-scale measurement."
        >
          <input type="checkbox" role="switch" checked={on} disabled={disabled} onChange={(e) => void session.write("limit.on", { bool: (e.target as HTMLInputElement).checked }, [output])} />
          Limiter on
        </label>
        <div class={off ? "limiter-params off" : "limiter-params"}>
          <LevelControl
            label="Threshold"
            name="Threshold"
            unit="dB"
            value={s.limiterThresholdDb[output] ?? -1}
            range={THRESHOLD}
            slider={THRESHOLD}
            step={0.5}
            page={6}
            text={(v) => levelText(v)}
            write={(v) => session.write("limit.threshold", { float: v }, [output])}
            disabled={off}
            reset={-1}
          />
          <p class="note">The ceiling, in dBFS. No sample leaves this output above it.</p>
          <p class="note limiter-ends" aria-hidden="true">
            <span>-30 dBFS</span>
            <span>0 dBFS</span>
          </p>
          <LevelControl
            label="Release"
            name="Release"
            unit="ms"
            value={s.limiterReleaseMs[output] ?? 100}
            range={RELEASE}
            slider={RELEASE}
            step={10}
            page={10}
            text={(v) => `${Math.round(v)} ms`}
            write={(v) => session.write("limit.release", { float: v }, [output])}
            disabled={off}
            reset={100}
          />
          <p class="note">How fast the gain recovers after a peak</p>
          <p class="note limiter-ends" aria-hidden="true">
            <span>10 ms</span>
            <span>1000 ms</span>
          </p>
        </div>
        <div class="limiter-link" role="radiogroup" aria-label="Link group">
          <span class="level-caption">Link group</span>
          {Array.from({ length: LINK_GROUPS + 1 }, (_, g) => (
            <button
              type="button"
              key={g}
              role="radio"
              aria-checked={g === group}
              aria-label={g === 0 ? "Not linked" : `Link group ${g}`}
              disabled={disabled}
              onClick={() => g !== group && void session.write("limit.link", { int: g }, [output])}
            >
              {g === 0 ? "Off" : g}
            </button>
          ))}
        </div>
        <p class="note limiter-summary" aria-live="polite">
          {linkSummary(group, others)}
        </p>
        <div class="limiter-batch">
          <button
            type="button"
            disabled={disabled}
            title="Give every output this output's threshold, release and on/off state. Link groups are left as they are."
            onClick={() =>
              writeAll([
                { path: "limit.threshold", value: { float: s.limiterThresholdDb[output] ?? -1 }, indices: [ALL] },
                { path: "limit.release", value: { float: s.limiterReleaseMs[output] ?? 100 }, indices: [ALL] },
                { path: "limit.on", value: { bool: on }, indices: [ALL] },
              ])
            }
          >
            Copy to all outputs
          </button>
          <Menu
            label="All outputs"
            disabled={disabled}
            entries={[
              {
                label: "Link all stereo pairs",
                onSelect: () => writeAll(stereoPairGroups(s).map((g, o) => ({ path: "limit.link", value: { int: g }, indices: [o] }))),
              },
              { label: "Unlink all outputs", onSelect: () => writeAll([{ path: "limit.link", value: { int: 0 }, indices: [ALL] }]) },
              "separator",
              { label: "Switch every limiter off", onSelect: () => writeAll([{ path: "limit.on", value: { bool: false }, indices: [ALL] }]) },
            ]}
          />
        </div>
      </div>
    </Dialog>
  );
}
