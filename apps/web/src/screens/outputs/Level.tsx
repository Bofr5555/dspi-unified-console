// A level or delay control (WP WB6c): the Console's card section (a caption,
// a value to read and type, a slider under it), with the owner's rule for the
// delay (SL1), which the gain follows on its own 0.1 dB grid:
//
// - a drag sends its positions live and its release commits, both on the
//   grid (whole ms, never -0), and the thumb then shows what was committed,
//   even from a fractional start;
// - a typed value is sent as typed (the core keeps 0.0001 ms), and a value
//   from the device, a file or another client is shown as it is;
// - the wheel and the keys step to the next grid value up or down (from
//   12.3 ms: 13 or 12);
// - a key or an assistive technology's change commits at once, as a release
//   does, not only live.
//
// Writes go through the slider pump (`useLiveWrite`): the newest wins, each
// read back before the next.

import { useRef } from "preact/hooks";
import { ValueField } from "../eq/Field";
import { useLiveWrite } from "../main/live";
import { clamp, onGrid, parseLevel, stepGrid } from "./model";

export function LevelControl(props: {
  /** The caption: "Gain", "Delay". */
  label: string;
  /** What a screen reader hears for the controls: "OUT1 Front L delay". */
  name: string;
  unit: string;
  /** The device's value. */
  value: number;
  /** What may be typed (the registry's or the device's range). */
  range: readonly [number, number];
  /** The slider's span, inside `range`. */
  slider: readonly [number, number];
  /** The grid the slider, the wheel and the keys land on. */
  step: number;
  /** Ten steps for Page Up and Down. */
  page?: number;
  text: (v: number) => string;
  write: (v: number) => Promise<boolean>;
  disabled: boolean;
  /** A double or secondary click on the slider puts it back here (`onReset`). */
  reset?: number;
  class?: string;
}) {
  const { step, disabled } = props;
  const live = useLiveWrite(props.write);
  const shown = live.draft ?? props.value;
  const dragging = useRef(false);
  const range = props.range;

  const raw = (e: Event) => Number((e.target as HTMLInputElement).value);
  const commit = (v: number) => {
    if (!disabled) live.release(v);
  };
  const wheel = (e: WheelEvent) => {
    if (disabled || e.deltaY === 0) return;
    e.preventDefault();
    commit(stepGrid(shown, e.deltaY < 0 ? 1 : -1, step, range));
  };
  const key = (e: KeyboardEvent) => {
    if (disabled) return;
    const page = props.page ?? 10;
    let v: number | null = null;
    switch (e.key) {
      case "ArrowUp":
      case "ArrowRight":
        v = stepGrid(shown, 1, step, props.slider);
        break;
      case "ArrowDown":
      case "ArrowLeft":
        v = stepGrid(shown, -1, step, props.slider);
        break;
      case "PageUp":
        v = stepGrid(shown, 1, step, props.slider, page);
        break;
      case "PageDown":
        v = stepGrid(shown, -1, step, props.slider, page);
        break;
      case "Home":
        v = props.slider[0];
        break;
      case "End":
        v = props.slider[1];
        break;
    }
    if (v === null) return;
    e.preventDefault();
    commit(v);
  };
  const reset = (e: Event) => {
    e.preventDefault();
    if (props.reset !== undefined) commit(props.reset);
  };

  return (
    <div class={`level${props.class ? ` ${props.class}` : ""}`} onWheel={wheel}>
      <div class="level-head">
        <span class="level-caption" aria-hidden="true">
          {props.label}
        </span>
        <ValueField
          label={props.name}
          text={props.text(shown)}
          parse={(t) => parseLevel(t, props.unit.toLowerCase())}
          disabled={disabled}
          class="level-field"
          onCommit={(v) => commit(clamp(v, range))}
        />
      </div>
      <input
        type="range"
        class="level-slider"
        aria-label={props.name}
        aria-valuetext={props.text(shown)}
        min={props.slider[0]}
        max={props.slider[1]}
        step={step}
        value={clamp(shown, props.slider)}
        disabled={disabled}
        onPointerDown={() => {
          dragging.current = true;
          // Wherever the pointer is let go.
          const end = () => {
            dragging.current = false;
            window.removeEventListener("pointerup", end);
            window.removeEventListener("pointercancel", end);
          };
          window.addEventListener("pointerup", end);
          window.addEventListener("pointercancel", end);
        }}
        onInput={(e) => {
          const v = onGrid(raw(e), step);
          // A drag sends live; anything else (an assistive technology) commits.
          if (dragging.current) live.move(v);
          else commit(v);
        }}
        onChange={(e) => {
          dragging.current = false;
          commit(onGrid(raw(e), step));
        }}
        onKeyDown={key}
        onDblClick={reset}
        onContextMenu={reset}
      />
    </div>
  );
}
