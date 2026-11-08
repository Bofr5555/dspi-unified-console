// The graph's context menus (WP WB6b): the Console's band menu, its graph
// menu and its shape card, opened at a point of the graph. ARIA's menu
// pattern: the first item takes focus, arrows move, Escape closes and gives
// focus back to the graph.

import { useEffect, useLayoutEffect, useRef } from "preact/hooks";

export type PopupItem =
  | { label: string; disabled?: boolean; checked?: boolean; danger?: boolean; onSelect: () => void }
  | { heading: string }
  | "separator";

export function PopupMenu(props: {
  label: string;
  x: number;
  y: number;
  /** The graph's size: the menu stays inside it. */
  bounds: { width: number; height: number };
  items: PopupItem[];
  onClose: (refocus: boolean) => void;
}) {
  const ref = useRef<HTMLUListElement>(null);
  const items = () => Array.from(ref.current?.querySelectorAll<HTMLButtonElement>("button:not([disabled])") ?? []);

  // Inside the graph: moved left or up when it would leave it.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    el.style.left = `${Math.max(0, Math.min(props.x, props.bounds.width - w))}px`;
    el.style.top = `${Math.max(0, Math.min(props.y, props.bounds.height - h))}px`;
  }, [props.x, props.y, props.bounds.width, props.bounds.height]);

  useEffect(() => {
    items()[0]?.focus();
    const outside = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) props.onClose(false);
    };
    document.addEventListener("pointerdown", outside, true);
    return () => document.removeEventListener("pointerdown", outside, true);
  }, []);

  const onKey = (e: KeyboardEvent) => {
    const all = items();
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    const go = (i: number) => all[(i + all.length) % all.length]?.focus();
    if (e.key === "ArrowDown") go(at + 1);
    else if (e.key === "ArrowUp") go(at - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(all.length - 1);
    else if (e.key === "Escape") props.onClose(true);
    else if (e.key === "Tab") props.onClose(false);
    else return;
    e.preventDefault();
    e.stopPropagation();
  };

  return (
    <ul ref={ref} class="graph-menu" role="menu" aria-label={props.label} onKeyDown={onKey} style={{ left: `${props.x}px`, top: `${props.y}px` }}>
      {props.items.map((m, i) =>
        m === "separator" ? (
          <li key={`s${i}`} role="separator" />
        ) : "heading" in m ? (
          <li key={`h${i}`} role="presentation" class="graph-menu-heading">
            {m.heading}
          </li>
        ) : (
          <li key={m.label} role="none">
            <button
              type="button"
              role={m.checked === undefined ? "menuitem" : "menuitemradio"}
              aria-checked={m.checked === undefined ? undefined : m.checked}
              tabIndex={-1}
              class={m.danger ? "danger" : undefined}
              disabled={m.disabled}
              onClick={() => {
                props.onClose(true);
                m.onSelect();
              }}
            >
              {m.checked ? "✓ " : ""}
              {m.label}
            </button>
          </li>
        ),
      )}
    </ul>
  );
}
