// The EQ graph's picture (WP WB6b): one function of what is shown, drawn on a
// canvas only when that changes (EqGraph schedules it). The curves are the
// core's, on its grid; this only maps them to pixels.

import type { FilterParams } from "../../core/protocol";
import { type Geometry, x as xOf, y as yOf } from "./model";

export interface SceneBand {
  b: number;
  p: FilterParams;
  /** The band alone (the core's), for its fill; null until known. */
  curve: number[] | null;
  colour: string;
  /** The dot. */
  cx: number;
  cy: number;
  selected: boolean;
  hovered: boolean;
}

export interface Scene {
  g: Geometry;
  freqs: number[] | null;
  /** The other channels shown, each a thin line. */
  others: { curve: number[]; colour: string; offset: number; dashed?: boolean }[]; // -- WP P1WB2 -- dashed: an AutoEQ preview
  /** The edited channel's combined curve and its level shift (gain or trim). */
  main: { curve: number[] | null; colour: string; offset: number } | null;
  bands: SceneBand[];
  marquee: { x0: number; y0: number; x1: number; y1: number } | null;
  /** Dots large enough for a finger. */
  coarse: boolean;
  theme: { bg: string; grid: string; zero: string; text: string; ring: string };
  // -- WP WB6I -- the Graphing settings; without them the graph draws as before
  look?: GraphLook;
}

// -- WP WB6I --
/** What the Graphing settings change in the picture (`screens/settings/model.ts`). */
export interface GraphLook {
  freqGrid: boolean;
  freqLabels: boolean;
  dbGrid: boolean;
  dbLabels: boolean;
  /** The grid's opacity as a multiple of the theme's (1 is the theme's own). */
  gridAlpha: number;
  /** The edited channel's line, px. */
  lineWidth: number;
  glow: boolean;
}

const FREQ_LINES = [20, 30, 40, 50, 60, 70, 80, 90, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000, 20000];
const FREQ_LABELS = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000];

export function dotRadius(coarse: boolean): number {
  return coarse ? 9 : 7;
}

