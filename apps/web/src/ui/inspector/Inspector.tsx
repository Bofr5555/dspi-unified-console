// -- WP WEBM2 -- The inspector (MODLAY 2): a panel for what is selected on the page, beside the
// page in the shell's frame. A page fills it with `Inspect` (its content is the page's own, drawn
// into the panel through a portal), so the frame needs to know nothing about pages; with no page
// filling it, it is not shown. The viewer can hide it; it stacks under the page on a narrow console.

import type { ComponentChildren } from "preact";
import { createContext } from "preact";
import { createPortal } from "preact/compat";
import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from "preact/hooks";
import { safeLocalStorage } from "../../fleet/local";
import { bandInInspector, loadCollapsed, placement, saveCollapsed, type Placement } from "./model";

interface Slot {
  /** Where `Inspect` draws, null before the panel is mounted. */
  body: HTMLElement | null;
  placement: Placement;
  coarse: boolean;
  /** A page's `Inspect` mounted (+1) or gone (-1). */
  use: (d: 1 | -1) => void;
}

const SlotContext = createContext<Slot | null>(null);

/** A coarse pointer (a finger): the band bar stays under the graph. */
function useCoarse(): boolean {
  const query = typeof matchMedia === "function" ? matchMedia("(pointer: coarse)") : null;
  const [coarse, setCoarse] = useState(query?.matches ?? false);
  useEffect(() => {
    if (!query) return;
    const on = () => setCoarse(query.matches);
    query.addEventListener?.("change", on);
    return () => query.removeEventListener?.("change", on);
  }, []);
  return coarse;
}

/** The page and its inspector, side by side or stacked. */
export function InspectorArea(props: { children: ComponentChildren }) {
  const row = useRef<HTMLDivElement | null>(null);
  const [body, setBody] = useState<HTMLElement | null>(null);
  const [users, setUsers] = useState(0);
  const [width, setWidth] = useState(0);
  const [collapsed, setCollapsed] = useState(() => loadCollapsed(safeLocalStorage()));
  const coarse = useCoarse();
  useLayoutEffect(() => {
    const el = row.current;
    if (!el) return;
    const measure = () => setWidth(Math.round(el.getBoundingClientRect().width));
    measure();
    if (typeof ResizeObserver !== "function") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const use = useCallback((d: 1 | -1) => setUsers((n) => n + d), []);
  const place = placement(width, collapsed);
  const slot = useMemo<Slot>(() => ({ body, placement: place, coarse, use }), [body, place, coarse, use]);
  const toggle = () => {
    saveCollapsed(safeLocalStorage(), !collapsed);
    setCollapsed(!collapsed);
  };
  const shown = users > 0;
  return (
    <SlotContext.Provider value={slot}>
      <div class={`page-row${shown ? ` inspector-${place}` : ""}`} ref={row}>
        {props.children}
        <aside class="inspector" aria-label="Inspector" hidden={!shown} data-placement={place}>
          <div class="inspector-head">
            <h3>Inspector</h3>
            <button
              type="button"
              class="inspector-toggle"
              aria-expanded={!collapsed}
              title={collapsed ? "Show the inspector" : "Hide the inspector to give the page the full width"}
              onClick={toggle}
            >
              {collapsed ? "Show" : "Hide"}
            </button>
          </div>
          <div class="inspector-body" ref={setBody} hidden={collapsed} />
        </aside>
      </div>
    </SlotContext.Provider>
  );
}

/** Draws its children in the inspector beside the page; nothing outside a frame with one. */
export function Inspect(props: { children: ComponentChildren }) {
  const slot = useContext(SlotContext);
  const use = slot?.use;
  useLayoutEffect(() => {
    if (!use) return;
    use(1);
    return () => use(-1);
  }, [use]);
  return slot?.body ? createPortal(<>{props.children}</>, slot.body) : null;
}

/** Where the inspector is, for a page that puts things in it or beside its graph. */
export function useInspector(): { placement: Placement; coarse: boolean; band: boolean } {
  const slot = useContext(SlotContext);
  const place = slot?.body ? slot.placement : "hidden";
  const coarse = slot?.coarse ?? false;
  return { placement: place, coarse, band: bandInInspector(place, coarse) };
}

/** One part of the inspector, with its title, a short summary, and open or closed. */
export function InspectorSection(props: { title: string; summary?: string; open?: boolean; children: ComponentChildren }) {
  return (
    <details class="insp-sec" open={props.open ?? true} aria-label={props.title}>
      <summary>
        <span class="insp-title">{props.title}</span>
        {props.summary ? <span class="insp-summary">{props.summary}</span> : null}
      </summary>
      <div class="insp-body">{props.children}</div>
    </details>
  );
}
