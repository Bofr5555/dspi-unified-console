// The EQ graph with on-graph editing (WP WB6b, M21), after the Console's
// `PeqGraphEditor.swift` (modelled there on FabFilter Pro-Q). The curves are
// the core's; the canvas is drawn only when something it shows changes.
//
// Mouse (Ctrl or Cmd where the Console says Command):
//   double click empty graph   a bell there
//   Ctrl-click empty graph     a card of shapes, to add one there
//   click empty graph          deselect
//   drag the curve             pull a new band out of it (a shelf near either end)
//   drag empty graph           marquee selection (Shift adds)
//   click a dot                select; Ctrl toggles, Shift selects a range
//   drag a dot                 frequency and gain (Q for 2nd-order cuts;
//                              frequency only for notches, all-passes, 1st-order cuts)
//   Ctrl-drag                  Q;  Shift-drag fine;  Alt-drag one axis
//   Alt-click a dot            bypass
//   double click a dot         type its values (the chip below)
//   wheel on a band            Q; Ctrl-wheel gain; Shift fine. Wheel at the left edge: dB zoom
//   right click                band menu on a dot, graph menu elsewhere
// Touch: tap selects, drag moves, double tap adds a bell, two fingers pinch Q,
// a long press opens the menus. The chip has steppers for each value too.
// Keys (the graph focused): Tab and Shift-Tab step through the bands (and
// leave the graph past the last), arrows move frequency and gain (Alt-arrows
// Q, Shift fine), Delete removes, Escape deselects, Ctrl-A selects all, Enter
// types values, Shift-F10 or the menu key opens the band menu.

import { memo } from "preact/compat";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "preact/hooks";
import type { FilterParams } from "../../core/protocol";
import type { Curves } from "./curves";
import { dotRadius, draw, type Scene, type SceneBand } from "./draw";
import type { BandEdits } from "./edits";
import { PopupMenu, type PopupItem } from "./Menu";
// -- WP WEBM4 -- the band and graph menus' rows are actions.json's
import { menuItems as actionMenu, ownRow, popupItems, type Handler, type MenuRow } from "../../ui/menus/actions";
// -- WP WB6I -- the Graphing settings
import { readoutText, usePrefs } from "../settings/model";
import {
  OFF_BAND,
  SHAPES,
  bandColour,
  bandLabel,
  bandTag,
  clamp,
  curveAt,
  dragResult,
  frequencyOrder,
  isGraphBand,
  newBand,
  nudge,
  orderTitle,
  pinch,
  retyped,
  roleDb,
  roleOf,
  shapeOf,
  T,
  usesGain,
  wheel as wheelEdit,
  x as xOf,
  y as yOf,
  type DragStart,
  type Geometry,
  type Limits,
  type Role,
} from "./model";

export interface GraphProps {
  /** For ids: one graph per session view. */
  idBase: string;
  channelName: string;
  edits: BandEdits;
  curves: Curves | null;
  freqs: number[] | null;
  /** The other channels shown. */
  others: { curve: number[]; colour: string; offset: number; dashed?: boolean }[]; // -- WP P1WB2 --
  /** The edited channel: its curve's colour and level shift; its curve is `curves.chains[0]`. */
  colour: string;
  offset: number;
  selection: ReadonlySet<number>;
  anchor: number | null;
  select: (sel: Set<number>, anchor: number | null) => void;
  limits: Limits;
  available: number[];
  bypassOk: boolean;
  disabled: boolean;
  /** Type values for this band (the chip's fields). */
  openFields: (b: number) => void;
}

type Gesture =
  | { kind: "idle" }
  | { kind: "press"; b: number; x: number; y: number; ctrl: boolean; alt: boolean; shift: boolean; touch: boolean }
  | { kind: "background"; x: number; y: number; onCurve: boolean; shift: boolean; touch: boolean }
  | { kind: "drag"; d: DragStart; last: { x: number; y: number }; offset: { x: number; y: number }; axis: "h" | "v" | null; pending: { x: number; y: number } | null }
  | { kind: "marquee"; x0: number; y0: number; x1: number; y1: number; base: Set<number> }
  | { kind: "pinch"; d0: number; start: Map<number, FilterParams> };

// -- WP WB6c -- (WB6b-4) "orders": the shape card's second step, a shape's slopes
type Popup = { kind: "band" | "graph" | "shapes" | "orders"; x: number; y: number; b?: number; shape?: string };

const LONG_PRESS_MS = 500;
const DOUBLE_MS = 400;
const ZOOM_ZONE = 40;

