// -- WP INTD -- A popover (DESIGN-LANGUAGE §5, Qt's PopoverPanel): a button and, while open, a
// floating surface under it with its own parts (the top bar's presets, Snapshots' Capture). A
// dialog without a modal scrim: its first control takes the focus, Escape or a press outside
// closes it, and the focus goes back to its button. `children` gets `close` for an action that is
// done with it.

import type { ComponentChildren } from "preact";
import { useLayoutEffect, useRef, useState } from "preact/hooks";

export function Popover(props: {
  /** The dialog's name ("Presets", "Capture Snapshot"). */
  label: string;
  /** The button's face; its accessible name is `buttonLabel`, else the face's text. */
  button: ComponentChildren;
  buttonLabel?: string;
  buttonClass?: string;
  title?: string;
  disabled?: boolean;
  /** Where the surface lines up with its button: its left edge (the default) or its right. */
  align?: "left" | "right";
  class?: string;
  children: (close: (refocus?: boolean) => void) => ComponentChildren;
}) {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const close = (refocus = true) => {
    setOpen(false);
    if (refocus) button.current?.focus();
  };
  useLayoutEffect(() => {
    if (!open) return;
    const first = panel.current?.querySelector<HTMLElement>("[data-autofocus], input:not([disabled]), button:not([disabled]), select:not([disabled])");
    (first ?? panel.current)?.focus();
    const outside = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!panel.current?.contains(t) && !button.current?.contains(t)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);
  return (
    <div class={`popover-anchor${props.class ? ` ${props.class}` : ""}`}>
      <button
        ref={button}
        type="button"
        class={props.buttonClass}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={props.buttonLabel}
        title={props.title}
        disabled={props.disabled}
        onClick={() => setOpen(!open)}
      >
        {props.button}
      </button>
      {open ? (
        <div
          ref={panel}
          class={`popover${props.align === "right" ? " popover-right" : ""}`}
          role="dialog"
          aria-label={props.label}
          tabIndex={-1}
          onKeyDown={(e) => {
            // A menu inside closes itself first (it stops its own Escape)
            if (e.key !== "Escape" || e.defaultPrevented) return;
            e.preventDefault();
            e.stopPropagation();
            close();
          }}
        >
          {props.children(close)}
        </div>
      ) : null}
    </div>
  );
}
