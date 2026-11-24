// A band value to read and type (WP WB6b): the band list's and the chip's
// frequency, gain and Q. It shows the band's value (live during a drag) until
// it has focus; then what is typed stays until Enter or leaving it writes it
// (Escape puts the value back). Frequencies take "2k" and note names ("A4").
// Optional − and + buttons step it, for touch.
//
// -- WP WEB6 -- (REVE C1) Leaving a field writes only what was typed: Tab through it, or text typed
// back to what it shows, writes nothing (Qt's BorderedField; the rounded text written back once
// moved a 4.1234 ms delay to 4.1108 ms). `unit` is drawn after the field, as Qt's.

import { useEffect, useRef, useState } from "preact/hooks";

/** A narrow screen (a phone): the band list shows its values in short. */
export function useCompact(): boolean {
  const query = typeof matchMedia === "function" ? matchMedia("(max-width: 520px)") : null;
  const [compact, setCompact] = useState(query?.matches ?? false);
  useEffect(() => {
    if (!query) return;
    const on = () => setCompact(query.matches);
    query.addEventListener?.("change", on);
    return () => query.removeEventListener?.("change", on);
  }, []);
  return compact;
}

export function ValueField(props: {
  label: string;
  /** The value as text (`freqText`, `gainText`, `qText`). */
  text: string;
  /** Typed text to a value, or null when it is not one. */
  parse: (s: string) => number | null;
  onCommit: (v: number) => void;
  disabled?: boolean;
  /** Steppers: the value one step down and up. */
  step?: { down: () => void; up: () => void };
  class?: string;
  /** Focus requests from the graph (a double click on a dot). */
  inputRef?: (el: HTMLInputElement | null) => void;
  /** -- WP WEB6 -- the unit after the field ("Hz", "dB", "Q"). */
  unit?: string;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const [bad, setBad] = useState(false);
  const skip = useRef(false);
  // -- WP WEB6 -- (REVE C1) whether anything was typed since the field took the focus
  const typed = useRef(false);
  const commit = () => {
    if (draft === null) return;
    if (!typed.current || draft === props.text) {
      setBad(false);
      setDraft(null);
      return;
    }
    const v = props.parse(draft);
    if (v === null) {
      setBad(true);
      return;
    }
    setBad(false);
    setDraft(null);
    props.onCommit(v);
  };
  return (
    <span class={`value-field${props.class ? ` ${props.class}` : ""}`}>
      {props.step ? (
        <button type="button" class="step" aria-label={`${props.label} down`} disabled={props.disabled} onClick={props.step.down}>
          −
        </button>
      ) : null}
      <input
        ref={props.inputRef}
        type="text"
        inputMode="decimal"
        autoComplete="off"
        spellcheck={false}
        aria-label={props.label}
        aria-invalid={bad || undefined}
        disabled={props.disabled}
        value={draft ?? props.text}
        onFocus={(e) => {
          typed.current = false;
          setDraft(props.text);
          (e.target as HTMLInputElement).select();
        }}
        onInput={(e) => {
          typed.current = true;
          setDraft((e.target as HTMLInputElement).value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            skip.current = true;
            setBad(false);
            setDraft(null);
            (e.target as HTMLInputElement).blur();
          }
        }}
        onBlur={() => {
          if (skip.current) {
            skip.current = false;
            return;
          }
          commit();
          setDraft(null);
          setBad(false);
        }}
      />
      {props.unit ? (
        <span class="field-unit" aria-hidden="true">
          {props.unit}
        </span>
      ) : null}
      {props.step ? (
        <button type="button" class="step" aria-label={`${props.label} up`} disabled={props.disabled} onClick={props.step.up}>
          +
        </button>
      ) : null}
    </span>
  );
}
