// -- WP WEB7 -- Stats for nerbs laid out as Qt Modern's StatsPanel (REVE §3.1): three columns of
// caps-labelled sections with rules between them. The left one is the device (information,
// system, the audio output's counters, the output ring's health, the PDM subwoofer's buffers);
// the middle one is the S/PDIF DMA starvation and the buffer fill levels as Qt's StatsFillRow bars
// (the watermarks' band, the fill now, the last 15 seconds traced); the right one is every other
// section (S/PDIF input, LG Sound Sync, ADAT, I2S, clock diagnostics, USB errors). The readings
// and their words are stats.ts's (WB6j); this only arranges them as Qt does and adds the bars.

import type { OutputFillRecord, StatsRead, StatsTickRecord } from "../../core/tools-protocol";
import { formatClock, formatTemperature, formatVoltage, khz, slowSections, type Row, type Section } from "./stats";

/** Qt's counter row: a label with its path under it, the overruns and underruns as two numbers. */
export interface CounterRow {
  label: string;
  path: string;
  over: number | null;
  /** null: the counter has no underrun side (the USB ring). */
  under: number | null | undefined;
}

export interface FillRow {
  key: string;
  title: string;
  fill: number;
  min: number;
  max: number;
  band: "good" | "warn" | "bad";
  /** The series' colour key: the first output of the pair, or "pdm". */
  series: number | "pdm";
  hollow: boolean;
  dashed: boolean;
}

/** How many fast ticks a trace keeps: 15 seconds of the 250 ms poll. */
export const HISTORY = 60;

const dash = "-";
const n = (v: number | null | undefined): number | null => (typeof v === "number" ? v : null);

/** Qt's ring health rows in Qt's words (StatsController::ringHealth). */
export function ringRows(s: StatsRead): Row[] {
  const r = s.ring;
  if (!r?.present) return [];
  const ms = (us: number | undefined) => (typeof us !== "number" ? "" : us % 1000 === 0 ? ` (${us / 1000} ms)` : ` (${(us / 1000).toFixed(2)} ms)`);
  const gap = (us: number | undefined) => (typeof us !== "number" ? dash : us < 10_000 ? `${us} µs` : `${(us / 1000).toFixed(1)} ms`);
  const count = (v: number | undefined) => (typeof v === "number" ? String(v) : dash);
  return [
    { label: "Fill Target", text: typeof r.targetFrames === "number" ? `${r.targetFrames} frames${ms(r.targetUs)}` : dash },
    { label: "Underrun Episodes", text: count(r.underrunEpisodes), band: r.underrunEpisodes ? "bad" : undefined },
    { label: "Refused Blocks", text: count(r.refusedBlocks), band: r.refusedBlocks ? "warn" : undefined },
    { label: "Lowest Fill", text: r.lowestFill == null ? dash : `${r.lowestFill} frames` },
    { label: "Worst Input Gap", text: gap(r.worstInputGapUs) },
    { label: "Worst USB Gap", text: gap(r.worstUsbGapUs) },
    { label: "Over-Budget Steps", text: count(r.overBudgetSteps), band: r.overBudgetSteps ? "warn" : undefined },
  ];
}

export function systemRows(s: StatsRead): Row[] {
  const c = s.counters;
  return [
    { label: "Clock Frequency", text: formatClock(c.clockHz) },
    { label: "Core Voltage", text: formatVoltage(c.coreMv) },
    { label: "Sample Rate", text: typeof c.sampleRate === "number" && c.sampleRate > 0 ? khz(c.sampleRate) : dash },
    { label: "Temperature", text: formatTemperature(c.temperature) },
  ];
}

export function audioCounters(s: StatsRead): CounterRow[] {
  const c = s.counters;
  return [
    { label: "USB Ring", path: "ISR → Main Loop", over: n(c.usbRingOverruns), under: undefined },
    { label: "Buffer Pool", path: "USB → DMA", over: n(c.spdifOverruns), under: n(c.spdifUnderruns) },
  ];
}

export function pdmCounters(s: StatsRead): CounterRow[] {
  const c = s.counters;
  return [
    { label: "Ring Buffer", path: "Core 0 → Core 1", over: n(c.pdmRingOverruns), under: n(c.pdmRingUnderruns) },
    { label: "DMA Buffer", path: "Core 1 → PIO", over: n(c.pdmDmaOverruns), under: n(c.pdmDmaUnderruns) },
  ];
}

