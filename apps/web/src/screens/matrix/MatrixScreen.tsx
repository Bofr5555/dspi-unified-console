// The matrix mixer of one session (WP WB6c, B10), after the Console's
// `MatrixMixerView`: the live inputs down, every output across (its name, its
// descriptor and its slot), a crosspoint per cell (connect, level, polarity),
// and per output ENABLE, GAIN, DELAY and MUTE. Enabling an output that shares
// Core 1 with the PDM sub asks first, in the Console's words, and switches
// over as the Console does. A column's menu renames the output.
//
// Every change is the core's, read back: crosspoints by `crosspoint` (the
// registry's `mix`), the rest by `write`. The delay follows the owner's rule
// (SL1): typed values kept, device values shown as they are, the wheel to
// the next whole ms. On a phone the grid scrolls sideways inside its card,
// never the page. A read-only session shows it all, the controls off.

import { memo } from "preact/compat";
import { useEffect, useState } from "preact/hooks";
import type { DspState, OutputLabel } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { Ask, ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { ValueField } from "../eq/Field";
import { channelColour } from "../eq/model";
import { RenameDialog } from "../main/Dialog";
import { useLiveWrite } from "../main/live";
import { Menu } from "../main/Menu";
import { inputRows, liveInputs, outputLabel, type ChannelRow } from "../main/model";
import { CROSSPOINT_RANGE } from "../outputs/OutputCard";
import { clamp, compactGain, delayText, enableConflict, eqWorkerRange, outputSlotName, parseLevel, pdmOutput, stepGrid } from "../outputs/model";

export function MatrixScreen(props: ScreenProps) {
  const { session, ask } = props;
  const st = useSessionState(session);
  const s = st.snapshot?.state;
  const ro = session.readOnlyReason !== "";

  const [usbActive, setUsbActive] = useState<number | null>(null);
  const [maxDelay, setMaxDelay] = useState(21);
  useEffect(() => {
    let alive = true;
    session.core.call("meters", undefined).then(
      (m) => alive && setUsbActive(m.activeInputChannels),
      () => undefined,
    );
    session.core.call("outputLimits", undefined).then(
      (l) => alive && setMaxDelay(l.maxDelayMs),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [session]);

  if (!s || !Array.isArray(s.matrixRouting) || !s.matrixRouting.length) return <p class="empty">This DSPi's matrix is not known yet.</p>;
  return <MatrixView session={session} ask={ask} s={s} ro={ro} usbActive={usbActive} maxDelay={maxDelay} labels={st.snapshot?.outputLabels} />;
}

const MatrixView = memo(
  // -- BUG1 -- labels: the outputs' labels, the snapshot's
  function MatrixView(props: { session: Session; ask: Ask; s: DspState; ro: boolean; usbActive: number | null; maxDelay: number; labels?: OutputLabel[] }) {
    const { session, ask, s, ro } = props;
    const inputs = inputRows(s, liveInputs(s, props.usbActive));
    const outs = Array.from({ length: s.numOutputChannels }, (_, o) => ({
      index: o,
      wire: s.numInputChannels + o,
      descriptor: `OUT${o + 1}`,
      name: outputLabel(s, props.labels, o),
      stored: s.channelNames[s.numInputChannels + o] ?? `Output ${o + 1}`,
      slot: props.labels?.[o]?.slotName || outputSlotName(s, o),
      enabled: s.outputEnabled[o] ?? false,
    }));
    const [renaming, setRenaming] = useState<(typeof outs)[number] | null>(null);
    const crosspoint = (i: number, o: number, p: { enabled?: boolean; gain?: number; invert?: boolean }) =>
      session.change("crosspoint", {
        input: i,
        output: o,
        enabled: p.enabled ?? s.matrixRouting[i]?.[o] ?? false,
        gain: p.gain ?? s.matrixGain[i]?.[o] ?? 0,
        invert: p.invert ?? s.matrixInvert[i]?.[o] ?? false,
      });

    const enable = async (o: number) => {
      if (s.outputEnabled[o]) return void session.write("out.enable", { bool: false }, [o]);
      const conflict = enableConflict(s, o);
      if (!conflict) return void session.write("out.enable", { bool: true }, [o]);
      const [a, b] = eqWorkerRange(s);
      const pdm = conflict === "pdm";
      const answer = await ask({
        title: "Warning",
        body: <p>{pdm ? `Outputs ${a + 1}-${b + 1} will be disabled. Are you sure?` : "The PDM output will be disabled. Are you sure?"}</p>,
        buttons: [
          { label: pdm ? "Enable PDM" : "Disable PDM", value: "go" },
          { label: "Cancel", value: "cancel" },
        ],
      });
      if (answer !== "go") return;
      // As the Console switches over (`switchToPDM`, `switchFromPDM`): the others off first.
      const off = pdm ? Array.from({ length: b - a + 1 }, (_, k) => a + k) : [pdmOutput(s)];
      await session.writeAll([...off.map((x) => ({ path: "out.enable", value: { bool: false }, indices: [x] })), { path: "out.enable", value: { bool: true }, indices: [o] }]);
    };

    // The 8-channel mode's actions (`Direct 1:1`, `Clear`).
    const many = inputs.length > 2;
    // -- WP P1WB2 -- each one undo step (P1UG)
    const direct = () => session.group("Direct 1:1", async () => {
      for (const i of inputs) {
        for (const o of outs) {
          const want = o.index === i.index && o.index !== pdmOutput(s);
          if ((s.matrixRouting[i.index]?.[o.index] ?? false) !== want && !(await crosspoint(i.index, o.index, { enabled: want }))) return;
        }
      }
      if (s.outputEnabled[pdmOutput(s)]) await session.write("out.enable", { bool: false }, [pdmOutput(s)]);
    });
    const clearAll = () => session.group("Clear routing", async () => {
      for (const i of inputs) for (const o of outs) if (s.matrixRouting[i.index]?.[o.index] && !(await crosspoint(i.index, o.index, { enabled: false }))) return;
    });

    return (
      <div class="matrix-screen">
        <section class="card matrix-card" aria-label="Matrix Mixer">
          <div class="matrix-head">
            <h3>Routing</h3>
            {many ? (
              <span class="matrix-actions">
                <button type="button" disabled={ro} title="Route each input to the matching output (FL→OUT1, FR→OUT2, …) and disable the PDM sub" onClick={() => void direct()}>
                  Direct 1:1
                </button>
                <button type="button" disabled={ro} title="Disconnect every crosspoint" onClick={() => void clearAll()}>
                  Clear
                </button>
              </span>
            ) : null}
          </div>
          <div class="matrix-scroll" role="region" aria-label="Matrix" tabIndex={0}>
            <table class="matrix">
              <thead>
                <tr>
                  <th scope="col" class="m-corner">
                    <span class="visually-hidden">Input</span>
                  </th>
                  {outs.map((o) => (
                    <th scope="col" key={o.index} class={o.enabled ? "m-out" : "m-out disabled"} title={o.slot}>
                      <Menu
                        label={o.name}
                        disabled={ro}
                        entries={[{ label: "Rename…", onSelect: () => setRenaming(o) }]}
                      />
                      <span class="m-desc" style={{ color: channelColour(o.wire) }}>
                        {o.descriptor}
                      </span>
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {inputs.map((i, r) => (
                  <tr key={i.wire} class={dividerAfter(r, inputs.length) ? "m-divide" : undefined}>
                    <th scope="row" class="m-in">
                      <span class="m-desc" style={{ color: channelColour(i.wire) }}>
                        {i.descriptor}
                      </span>
                      <span class="m-name">{i.name}</span>
                    </th>
                    {outs.map((o) => (
                      <Cell
                        key={o.index}
                        input={i}
                        out={o}
                        on={s.matrixRouting[i.index]?.[o.index] ?? false}
                        gain={s.matrixGain[i.index]?.[o.index] ?? 0}
                        inv={s.matrixInvert[i.index]?.[o.index] ?? false}
                        disabled={ro}
                        set={(p) => void crosspoint(i.index, o.index, p)}
                      />
                    ))}
                  </tr>
                ))}
              </tbody>
              <tbody class="m-outputs">
                <tr>
                  <th scope="row" class="m-label">
                    ENABLE
                  </th>
                  {outs.map((o) => {
                    const conflict = !o.enabled && enableConflict(s, o.index) !== null;
                    return (
                      <td key={o.index}>
                        <button
                          type="button"
                          class={conflict ? "toggle m-enable conflict" : "toggle m-enable"}
                          aria-pressed={o.enabled}
                          aria-label={`Enable ${o.name}`}
                          title={o.enabled ? "Disable output (saves CPU)" : conflict ? "Conflict: shared Core 1 resource" : "Enable output"}
                          disabled={ro}
                          onClick={() => void enable(o.index)}
                        >
                          <span aria-hidden="true">⏻</span>
                        </button>
                      </td>
                    );
                  })}
                </tr>
                <tr>
                  <th scope="row" class="m-label">
                    GAIN
                  </th>
                  {outs.map((o) => (
                    <td key={o.index} class={o.enabled ? undefined : "disabled"}>
                      <CompactField
                        label={`${o.name} gain`}
                        value={s.outputGainDb[o.index] ?? 0}
                        text={compactGain}
                        unit="db"
                        range={CROSSPOINT_RANGE}
                        step={0.1}
                        disabled={ro}
                        write={(v) => session.write("out.gain", { float: v }, [o.index])}
                      />
                    </td>
                  ))}
                </tr>
                <tr>
                  <th scope="row" class="m-label">
                    DELAY
                  </th>
                  {outs.map((o) => (
                    <td key={o.index} class={o.enabled ? undefined : "disabled"}>
                      <CompactField
                        label={`${o.name} delay`}
                        value={s.outputDelayMs[o.index] ?? 0}
                        text={delayText}
                        unit="ms"
                        range={[0, props.maxDelay]}
                        step={1}
                        disabled={ro}
                        write={(v) => session.write("out.delay", { float: v }, [o.index])}
                      />
                    </td>
                  ))}
                </tr>
                <tr>
                  <th scope="row" class="m-label">
                    MUTE
                  </th>
                  {outs.map((o) => {
                    const muted = s.outputMuted[o.index] ?? false;
                    return (
                      <td key={o.index} class={o.enabled ? undefined : "disabled"}>
                        <button
                          type="button"
                          class="toggle ch-mute"
                          aria-pressed={muted}
                          aria-label={`Mute ${o.name}`}
                          title={muted ? "Unmute" : "Mute"}
                          disabled={ro}
                          onClick={() => void session.write("out.mute", { bool: !muted }, [o.index])}
                        >
                          M
                        </button>
                      </td>
                    );
                  })}
                </tr>
              </tbody>
            </table>
          </div>
        </section>
        {renaming ? (
          <RenameDialog
            title={`Rename ${renaming.name}`}
            label="Name"
            initial={renaming.stored}
            onCancel={() => setRenaming(null)}
            onSave={(name) => {
              const wire = renaming.wire;
              setRenaming(null);
              if (name.trim()) void session.write("ch.name", { text: name }, [wire]);
            }}
          />
        ) : null}
      </div>
    );
  },
  (a, b) => a.session === b.session && a.ask === b.ask && a.ro === b.ro && a.usbActive === b.usbActive && a.maxDelay === b.maxDelay && matrixKey(a.s) === matrixKey(b.s) && JSON.stringify(a.labels) === JSON.stringify(b.labels),
);

/** The Console's dividers: after the first row in stereo, after each pair with more inputs. */
function dividerAfter(r: number, n: number): boolean {
  if (r >= n - 1) return false;
  return n <= 2 ? r === 0 : r % 2 === 1;
}

/** What the matrix shows; the meters and the rest of the state do not reach it. */
function matrixKey(s: DspState): string {
  return JSON.stringify([
    s.matrixRouting,
    s.matrixGain,
    s.matrixInvert,
    s.outputEnabled,
    s.outputGainDb,
    s.outputDelayMs,
    s.outputMuted,
    s.channelNames,
    s.numInputChannels,
    s.numOutputChannels,
    s.inputSource,
    s.i2sInputChannels,
    s.platformId,
    s.outputTypes,
  ]);
}

/** A crosspoint (`MatrixPoint`): its level over the connect dot and its polarity under it, both only when connected. */
const Cell = memo(function Cell(props: {
  input: ChannelRow;
  out: { index: number; name: string; enabled: boolean };
  on: boolean;
  gain: number;
  inv: boolean;
  disabled: boolean;
  set: (p: { enabled?: boolean; gain?: number; invert?: boolean }) => void;
}) {
  const { input, out, on, disabled } = props;
  const where = `${input.name} to ${out.name}`;
  return (
    <td class={out.enabled ? (on ? "m-cell on" : "m-cell") : on ? "m-cell on disabled" : "m-cell disabled"}>
      <span class="m-level">
        {on ? (
          <ValueField
            label={`${where} level`}
            text={compactGain(props.gain)}
            parse={(t) => parseLevel(t, "db")}
            disabled={disabled}
            onCommit={(v) => props.set({ gain: clamp(v, CROSSPOINT_RANGE) })}
          />
        ) : null}
      </span>
      <button
        type="button"
        class="m-point"
        aria-pressed={on}
        aria-label={`Route ${where}`}
        title={on ? "Disconnect" : "Connect"}
        disabled={disabled}
        onClick={() => props.set({ enabled: !on })}
      >
        <span class={on ? "route-dot on" : "route-dot"} aria-hidden="true" />
      </button>
      <span class="m-inv">
        {on ? (
          <button
            type="button"
            class="toggle route-inv"
            aria-pressed={props.inv}
            aria-label={`${where} inverted`}
            title={props.inv ? "Phase inverted" : "Normal phase"}
            disabled={disabled}
            onClick={() => props.set({ invert: !props.inv })}
          >
            INV
          </button>
        ) : null}
      </span>
    </td>
  );
});

/**
 * An output's gain or delay in the matrix (`CompactGainField`,
 * `CompactDelayField`): typed values kept as typed (within the range), the
 * wheel to the next grid value, the newest write winning.
 */
function CompactField(props: {
  label: string;
  value: number;
  text: (v: number) => string;
  unit: string;
  range: [number, number];
  step: number;
  disabled: boolean;
  write: (v: number) => Promise<boolean>;
}) {
  const live = useLiveWrite(props.write);
  const shown = live.draft ?? props.value;
  return (
    <span
      class="m-field"
      onWheel={(e) => {
        if (props.disabled || e.deltaY === 0) return;
        e.preventDefault();
        live.release(stepGrid(shown, e.deltaY < 0 ? 1 : -1, props.step, props.range));
      }}
    >
      <ValueField
        label={props.label}
        text={props.text(shown)}
        parse={(t) => parseLevel(t, props.unit)}
        disabled={props.disabled}
        onCommit={(v) => live.release(clamp(v, props.range))}
      />
    </span>
  );
}