export const EqGraph = memo(function EqGraph(props: GraphProps) {
  const { edits, curves, freqs, selection, anchor, select, limits, available, disabled } = props;
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  // -- WP WB6I -- the Graphing settings: the frequency span, the vertical range and centre
  const look = usePrefs().graph;
  const [range, setRange] = useState(look.dbRange / 2);
  useEffect(() => setRange(look.dbRange / 2), [look.dbRange]);
  const readout = useRef<HTMLDivElement>(null);
  const [hovered, setHovered] = useState<number | null>(null);
  const [marquee, setMarquee] = useState<Scene["marquee"]>(null);
  const [popup, setPopup] = useState<Popup | null>(null);
  const [note, setNote] = useState("");
  const [coarse, setCoarse] = useState(false);
  const gesture = useRef<Gesture>({ kind: "idle" });
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  const lastUp = useRef<{ t: number; x: number; y: number; b: number | null } | null>(null);
  const longPress = useRef<ReturnType<typeof setTimeout> | null>(null);
  const keyMenuAt = useRef(0);

  const g: Geometry = useMemo(
    () => ({ width: size.width, height: size.height, minFreq: look.minFreq, maxFreq: look.maxFreq, dbTop: look.dbCenter + range, dbBottom: look.dbCenter - range }), // -- WP WB6I --
    [size.width, size.height, range, look.minFreq, look.maxFreq, look.dbCenter],
  );
  const bands = edits.shown;
  const graphBands = useMemo(() => bands.map((p, b) => (isGraphBand(p) ? b : -1)).filter((b) => b >= 0), [bands]);
  const role = (b: number, p: FilterParams): Role => roleOf(p, curves?.bands[b]);
  const dot = (b: number, p: FilterParams) => ({
    x: xOf(g, p.freq),
    y: clamp(yOf(g, roleDb(role(b, p), p)), [6, Math.max(6, g.height - 6)]),
  });

  // ------------------------------------------------------------ size and drawing

  useLayoutEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const measure = () => {
      const r = el.getBoundingClientRect();
      setSize((s) => (s.width === Math.round(r.width) && s.height === Math.round(r.height) ? s : { width: Math.round(r.width), height: Math.round(r.height) }));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const scene = useRef<Scene | null>(null);
  scene.current = {
    g,
    freqs,
    others: props.others,
    main: { curve: curves?.chains[0] ?? null, colour: props.colour, offset: props.offset },
    bands: graphBands.map<SceneBand>((b) => {
      const p = bands[b]!;
      const d = dot(b, p);
      return { b, p, curve: curves?.bands[b]?.curve ?? null, colour: bandColour(b), cx: d.x, cy: d.y, selected: selection.has(b), hovered: hovered === b };
    }),
    marquee,
    coarse,
    theme: { bg: "", grid: "", zero: "", text: "", ring: "" },
    // -- WP WB6I --
    look: {
      freqGrid: look.showFreqGrid,
      freqLabels: look.showFreqLabels,
      dbGrid: look.showDbGrid,
      dbLabels: look.showDbLabels,
      gridAlpha: look.gridOpacity / 0.5,
      lineWidth: look.lineWidth,
      glow: look.showGlow,
    },
  };

  const frame = useRef(0);
  const visible = useRef(true);
  const dirty = useRef(false);
  const paint = () => {
    frame.current = 0;
    const c = canvas.current;
    const s = scene.current;
    if (!c || !s || !wrap.current || s.g.width === 0 || s.g.height === 0) return;
    if (!visible.current || document.visibilityState === "hidden") {
      dirty.current = true;
      return;
    }
    dirty.current = false;
    const css = getComputedStyle(wrap.current);
    const v = (n: string, d: string) => css.getPropertyValue(n).trim() || d;
    s.theme = {
      bg: v("--eq-bg", "#151518"),
      grid: v("--eq-grid", "rgba(255,255,255,0.08)"),
      zero: v("--eq-zero", "rgba(255,255,255,0.3)"),
      text: v("--eq-text", "rgba(255,255,255,0.55)"),
      ring: v("--eq-ring", "#ffffff"),
    };
    const dpr = window.devicePixelRatio || 1;
    const w = Math.round(s.g.width * dpr);
    const h = Math.round(s.g.height * dpr);
    if (c.width !== w || c.height !== h) {
      c.width = w;
      c.height = h;
    }
    const ctx = c.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    draw(ctx, s);
    // For the tests' redraw probe: how often it was drawn.
    (c as unknown as { dspiDraws: number }).dspiDraws = ((c as unknown as { dspiDraws?: number }).dspiDraws ?? 0) + 1;
  };
  const schedule = () => {
    if (!frame.current) frame.current = requestAnimationFrame(paint);
  };

  // A redraw only when what is drawn changes.
  useEffect(schedule, [g, freqs, props.others, curves, bands, selection, hovered, marquee, coarse, props.colour, props.offset, look]); // -- WP WB6I -- look

  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    // Off screen (scrolled away, another tab): drawn when seen again, if it changed meanwhile.
    const io = new IntersectionObserver((es) => {
      visible.current = es.some((e) => e.isIntersecting);
      if (visible.current && dirty.current) schedule();
    });
    io.observe(el);
    const seen = () => document.visibilityState === "visible" && dirty.current && schedule();
    document.addEventListener("visibilitychange", seen);
    const dark = window.matchMedia?.("(prefers-color-scheme: dark)");
    dark?.addEventListener?.("change", schedule);
    // -- WP WB6c -- (WB6b-2) a host page that flips `data-theme` on the console (or above it)
    const themed = new MutationObserver(schedule);
    for (let n: Element | null = el; n; n = n.parentElement) themed.observe(n, { attributes: true, attributeFilter: ["data-theme"] });
    return () => {
      themed.disconnect();
      io.disconnect();
      document.removeEventListener("visibilitychange", seen);
      dark?.removeEventListener?.("change", schedule);
      if (frame.current) cancelAnimationFrame(frame.current);
      frame.current = 0;
    };
  }, []);

  // ------------------------------------------------------------ hit tests

  const point = (e: { clientX: number; clientY: number }) => {
    const r = wrap.current!.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };

  /** The dot under `p`, selected and hovered dots first (they are on top). */
  const nodeAt = (p: { x: number; y: number }, touch: boolean): number | null => {
    const reach = dotRadius(coarse) + (touch ? 10 : 3);
    let best: { b: number; score: number } | null = null;
    for (const b of graphBands) {
      const d = dot(b, bands[b]!);
      const dist = Math.hypot(d.x - p.x, d.y - p.y);
      if (dist > reach) continue;
      const score = dist - (selection.has(b) ? 3 : 0) - (hovered === b ? 2 : 0);
      if (!best || score < best.score) best = { b, score };
    }
    return best?.b ?? null;
  };

  /** The band whose area (between its curve and 0 dB) holds `p`: it takes the wheel. */
  const lobeAt = (p: { x: number; y: number }): number | null => {
    if (!freqs || !curves) return null;
    const f = 10 ** (Math.log10(g.minFreq) + (p.x / Math.max(g.width, 1)) * (Math.log10(g.maxFreq) - Math.log10(g.minFreq)));
    const zero = yOf(g, 0);
    let best: { b: number; d: number } | null = null;
    for (const b of graphBands) {
      const c = curves.bands[b]?.curve;
      if (!c?.length || role(b, bands[b]!).kind === "locked") continue;
      const py = yOf(g, curveAt(freqs, c, f));
      if (Math.abs(py - zero) < 3 || p.y < Math.min(py, zero) || p.y > Math.max(py, zero)) continue;
      const d = Math.abs(p.y - py);
      if (!best || d < best.d) best = { b, d };
    }
    return best?.b ?? null;
  };

  const nearCurve = (p: { x: number; y: number }, touch: boolean) => {
    const c = curves?.chains[0];
    if (!freqs || !c?.length) return false;
    const f = 10 ** (Math.log10(g.minFreq) + (p.x / Math.max(g.width, 1)) * (Math.log10(g.maxFreq) - Math.log10(g.minFreq)));
    return Math.abs(yOf(g, curveAt(freqs, c, f) + props.offset) - p.y) <= (touch ? 12 : 6);
  };

  const freeSlot = (): number | null => {
    const i = bands.findIndex((p, b) => p.filterType === 0 && edits.current(b).filterType === 0);
    return i >= 0 ? i : null;
  };

  // ------------------------------------------------------------ band operations

  const targets = (b: number) => (selection.has(b) ? [...selection] : [b]);

  const create = (p: FilterParams) => {
    const slot = freeSlot();
    if (slot === null) {
      setNote(`All ${bands.length} bands are in use.`);
      return;
    }
    setNote("");
    edits.commit(new Map([[slot, p]]));
    select(new Set([slot]), slot);
  };

  const remove = (bs: number[]) => {
    const real = bs.filter((b) => isGraphBand(bands[b]!));
    if (!real.length) return;
    edits.commit(new Map(real.map((b) => [b, { ...OFF_BAND }])));
    select(new Set([...selection].filter((b) => !real.includes(b))), null);
  };

  const toggleBypass = (bs: number[]) => {
    if (!props.bypassOk) return;
    const real = bs.filter((b) => isGraphBand(bands[b]!)).sort((a, b) => a - b);
    if (!real.length) return;
    const to = !edits.current(real[0]!).bypass;
    edits.commit(new Map(real.map((b) => [b, { ...edits.current(b), bypass: to }])));
  };

  const setOrder = (bs: number[], order: 1 | 2) => {
    const m = new Map<number, FilterParams>();
    for (const b of bs) {
      const p = edits.current(b);
      const s = shapeOf(p.filterType);
      const t = s ? (s.shape.types as Partial<Record<1 | 2, number>>)[order] : undefined;
      if (t !== undefined && available.includes(t)) m.set(b, retyped(p, t));
    }
    edits.commit(m);
  };

  const invert = (bs: number[]) =>
    edits.commit(new Map(bs.filter((b) => usesGain(edits.current(b).filterType)).map((b) => [b, { ...edits.current(b), gain: -edits.current(b).gain }])));

  // ------------------------------------------------------------ pointer

  const stopLongPress = () => {
    if (longPress.current) clearTimeout(longPress.current);
    longPress.current = null;
  };

  /** Events in the open menu are the menu's. */
  const inMenu = (e: Event) => (e.target as Element | null)?.closest?.(".graph-menu") != null;

  const onPointerDown = (e: PointerEvent) => {
    if (inMenu(e)) return;
    if (e.button !== 0 && e.pointerType === "mouse") return;
    const touch = e.pointerType !== "mouse";
    if (touch !== coarse) setCoarse(touch);
    const p = point(e);
    wrap.current?.focus({ preventScroll: true });
    if (popup) {
      setPopup(null);
      return;
    }
    try {
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    } catch {
      // A pointer the browser no longer knows: the moves still arrive here.
    }
    pointers.current.set(e.pointerId, p);
    // Two fingers: pinch the selection's Q (or the band the first finger holds).
    if (pointers.current.size === 2 && !disabled) {
      stopLongPress();
      const [a, b] = [...pointers.current.values()];
      const cur = gesture.current;
      const held = cur.kind === "press" ? [cur.b] : cur.kind === "drag" ? [cur.d.grabbed] : [];
      const bs = selection.size ? [...selection] : held;
      if (cur.kind === "drag") edits.release();
      gesture.current = { kind: "pinch", d0: Math.hypot(a!.x - b!.x, a!.y - b!.y), start: new Map(bs.map((x) => [x, edits.current(x)])) };
      // -- WP WB8 -- one undo step for the pinch
      edits.drag();
      return;
    }
    if (pointers.current.size > 2) return;
    const ctrl = e.ctrlKey || e.metaKey;
    const b = nodeAt(p, touch);
    if (b !== null) {
      if (!ctrl && !e.altKey && !e.shiftKey && !selection.has(b)) select(new Set([b]), b);
      gesture.current = { kind: "press", b, x: p.x, y: p.y, ctrl, alt: e.altKey, shift: e.shiftKey, touch };
      if (touch) longPress.current = setTimeout(() => openMenu("band", p, b), LONG_PRESS_MS);
      return;
    }
    if (ctrl && !disabled) {
      gesture.current = { kind: "idle" };
      setPopup({ kind: "shapes", x: p.x, y: p.y });
      return;
    }
    gesture.current = { kind: "background", x: p.x, y: p.y, onCurve: !disabled && freeSlot() !== null && nearCurve(p, touch), shift: e.shiftKey, touch };
    if (touch) longPress.current = setTimeout(() => openMenu("graph", p), LONG_PRESS_MS);
  };

  const beginDrag = (grabbed: number, bs: number[], start?: Map<number, FilterParams>, qMode = false) => {
    const s = start ?? new Map(bs.map((b) => [b, edits.current(b)]));
    if (!s.has(grabbed)) s.set(grabbed, edits.current(grabbed));
    gesture.current = { kind: "drag", d: { grabbed, start: s, qMode }, last: { x: 0, y: 0 }, offset: { x: 0, y: 0 }, axis: null, pending: null };
    // -- WP WB8 -- one undo step for the drag
    edits.drag();
  };

  const continueDrag = (p: { x: number; y: number }, e: PointerEvent) => {
    const cur = gesture.current;
    if (cur.kind !== "drag") return;
    const scale = e.shiftKey ? 0.12 : 1;
    const move = { x: (p.x - cur.last.x) * scale, y: (p.y - cur.last.y) * scale };
    cur.last = p;
    const dominant = (v: { x: number; y: number }) => (Math.abs(v.x) >= Math.abs(v.y) ? "h" : "v");
    if (!e.altKey) {
      cur.axis = null;
      cur.pending = null;
      cur.offset.x += move.x;
      cur.offset.y += move.y;
    } else {
      // Alt locks the drag to the axis it shows, without a jump.
      if (cur.axis === null && cur.pending === null) {
        if (Math.hypot(cur.offset.x, cur.offset.y) >= 4) cur.axis = dominant(cur.offset);
        else cur.pending = { x: 0, y: 0 };
      }
      if (cur.pending) {
        cur.pending.x += move.x;
        cur.pending.y += move.y;
        if (Math.hypot(cur.pending.x, cur.pending.y) >= 4) {
          cur.axis = dominant(cur.pending);
          if (cur.axis === "h") cur.offset.x += cur.pending.x;
          else cur.offset.y += cur.pending.y;
          cur.pending = null;
        }
      } else if (cur.axis === "h") cur.offset.x += move.x;
      else cur.offset.y += move.y;
    }
    edits.live(dragResult(cur.d, cur.offset.x, cur.offset.y, g, limits, role, e.ctrlKey || e.metaKey));
  };

  const onPointerMove = (e: PointerEvent) => {
    if (inMenu(e)) return;
    const p = point(e);
    if (pointers.current.has(e.pointerId)) pointers.current.set(e.pointerId, p);
    const cur = gesture.current;
    if (cur.kind === "idle") {
      if (e.pointerType === "mouse") {
        const h = nodeAt(p, false) ?? lobeAt(p);
        if (h !== hovered) setHovered(h);
        // -- WP WB6I -- the cursor readout, without a render
        if (readout.current) {
          const f = 10 ** (Math.log10(g.minFreq) + (p.x / Math.max(g.width, 1)) * (Math.log10(g.maxFreq) - Math.log10(g.minFreq)));
          const db = g.dbTop - (p.y / Math.max(g.height, 1)) * (g.dbTop - g.dbBottom);
          readout.current.textContent = readoutText(look, f, db);
        }
      }
      return;
    }
    if (cur.kind === "pinch") {
      const [a, b] = [...pointers.current.values()];
      if (!a || !b || cur.d0 <= 0) return;
      const ratio = Math.hypot(a.x - b.x, a.y - b.y) / cur.d0;
      const m = new Map<number, FilterParams>();
      for (const [x, s] of cur.start) {
        const n = pinch(s, ratio, limits);
        if (n) m.set(x, n);
      }
      edits.live(m);
      return;
    }
    const threshold = (cur.kind === "press" || cur.kind === "background") && cur.touch ? 6 : 2;
    if (cur.kind === "press") {
      if (Math.hypot(p.x - cur.x, p.y - cur.y) < threshold) return;
      stopLongPress();
      if (disabled || role(cur.b, edits.current(cur.b)).kind === "locked") {
        gesture.current = { kind: "idle" };
        return;
      }
      let bs = selection.has(cur.b) ? [...selection] : [cur.b];
      if (cur.ctrl && !selection.has(cur.b)) {
        bs = [...selection, cur.b];
        select(new Set(bs), cur.b);
      } else if (!selection.has(cur.b)) select(new Set([cur.b]), cur.b);
      beginDrag(cur.b, bs, undefined, cur.ctrl);
      (gesture.current as Extract<Gesture, { kind: "drag" }>).last = { x: cur.x, y: cur.y };
      continueDrag(p, e);
      return;
    }
    if (cur.kind === "background") {
      if (Math.hypot(p.x - cur.x, p.y - cur.y) < threshold) return;
      stopLongPress();
      const slot = cur.onCurve ? freeSlot() : null;
      if (slot !== null) {
        // Pull a new band out of the curve: it starts flat, so the curve does not jump.
        const band = { ...newBand(g, cur.x, cur.y, true, limits, available), gain: 0 };
        select(new Set([slot]), slot);
        beginDrag(slot, [slot], new Map([[slot, band]]));
        (gesture.current as Extract<Gesture, { kind: "drag" }>).last = { x: cur.x, y: cur.y };
        continueDrag(p, e);
      } else {
        gesture.current = { kind: "marquee", x0: cur.x, y0: cur.y, x1: p.x, y1: p.y, base: cur.shift ? new Set(selection) : new Set() };
        updateMarquee();
      }
      return;
    }
    if (cur.kind === "drag") continueDrag(p, e);
    else if (cur.kind === "marquee") {
      cur.x1 = p.x;
      cur.y1 = p.y;
      updateMarquee();
    }
  };

  const updateMarquee = () => {
    const m = gesture.current;
    if (m.kind !== "marquee") return;
    setMarquee({ x0: m.x0, y0: m.y0, x1: m.x1, y1: m.y1 });
    const [lx, hx, ly, hy] = [Math.min(m.x0, m.x1), Math.max(m.x0, m.x1), Math.min(m.y0, m.y1), Math.max(m.y0, m.y1)];
    const inside = graphBands.filter((b) => {
      const d = dot(b, bands[b]!);
      return d.x >= lx && d.x <= hx && d.y >= ly && d.y <= hy;
    });
    const next = new Set([...m.base, ...inside]);
    if (next.size !== selection.size || [...next].some((b) => !selection.has(b))) select(next, frequencyOrder(bands, inside)[0] ?? anchor);
  };

  const onPointerUp = (e: PointerEvent) => {
    if (inMenu(e)) return;
    stopLongPress();
    const p = point(e);
    pointers.current.delete(e.pointerId);
    const cur = gesture.current;
    if (cur.kind === "pinch") {
      if (pointers.current.size === 0) {
        edits.release();
        gesture.current = { kind: "idle" };
      }
      return;
    }
    gesture.current = { kind: "idle" };
    const now = performance.now();
    const prev = lastUp.current;
    const double = !!prev && now - prev.t < DOUBLE_MS && Math.hypot(prev.x - p.x, prev.y - p.y) < (e.pointerType === "mouse" ? 6 : 24);
    switch (cur.kind) {
      case "press": {
        lastUp.current = { t: now, x: p.x, y: p.y, b: cur.b };
        if (double && prev?.b === cur.b) {
          lastUp.current = null;
          props.openFields(cur.b);
          return;
        }
        if (cur.alt && !disabled) toggleBypass(targets(cur.b));
        else if (cur.ctrl) {
          const next = new Set(selection);
          if (next.has(cur.b)) next.delete(cur.b);
          else next.add(cur.b);
          select(next, cur.b);
        } else if (cur.shift) {
          const order = frequencyOrder(bands, graphBands);
          const i = anchor === null ? -1 : order.indexOf(anchor);
          const j = order.indexOf(cur.b);
          if (i >= 0 && j >= 0) select(new Set(order.slice(Math.min(i, j), Math.max(i, j) + 1)), anchor);
          else select(new Set([cur.b]), cur.b);
        } else select(new Set([cur.b]), cur.b);
        return;
      }
      case "background":
        lastUp.current = { t: now, x: p.x, y: p.y, b: null };
        if (double && prev?.b === null) {
          lastUp.current = null;
          if (!disabled) create(newBand(g, p.x, p.y, false, limits, available));
          return;
        }
        if (selection.size) select(new Set(), null);
        return;
      case "drag":
        lastUp.current = null;
        edits.release();
        return;
      case "marquee":
        lastUp.current = null;
        setMarquee(null);
        return;
    }
  };

  const onPointerCancel = (e: PointerEvent) => {
    stopLongPress();
    pointers.current.delete(e.pointerId);
    const cur = gesture.current;
    if (cur.kind === "drag" || cur.kind === "pinch") edits.release();
    if (cur.kind === "marquee") setMarquee(null);
    gesture.current = { kind: "idle" };
  };

  // -- WP WB8 -- the capture gone (the element removed, the browser took it): as a cancel,
  // once no pointer is left down
  const onLostPointerCapture = (e: PointerEvent) => {
    if (pointers.current.has(e.pointerId)) onPointerCancel(e);
  };

  const onPointerLeave = (e: PointerEvent) => {
    if (readout.current) readout.current.textContent = ""; // -- WP WB6I --
    if (e.pointerType === "mouse" && gesture.current.kind === "idle" && hovered !== null) setHovered(null);
  };

  // ------------------------------------------------------------ wheel

  const wheelBand = useRef<{ b: number; t: number } | null>(null);
  const onWheel = (e: WheelEvent) => {
    const p = point(e);
    const now = performance.now();
    const owner = anchor !== null && selection.has(anchor) ? anchor : selection.size ? frequencyOrder(bands, [...selection])[0]! : null;
    let b: number | null = null;
    const cur = gesture.current;
    if (cur.kind === "drag") b = cur.d.grabbed;
    else if (wheelBand.current && now - wheelBand.current.t < 500 && (owner === null || selection.has(wheelBand.current.b))) b = wheelBand.current.b;
    else {
      const under = nodeAt(p, false) ?? lobeAt(p);
      if (under !== null) b = owner ?? under;
    }
    if (b === null) {
      wheelBand.current = null;
      if (p.x <= ZOOM_ZONE) {
        e.preventDefault();
        setRange((r) => clamp(r + Math.sign(e.deltaY) * 2.5, [5, 50]));
      }
      return;
    }
    e.preventDefault();
    wheelBand.current = { b, t: now };
    if (disabled) return;
    const raw = -(e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY);
    const delta = raw * (e.shiftKey ? 0.12 : 1);
    if (!delta) return;
    const m = new Map<number, FilterParams>();
    for (const x of targets(b)) {
      const n = wheelEdit(edits.current(x), delta, e.ctrlKey || e.metaKey, limits);
      if (n) m.set(x, n);
    }
    if (cur.kind === "drag") {
      for (const [x, s] of cur.d.start) {
        const n = m.has(x) ? wheelEdit(s, delta, e.ctrlKey || e.metaKey, limits) : null;
        if (n) cur.d.start.set(x, n);
      }
      edits.live(m);
    } else edits.commit(m);
  };

  useEffect(() => {
    // Not passive: a wheel on a band must not scroll the page.
    const el = wrap.current;
    if (!el) return;
    const h = (e: WheelEvent) => wheelRef.current(e);
    el.addEventListener("wheel", h, { passive: false });
    return () => el.removeEventListener("wheel", h);
  }, []);
  const wheelRef = useRef(onWheel);
  wheelRef.current = onWheel;

  // ------------------------------------------------------------ keys and menus

  const openMenu = (kind: "band" | "graph", p: { x: number; y: number }, b?: number) => {
    gesture.current = { kind: "idle" };
    if (kind === "band" && b !== undefined && !selection.has(b)) select(new Set([b]), b);
    setPopup({ kind, x: p.x, y: p.y, b });
  };

  const onContextMenu = (e: MouseEvent) => {
    e.preventDefault();
    if (inMenu(e)) return;
    if (performance.now() - keyMenuAt.current < 500) return;
    const p = point(e);
    const b = nodeAt(p, false);
    openMenu(b !== null ? "band" : "graph", p, b ?? undefined);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.target !== wrap.current) return;
    const ctrl = e.ctrlKey || e.metaKey;
    const order = frequencyOrder(bands, graphBands);
    if (e.key === "Tab" && !ctrl && !e.altKey) {
      if (!order.length) return;
      const at = anchor === null || !selection.has(anchor) ? -1 : order.indexOf(anchor);
      const next = e.shiftKey ? (at < 0 ? order.length - 1 : at - 1) : at + 1;
      // Past either end, Tab leaves the graph.
      if (next < 0 || next >= order.length) {
        if (selection.size) select(new Set(), null);
        return;
      }
      e.preventDefault();
      select(new Set([order[next]!]), order[next]!);
      return;
    }
    if (e.key === "Escape") {
      if (selection.size) {
        e.preventDefault();
        select(new Set(), null);
      }
      return;
    }
    if (ctrl && e.key.toLowerCase() === "a") {
      e.preventDefault();
      select(new Set(graphBands), order[0] ?? null);
      return;
    }
    if ((e.key === "F10" && e.shiftKey) || e.key === "ContextMenu") {
      e.preventDefault();
      keyMenuAt.current = performance.now();
      const b = anchor !== null && selection.has(anchor) ? anchor : null;
      const at = b !== null ? dot(b, bands[b]!) : { x: g.width / 2, y: g.height / 2 };
      openMenu(b !== null ? "band" : "graph", at, b ?? undefined);
      return;
    }
    if (e.key === "Enter" && anchor !== null && selection.has(anchor)) {
      e.preventDefault();
      props.openFields(anchor);
      return;
    }
    if (disabled) return;
    if ((e.key === "Delete" || e.key === "Backspace") && selection.size) {
      e.preventDefault();
      remove([...selection]);
      return;
    }
    if (e.key === "ArrowLeft" || e.key === "ArrowRight" || e.key === "ArrowUp" || e.key === "ArrowDown") {
      if (!selection.size) return;
      e.preventDefault();
      const m = new Map<number, FilterParams>();
      for (const b of selection) {
        if (!isGraphBand(bands[b]!)) continue;
        const n = nudge(edits.current(b), e.key, e.altKey, e.shiftKey, g, limits);
        if (n) m.set(b, n);
      }
      edits.commit(m);
    }
  };

  const menuItems = (pp: Popup): PopupItem[] => {
    // -- WP WB6c -- (WB6b-4) as the Console's card: a shape with two slopes asks which (`PeqShapeChooser`)
    const add = (t: number) => {
      const base = newBand(g, pp.x, pp.y, false, limits, available);
      create({ ...base, filterType: t, gain: usesGain(t) ? base.gain : 0, q: t === T.peaking || t === T.notch ? 1 : 0.707 });
    };
    const orders = (sh: (typeof SHAPES)[number]) => ([1, 2] as const).filter((o) => {
      const t = (sh.types as Partial<Record<1 | 2, number>>)[o];
      return t !== undefined && available.includes(t);
    });
    if (pp.kind === "shapes") {
      return [
        { heading: "Add Band" },
        ...SHAPES.filter((sh) => orders(sh).length > 0).map((sh) => {
          const os = orders(sh);
          return {
            label: os.length > 1 ? `${sh.title}…` : sh.title,
            onSelect: () => (os.length > 1 ? setPopup({ ...pp, kind: "orders", shape: sh.id }) : add((sh.types as Partial<Record<1 | 2, number>>)[os[0]!]!)),
          };
        }),
      ];
    }
    if (pp.kind === "orders") {
      const sh = SHAPES.find((x) => x.id === pp.shape);
      if (!sh) return [];
      return [
        { heading: sh.title },
        ...orders(sh).map((o) => ({ label: slopeChoice(sh.id, o), onSelect: () => add((sh.types as Partial<Record<1 | 2, number>>)[o]!) })),
        "separator",
        { label: "Back to shapes", onSelect: () => setPopup({ ...pp, kind: "shapes" }) },
      ];
    }
    // -- WP WEBM4 -- the graph's and a band's rows, groups and words from actions.json (LAY3b's
    // `graph` and `band` menus); the rows the web has no handler for are left out
    if (pp.kind === "graph") {
      const h: Record<string, Handler> = {
        "graph.selectAll": { run: () => select(new Set(graphBands), frequencyOrder(bands, graphBands)[0] ?? null), enabled: graphBands.length > 0 },
        "graph.deselect": { run: () => select(new Set(), null), enabled: selection.size > 0 },
      };
      if (!disabled) h["graph.addBand"] = { run: () => setPopup({ kind: "shapes", x: pp.x, y: pp.y }), enabled: freeSlot() !== null };
      if (selection.size && !disabled) {
        const n = selection.size;
        h["graph.delete"] = { run: () => remove([...selection]), danger: true, text: n > 1 ? `Delete ${n} Selected Bands` : "Delete Selected Band" };
      }
      return popupItems(actionMenu("graph", h));
    }
    const b = pp.b!;
    const bs = targets(b);
    const p = edits.current(b);
    const h: Record<string, Handler> = {};
    // The slope is the band menu's own rows (`band.slope`, rows: submenu), each a choice
    const slopes: MenuRow[] = [];
    const s = shapeOf(p.filterType);
    const types = s?.shape.types as Partial<Record<1 | 2, number>> | undefined;
    if (s && types?.[1] !== undefined && types[2] !== undefined && available.includes(types[1]) && available.includes(types[2])) {
      for (const o of [1, 2] as const)
        slopes.push(ownRow(`band.slope.${o}`, `${s.shape.id === "allPass" ? "Order" : "Slope"}: ${orderTitle(s.shape, o)}`, () => setOrder(bs, o), { checked: o === s.order, disabled }));
    }
    if (props.bypassOk) h["band.bypass"] = { run: () => toggleBypass(bs), on: p.bypass, enabled: !disabled };
    if (usesGain(p.filterType)) h["band.invert"] = { run: () => invert(bs), enabled: !disabled };
    // Removing is the web's own row (the desktop's is the Delete key)
    const del = ownRow("band.delete", bs.length > 1 ? `Delete ${bs.length} Bands` : "Delete Band", () => remove(bs), { danger: true, disabled });
    return [{ heading: bs.length > 1 ? `${bs.length} Bands` : `Band ${b + 1}` }, ...popupItems(actionMenu("band", h, [{ with: "band.slope", rows: slopes }, { rows: [del] }]))];
  };

  // ------------------------------------------------------------ what a screen reader hears

  const idle = gesture.current.kind === "idle";
  const announced = anchor !== null && selection.has(anchor) && bands[anchor] ? bandLabel(anchor, bands[anchor]!) : "";
  const [said, setSaid] = useState("");
  // Not during a drag: the values settle first.
  useEffect(() => {
    if (idle && announced !== said) setSaid(announced);
  });
  const helpId = `${props.idBase}-graph-help`;
  // -- WP WEBM2 -- the on-graph label: above the dot, below it near the top; kept inside the graph
  const tagBand = anchor !== null && selection.has(anchor) && bands[anchor] && isGraphBand(bands[anchor]!) && g.width > 0 ? anchor : null;
  const tag = (() => {
    if (tagBand === null || popup) return null;
    const d = dot(tagBand, bands[tagBand]!);
    return { b: tagBand, x: clamp(d.x, [60, Math.max(60, g.width - 60)]), y: d.y, below: d.y < 40, text: bandTag(tagBand, bands[tagBand]!) };
  })();

  return (
    <div
      ref={wrap}
      class="eq-graph"
      role="application"
      aria-roledescription="EQ graph"
      aria-label={`EQ graph, ${props.channelName}`}
      aria-describedby={helpId}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerCancel}
      onLostPointerCapture={onLostPointerCapture}
      onPointerLeave={onPointerLeave}
      onContextMenu={onContextMenu}
      onKeyDown={onKeyDown}
    >
      <canvas ref={canvas} aria-hidden="true" />
      {/* -- WP WB6I -- the cursor readout (Graphing: Show Frequency / Gain Readout) */}
      <div ref={readout} class="eq-readout" aria-hidden="true" />
      {/* -- WP WEBM2 -- the chosen band's values on the graph, by its dot (live while it is dragged) */}
      {tag ? (
        <div class={`eq-band-tag${tag.below ? " below" : ""}`} style={{ left: `${tag.x}px`, top: `${tag.y}px`, borderColor: bandColour(tag.b) }} aria-hidden="true">
          {tag.text}
        </div>
      ) : null}
      <p id={helpId} class="visually-hidden">
        {disabled
          ? "Tab steps through the bands."
          : "Tab steps through the bands. Arrows move frequency and gain, Alt with up and down changes Q, Shift is fine. Delete removes, Enter types values, Shift F10 opens the band menu. Double click adds a band."}
      </p>
      <ul class="visually-hidden" aria-label="Bands on the graph">
        {graphBands.map((b) => (
          <li key={b}>
            {bandLabel(b, bands[b]!)}
            {selection.has(b) ? ", selected" : ""}
          </li>
        ))}
      </ul>
      <p class="visually-hidden graph-announce" aria-live="polite">
        {note || said}
      </p>
      {popup ? (
        <PopupMenu
          key={`${popup.kind}${popup.shape ?? ""}`}
          label={popup.kind === "band" ? "Band" : popup.kind === "shapes" || popup.kind === "orders" ? "Add Band" : "Graph"}
          x={popup.x}
          y={popup.y}
          bounds={size}
          items={menuItems(popup)}
          onClose={(refocus) => {
            setPopup(null);
            if (refocus) wrap.current?.focus({ preventScroll: true });
          }}
        />
      ) : null}
    </div>
  );
});

// -- WP WB6c -- (WB6b-4)
/** The shape card's slope buttons (`PeqShapeChooser`): "6 dB" / "12 dB", for an all-pass "180°" / "360°". */
function slopeChoice(shape: string, order: 1 | 2): string {
  if (shape === "allPass") return order === 1 ? "180°" : "360°";
  return order === 1 ? "6 dB" : "12 dB";
}