/** The right column: every section of the slow read the left and middle ones do not show, in Qt's order. */
export function otherSections(s: StatsRead): Section[] {
  const shown = new Set(["System Information", "Audio Output", "S/PDIF DMA Starvation", "Output Ring Health", "LG Sound Sync"]);
  const rest = slowSections(s).filter((x) => !shown.has(x.title));
  const lg = s.lg
    ? [
        {
          title: "LG Sound Sync",
          rows: [
            { label: "Enabled", text: s.lg.enabled ? "Yes" : "No" },
            { label: "Present", text: s.lg.present ? "Yes" : "No", band: (s.lg.present ? "good" : "off") as Row["band"] },
            { label: "TV Volume", text: s.lg.volume === 0xff || !s.lg.present ? dash : String(s.lg.volume) },
            { label: "TV Mute", text: s.lg.muted ? "On" : "Off" },
          ],
        },
      ]
    : [];
  // Qt: S/PDIF input first, then LG Sound Sync, then the rest
  const spdif = rest.filter((x) => x.title === "S/PDIF Input");
  return [...spdif, ...lg, ...rest.filter((x) => x.title !== "S/PDIF Input")];
}

/** Qt's spdifBand: empty or full is bad, outside a quarter to three quarters a warning. */
export function spdifBand(pct: number): FillRow["band"] {
  const v = Math.floor(Math.min(100, Math.max(0, pct)));
  if (v === 0 || v === 100) return "bad";
  if (v < 25 || v > 75) return "warn";
  return "good";
}

function pdmDmaBand(pct: number): FillRow["band"] {
  const v = Math.floor(Math.min(100, Math.max(0, pct)));
  return v > 50 ? "bad" : v < 5 || v > 30 ? "warn" : "good";
}

function pdmRingBand(pct: number): FillRow["band"] {
  const v = Math.floor(Math.min(100, Math.max(0, pct)));
  return v > 50 ? "bad" : v > 20 ? "warn" : "good";
}

/**
 * Qt's fill rows (StatsController::fillRows): one per output pair the firmware feeds by S/PDIF or
 * I2S ("Out 1/2 (SPDIF)"), from the core's fills on the ring's scale when it has them, else the
 * pair's DMA buffer; then the PDM DMA and ring while the subwoofer plays. `i2s(pair)`: the pair is I2S.
 */
export function fillRows(t: StatsTickRecord, i2s: (pair: number) => boolean): FillRow[] {
  const b = t.buffers;
  const rows: FillRow[] = [];
  for (let i = 0; i < b.numSpdif; i++) {
    const f: OutputFillRecord | undefined = t.fills?.[i];
    const d = b.spdif[i];
    const fill = f ? f.fillPct : (d?.fillPct ?? 0);
    rows.push({
      key: `out${i}`,
      title: `Out ${2 * i + 1}/${2 * i + 2} (${i2s(i) ? "I2S" : "SPDIF"})`,
      fill,
      min: f ? f.minPct : (d?.minPct ?? 0),
      max: f ? f.maxPct : (d?.maxPct ?? 0),
      band: f ? f.band : spdifBand(fill),
      series: 2 * i,
      hollow: false,
      dashed: false,
    });
  }
  if (b.pdmActive) {
    const p = b.pdm;
    rows.push({ key: "pdmDma", title: "PDM DMA", fill: p.dmaFillPct, min: p.dmaMinPct, max: p.dmaMaxPct, band: pdmDmaBand(p.dmaFillPct), series: "pdm", hollow: false, dashed: false });
    rows.push({ key: "pdmRing", title: "PDM Ring", fill: p.ringFillPct, min: p.ringMinPct, max: p.ringMaxPct, band: pdmRingBand(p.ringFillPct), series: "pdm", hollow: true, dashed: true });
  }
  return rows;
}

/** The traces with this tick's fills added, each at most `HISTORY` long (a row gone is dropped). */
export function withHistory(h: ReadonlyMap<string, number[]>, rows: readonly FillRow[], limit = HISTORY): Map<string, number[]> {
  const out = new Map<string, number[]>();
  for (const r of rows) {
    const was = h.get(r.key) ?? [];
    const next = was.length >= limit ? was.slice(was.length - limit + 1) : was.slice();
    next.push(r.fill);
    out.set(r.key, next);
  }
  return out;
}

/** Qt's formatElapsed: "4m 05s", "1h 02m 03s". */
export function elapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}h ${String(m).padStart(2, "0")}m ${sec}s` : `${m}m ${sec}s`;
}
