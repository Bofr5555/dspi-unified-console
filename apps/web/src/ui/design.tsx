// -- WP WEB -- The Modern design language's shared pieces on the web (docs/DESIGN-LANGUAGE.md, as
// DL0 made them in Qt): `EmptyState` (§16) wherever a page has nothing to show, `NoticeRow` (§14)
// for a condition that blocks an action in its place, `ChipGroup` (§13) for options that can be
// picked, `Glyph` for their icons. The look is the stylesheet's (`.empty-state`, `.notice-row`,
// `.chip`); these only give every place the same parts in the same order.

import type { ComponentChildren } from "preact";

/** Lucide's strokes (24 px), for the pieces here and the matrix. */
const GLYPHS: Record<string, ComponentChildren> = {
  unplug: (
    <>
      <path d="m19 5 3-3M2 22l3-3" />
      <path d="M6.3 20.3a2.4 2.4 0 0 0 3.4 0L12 18l-6-6-2.3 2.3a2.4 2.4 0 0 0 0 3.4Z" />
      <path d="M7.5 13.5 10 11M10.5 16.5 13 14" />
      <path d="m12 6 6 6 2.3-2.3a2.4 2.4 0 0 0 0-3.4l-2.6-2.6a2.4 2.4 0 0 0-3.4 0Z" />
    </>
  ),
  network: (
    <>
      <rect x="16" y="16" width="6" height="6" rx="1" />
      <rect x="2" y="16" width="6" height="6" rx="1" />
      <rect x="9" y="2" width="6" height="6" rx="1" />
      <path d="M5 16v-3a1 1 0 0 1 1-1h12a1 1 0 0 1 1 1v3M12 12V8" />
    </>
  ),
  "chart-line": (
    <>
      <path d="M3 3v16a2 2 0 0 0 2 2h16" />
      <path d="m19 9-5 5-4-4-3 3" />
    </>
  ),
  "folder-open": <path d="m6 14 1.5-2.9A2 2 0 0 1 9.2 10H20a2 2 0 0 1 1.9 2.5l-1.5 6a2 2 0 0 1-1.9 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.7.9l.8 1.2a2 2 0 0 0 1.7.9H18a2 2 0 0 1 2 2v2" />,
  "list-ordered": <path d="M10 12h11M10 18h11M10 6h11M4 10h2M4 6h1v4M6 18H4c0-1 2-2 2-3s-1-1.5-2-1" />,
  history: (
    <>
      <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
      <path d="M3 3v5h5M12 7v5l4 2" />
    </>
  ),
  camera: (
    <>
      <path d="M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3l-2.5-3z" />
      <circle cx="12" cy="13" r="3" />
    </>
  ),
  sparkles: <path d="M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9L12 3zM5 3v4M3 5h4M19 17v4M17 19h4" />,
  info: (
    <>
      <circle cx="12" cy="12" r="10" />
      <path d="M12 16v-4M12 8h.01" />
    </>
  ),
  warning: (
    <>
      <path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3" />
      <path d="M12 9v4M12 17h.01" />
    </>
  ),
  error: (
    <>
      <circle cx="12" cy="12" r="10" />
      <path d="m15 9-6 6M9 9l6 6" />
    </>
  ),
  power: (
    <>
      <path d="M12 2v10" />
      <path d="M18.4 6.6a9 9 0 1 1-12.8 0" />
    </>
  ),
  volume: (
    <>
      <path d="M11 5 6 9H2v6h4l5 4V5z" />
      <path d="M15.5 8.5a5 5 0 0 1 0 7" />
    </>
  ),
  "volume-x": (
    <>
      <path d="M11 5 6 9H2v6h4l5 4V5z" />
      <path d="m23 9-6 6M17 9l6 6" />
    </>
  ),
  link: (
    <>
      <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
      <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
    </>
  ),
  "chevron-down": <path d="m6 9 6 6 6-6" />,
};

export type GlyphName = keyof typeof GLYPHS;

/** A Lucide icon in the text's colour (`.shell-icon`), hidden from assistive technology. */
export function Glyph(props: { name: string; class?: string }) {
  return (
    <svg class={`shell-icon${props.class ? ` ${props.class}` : ""}`} viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      {GLYPHS[props.name] ?? null}
    </svg>
  );
}

/**
 * §16: a page with nothing to show. Centred in its place: the icon, a title, one sentence and
 * one action (primary when it is the next step). `role="status"` so a page that empties says so.
 */
export function EmptyState(props: {
  icon: string;
  title: string;
  text?: ComponentChildren;
  action?: { label: string; run: () => void; primary?: boolean; disabled?: boolean };
  /** The region's name, when the empty state stands for a whole screen. */
  label?: string;
  children?: ComponentChildren;
}) {
  return (
    <div class="empty-state" role="status" aria-label={props.label}>
      <Glyph name={props.icon} class="empty-icon" />
      <p class="empty-title">{props.title}</p>
      {props.text ? <p class="empty-text">{props.text}</p> : null}
      {props.action ? (
        <button type="button" class={props.action.primary ? "primary" : undefined} disabled={props.action.disabled} onClick={props.action.run}>
          {props.action.label}
        </button>
      ) : null}
      {props.children}
    </div>
  );
}

/** §14: one condition in its place, one sentence, an optional action at the right. */
export function NoticeRow(props: {
  kind?: "info" | "warning" | "error";
  children: ComponentChildren;
  action?: { label: string; run: () => void; disabled?: boolean };
  /** "alert" for an error the user should hear at once; "status" otherwise. */
  role?: "alert" | "status";
  class?: string;
}) {
  const kind = props.kind ?? "info";
  return (
    <div class={`notice-row notice-row-${kind}${props.class ? ` ${props.class}` : ""}`} role={props.role ?? (kind === "error" ? "alert" : "status")}>
      <Glyph name={kind} class="notice-icon" />
      <span class="notice-text">{props.children}</span>
      {props.action ? (
        <button type="button" disabled={props.action.disabled} onClick={props.action.run}>
          {props.action.label}
        </button>
      ) : null}
    </div>
  );
}

/**
 * §13: options as chips on the control tone; the chosen ones in the selection tone with the
 * accent's text (`aria-pressed`). Many may be on (`selected` + `onToggle`).
 */
export function ChipGroup(props: {
  label: string;
  chips: readonly { key: string | number; text: string; on: boolean; title?: string; disabled?: boolean }[];
  onToggle: (key: string | number) => void;
  class?: string;
}) {
  return (
    <div class={`chips${props.class ? ` ${props.class}` : ""}`} role="group" aria-label={props.label}>
      {props.chips.map((c) => (
        <button type="button" key={c.key} class="chip" aria-pressed={c.on} title={c.title} disabled={c.disabled} onClick={() => props.onToggle(c.key)}>
          {c.text}
        </button>
      ))}
    </div>
  );
}
