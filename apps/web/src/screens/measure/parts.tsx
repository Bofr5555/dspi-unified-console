// -- WP WEB4 -- DL2's step dots for the auto-tune (Layout · Proposal · Applied). The icons and the
// status chip that were here are ui/design.tsx's `Glyph` and `StatusChip` now (INTD).

import { useState } from "preact/hooks";

/** DL2's step dots: the steps in a row, the reached ones in the accent, the current one named. */
export function StepDots(props: { steps: readonly string[]; current: number; label: string }) {
  return (
    <ol class="step-dots" aria-label={props.label}>
      {props.steps.map((s, i) => (
        <li key={s} class={i < props.current ? "done" : i === props.current ? "now" : undefined} aria-current={i === props.current ? "step" : undefined}>
          <span class="step-dot" aria-hidden="true" />
          <span class="step-name">{s}</span>
        </li>
      ))}
    </ol>
  );
}

// -- WP WEB7 -- Qt's AppSpinBox on the Measure pages: "−  value unit  +" on the control tone, for the
// integers (passes, positions, groups) and the sweep length Qt steps the same way. The field takes a
// typed number too (`onValue` as it is typed, as the plain fields did; `onCommit` when it is left).
// The − and + are for the pointer; the field is the spin button (arrow keys step it), so they are
// hidden from assistive technology rather than read as two more controls.


export function Stepper(props: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  decimals?: number;
  unit?: string;
  disabled?: boolean;
  /** Each value as it is typed or stepped. */
  onValue: (v: number) => void;
  /** A value settled: stepped, or the field left after typing. */
  onCommit?: (v: number) => void;
  class?: string;
}) {
  const step = props.step ?? 1;
  const dec = props.decimals ?? 0;
  const [draft, setDraft] = useState<string | null>(null);
  const clamp = (v: number) => Math.min(props.max, Math.max(props.min, v));
  const round = (v: number) => Number(v.toFixed(dec));
  const by = (k: number) => {
    const v = clamp(round(props.value + k * step));
    setDraft(null);
    props.onValue(v);
    props.onCommit?.(v);
  };
  const shown = draft ?? (Number.isFinite(props.value) ? props.value.toFixed(dec) : "");
  return (
    <span class={`m-stepper${props.disabled ? " disabled" : ""}${props.class ? ` ${props.class}` : ""}`}>
      <button type="button" class="m-step" tabIndex={-1} aria-hidden="true" disabled={props.disabled || props.value <= props.min} onClick={() => by(-1)}>
        −
      </button>
      <input
        type="text"
        inputMode={dec ? "decimal" : "numeric"}
        role="spinbutton"
        aria-label={props.label}
        aria-valuemin={props.min}
        aria-valuemax={props.max}
        aria-valuenow={Number.isFinite(props.value) ? props.value : undefined}
        disabled={props.disabled}
        value={shown}
        onFocus={(e) => (e.currentTarget as HTMLInputElement).select()}
        onInput={(e) => {
          const t = (e.currentTarget as HTMLInputElement).value;
          setDraft(t);
          const v = Number(t.replace(",", ".").replace("−", "-"));
          if (t.trim() !== "" && Number.isFinite(v)) props.onValue(v);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowUp") (e.preventDefault(), by(1));
          else if (e.key === "ArrowDown") (e.preventDefault(), by(-1));
          else if (e.key === "Enter") (e.currentTarget as HTMLInputElement).blur();
        }}
        onBlur={() => {
          if (draft === null) return;
          setDraft(null);
          const v = Number(draft.replace(",", ".").replace("−", "-"));
          if (draft.trim() !== "" && Number.isFinite(v)) props.onCommit?.(v);
        }}
      />
      {props.unit ? <span class="m-step-unit" aria-hidden="true">{props.unit}</span> : null}
      <button type="button" class="m-step" tabIndex={-1} aria-hidden="true" disabled={props.disabled || props.value >= props.max} onClick={() => by(1)}>
        +
      </button>
    </span>
  );
}
