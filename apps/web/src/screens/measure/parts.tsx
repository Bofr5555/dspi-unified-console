// -- WP WEB4: move to ui/ -- Pieces of DL0/DL2's language the web's ui/design.tsx does not have yet,
// used by the Measure pages: a status chip (§13: the control tone, a 6 px dot, small text) and the
// auto-tune's step dots (DL2's StepDots: Layout · Proposal · Applied).

import type { ComponentChildren } from "preact";
import { Glyph } from "../../ui/design";

/** Lucide strokes ui/design.tsx's `Glyph` does not have (yet); the rest are its. */
const MORE: Record<string, ComponentChildren> = {
  check: <path d="M20 6 9 17l-5-5" />,
  "circle-check": (
    <>
      <circle cx="12" cy="12" r="10" />
      <path d="m9 12 2 2 4-4" />
    </>
  ),
  lock: (
    <>
      <rect width="18" height="11" x="3" y="11" rx="2" ry="2" />
      <path d="M7 11V7a5 5 0 0 1 10 0v4" />
    </>
  ),
  "undo-2": <path d="M9 14 4 9l5-5M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5 5.5 5.5 0 0 1-5.5 5.5H11" />,
  "redo-2": <path d="m15 14 5-5-5-5M20 9H9.5A5.5 5.5 0 0 0 4 14.5 5.5 5.5 0 0 0 9.5 20H13" />,
  "file-up": (
    <>
      <path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" />
      <path d="M14 2v4a2 2 0 0 0 2 2h4M12 12v6M15 15l-3-3-3 3" />
    </>
  ),
  save: (
    <>
      <path d="M15.2 3a2 2 0 0 1 1.4.6l3.8 3.8a2 2 0 0 1 .6 1.4V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z" />
      <path d="M17 21v-7a1 1 0 0 0-1-1H8a1 1 0 0 0-1 1v7M7 3v4a1 1 0 0 0 1 1h7" />
    </>
  ),
  "list-x": <path d="M11 12H3M16 6H3M16 18H3M18 9l3 3M21 9l-3 3" />,
  timer: (
    <>
      <path d="M10 2h4M12 14l3-3" />
      <circle cx="12" cy="14" r="8" />
    </>
  ),
};

/** A Lucide icon: `MORE`'s, else ui/design.tsx's `Glyph`. */
export function Icon(props: { name: string; class?: string }) {
  const d = MORE[props.name];
  if (!d) return <Glyph name={props.name} class={props.class} />;
  return (
    <svg class={`shell-icon${props.class ? ` ${props.class}` : ""}`} viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      {d}
    </svg>
  );
}

/** §13: a pill with a 6 px dot in the state's colour and its words. */
export function StatusChip(props: { kind: "ok" | "idle" | "warn" | "bad"; children: ComponentChildren }) {
  return (
    <span class={`status-chip status-chip-${props.kind}`}>
      <span class={`status-dot${props.kind === "idle" ? "" : ` ${props.kind}`}`} aria-hidden="true" />
      {props.children}
    </span>
  );
}

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