export function draw(ctx: CanvasRenderingContext2D, s: Scene): void {
  const { g, theme } = s;
  ctx.clearRect(0, 0, g.width, g.height);
  ctx.fillStyle = theme.bg;
  ctx.fillRect(0, 0, g.width, g.height);

  // Grid: frequency lines, dB lines, labels.
  const look = s.look; // -- WP WB6I --
  ctx.lineWidth = 1;
  ctx.strokeStyle = theme.grid;
  ctx.globalAlpha = Math.min(1, Math.max(0, look?.gridAlpha ?? 1)); // -- WP WB6I --
  ctx.beginPath();
  for (const f of look && !look.freqGrid ? [] : FREQ_LINES) {
    const px = Math.round(xOf(g, f)) + 0.5;
    ctx.moveTo(px, 0);
    ctx.lineTo(px, g.height);
  }
  const span = g.dbTop - g.dbBottom;
  const step = span <= 30 ? 5 : span <= 60 ? 10 : 20;
  for (let db = Math.ceil(g.dbBottom / step) * step; db <= g.dbTop; db += step) {
    if (db === 0 || (look && !look.dbGrid)) continue; // -- WP WB6I --
    const py = Math.round(yOf(g, db)) + 0.5;
    ctx.moveTo(0, py);
    ctx.lineTo(g.width, py);
  }
  ctx.stroke();
  ctx.globalAlpha = 1; // -- WP WB6I --
  ctx.strokeStyle = theme.zero;
  ctx.beginPath();
  const zero = Math.round(yOf(g, 0)) + 0.5;
  ctx.moveTo(0, zero);
  ctx.lineTo(g.width, zero);
  ctx.stroke();

  ctx.fillStyle = theme.text;
  ctx.font = "10px system-ui, sans-serif";
  ctx.textBaseline = "bottom";
  let lastRight = -Infinity;
  for (const f of look && !look.freqLabels ? [] : FREQ_LABELS.filter((f) => f >= g.minFreq && f <= g.maxFreq)) { // -- WP WB6I --
    const text = f >= 1000 ? `${f / 1000}k` : String(f);
    const w = ctx.measureText(text).width;
    // -- WP WEB3 -- (§12) 6 px inside the plot, the last one ("20k") whole
    const px = Math.min(Math.max(xOf(g, f) - w / 2, LABEL_INSET), g.width - w - LABEL_INSET);
    if (px < lastRight + 6) continue;
    ctx.fillText(text, px, g.height - LABEL_INSET);
    lastRight = px + w;
  }
  ctx.textBaseline = "middle";
  for (let db = Math.ceil(g.dbBottom / step) * step; db <= g.dbTop; db += step) {
    const py = yOf(g, db);
    if (py < 0 || py > g.height || (look && !look.dbLabels)) continue; // -- WP WB6I --
    // -- WP WEB3 -- (§12) the top and bottom labels ("+25", "−25") whole, 6 px in, clear of the frequencies
    ctx.fillText(db > 0 ? `+${db}` : db < 0 ? `\u2212${-db}` : "0", LABEL_INSET, Math.min(Math.max(py, LABEL_INSET + 5), g.height - LABEL_INSET - 18));
  }

  if (!s.freqs) return;
  const freqs = s.freqs;
  const path = (curve: number[], offset: number) => {
    ctx.beginPath();
    for (let i = 0; i < curve.length && i < freqs.length; i++) {
      const px = xOf(g, freqs[i]!);
      const py = yOf(g, curve[i]! + offset);
      if (i === 0) ctx.moveTo(px, py);
      else ctx.lineTo(px, py);
    }
  };

  // The other channels shown.
  ctx.lineWidth = 1.25;
  ctx.globalAlpha = 0.7;
  for (const o of s.others) {
    ctx.strokeStyle = o.colour;
    if (o.dashed) ctx.setLineDash([6, 4]); // -- WP P1WB2 --
    path(o.curve, o.offset);
    ctx.stroke();
    if (o.dashed) ctx.setLineDash([]);
  }
  ctx.globalAlpha = 1;

  // The edited channel's bands: each one's area between its curve and 0 dB.
  for (const band of s.bands) {
    if (!band.curve || band.curve.length === 0 || band.p.bypass) continue;
    ctx.globalAlpha = band.selected ? 0.32 : band.hovered ? 0.26 : 0.16;
    ctx.fillStyle = band.colour;
    path(band.curve, 0);
    ctx.lineTo(xOf(g, freqs[Math.min(freqs.length, band.curve.length) - 1]!), zero);
    ctx.lineTo(xOf(g, freqs[0]!), zero);
    ctx.closePath();
    ctx.fill();
  }
  ctx.globalAlpha = 1;

  // Its combined curve.
  if (s.main?.curve) {
    ctx.lineWidth = look?.lineWidth ?? 2; // -- WP WB6I --
    ctx.strokeStyle = s.main.colour;
    if (look?.glow) {
      ctx.shadowColor = s.main.colour;
      ctx.shadowBlur = 8;
    }
    path(s.main.curve, s.main.offset);
    ctx.stroke();
    ctx.shadowBlur = 0; // -- WP WB6I --
  }

  // The dots, the selected ones on top.
  const r = dotRadius(s.coarse);
  const order = [...s.bands].sort((a, b) => Number(a.selected) - Number(b.selected) || Number(a.hovered) - Number(b.hovered));
  ctx.font = `600 ${s.coarse ? 10 : 9}px system-ui, sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  for (const band of order) {
    ctx.beginPath();
    ctx.arc(band.cx, band.cy, r, 0, Math.PI * 2);
    if (band.p.bypass) {
      ctx.fillStyle = theme.bg;
      ctx.fill();
      ctx.lineWidth = 1.5;
      ctx.strokeStyle = band.colour;
      ctx.stroke();
    } else {
      ctx.fillStyle = band.colour;
      ctx.fill();
    }
    if (band.selected || band.hovered) {
      ctx.beginPath();
      ctx.arc(band.cx, band.cy, r + 2.5, 0, Math.PI * 2);
      ctx.lineWidth = band.selected ? 2 : 1;
      ctx.strokeStyle = theme.ring;
      ctx.stroke();
    }
    ctx.fillStyle = band.p.bypass ? band.colour : "#1c1c1e";
    ctx.fillText(String(band.b + 1), band.cx, band.cy + 0.5);
  }
  ctx.textAlign = "start";

  if (s.marquee) {
    const m = s.marquee;
    ctx.setLineDash([4, 3]);
    ctx.lineWidth = 1;
    ctx.strokeStyle = theme.ring;
    ctx.fillStyle = theme.grid;
    const rx = Math.min(m.x0, m.x1);
    const ry = Math.min(m.y0, m.y1);
    ctx.fillRect(rx, ry, Math.abs(m.x1 - m.x0), Math.abs(m.y1 - m.y0));
    ctx.strokeRect(rx + 0.5, ry + 0.5, Math.abs(m.x1 - m.x0), Math.abs(m.y1 - m.y0));
    ctx.setLineDash([]);
  }
}

/** -- WP WEB3 -- How far inside the plot its axis labels are drawn (§12). */
const LABEL_INSET = 6;
