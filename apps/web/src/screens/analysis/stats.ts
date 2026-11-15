// WP WB6j: Stats for nerbs (M53, the Console's StatsView) and the Interrupt Monitor (M54): how the
// core's readings are put on screen. The readings are the core's (`statsRead`, `statsTick`,
// `monitorRead`), the fill bands too (on the ring's own scale, from its health); this only words
// them and keeps what a failed read leaves.

import type { MonitorEntryRecord, OutputFillRecord, StatsRead } from "../../core/tools-protocol";

/** The slow poll, ms (the Console's "Updated every 2 seconds"), and the fast one. */
export const SLOW_MS = 2000;
export const FAST_MS = 250;
/** The monitor's lines kept, as the core keeps them. */
export const MONITOR_LIMIT = 2000;
export const MONITOR_POLL_MS = 250;

export interface Row {
  label: string;
  text: string;
  /** A state dot's colour, when the row has one. */
  band?: "good" | "warn" | "bad" | "off";
}

export interface Section {
  title: string;
  rows: Row[];
  note?: string;
}

const dash = "-";
export const khz = (hz: number): string => `${(hz / 1000).toFixed(1)} kHz`;
const yesNo = (v: boolean): string => (v ? "Yes" : "No");
const n = (v: number | null | undefined): string => (typeof v === "number" ? String(v) : dash);

/**
 * The new slow read over the last: a section (or a counter) the device did not answer this time
 * keeps its last value, so a dropped read does not blank the window.
 */
export function mergeSlow(prev: StatsRead | null, next: StatsRead): StatsRead {
  if (!prev) return next;
  const counters: StatsRead["counters"] = { ...next.counters };
  for (const [k, v] of Object.entries(counters)) if (v === null && prev.counters[k] !== undefined) counters[k] = prev.counters[k]!;
  return {
    ...next,
    counters,
    ring: next.ring ?? prev.ring,
    starvation: next.starvation.length || !counters.starvationTotal ? next.starvation : prev.starvation,
  };
}

/** Starvation events since the last read: a lower total than before is a reset (a reboot), a new base. */
export function starvationDelta(last: number | null, total: number): number {
  return last !== null && total >= last ? total - last : 0;
}

export function formatClock(hz: number | null | undefined): string {
  return typeof hz === "number" && hz > 0 ? `${(hz / 1e6).toFixed(1)} MHz` : dash;
}

export function formatVoltage(mv: number | null | undefined): string {
  return typeof mv === "number" && mv > 0 ? `${(mv / 1000).toFixed(2)} V` : dash;
}

/** The firmware reports hundredths of a degree, as a signed 32-bit value in the counter's `u32`. */
export function formatTemperature(c100: number | null | undefined): string {
  if (typeof c100 !== "number") return dash;
  const t = c100 > 0x7fffffff ? c100 - 2 ** 32 : c100;
  return `${(t / 100).toFixed(1)} °C`;
}

export function formatUs(us: number | undefined): string {
  if (typeof us !== "number") return dash;
  return us >= 1000 ? `${(us / 1000).toFixed(2)} ms` : `${us} µs`;
}

export function fillText(f: OutputFillRecord): string {
  return `${f.fillPct.toFixed(1)}% (${f.minPct}–${f.maxPct}%)`;
}

const RX_STATES = ["Inactive", "Acquiring", "Locked", "Relocking"];

/** The sections the slow read gives, in the Console's order; a section the device lacks is left out. */
export function slowSections(s: StatsRead): Section[] {
  const c = s.counters;
  const out: Section[] = [
    {
      title: "System Information",
      rows: [
        { label: "Clock Frequency", text: formatClock(c.clockHz) },
        { label: "Core Voltage", text: formatVoltage(c.coreMv) },
        { label: "Sample Rate", text: typeof c.sampleRate === "number" && c.sampleRate > 0 ? khz(c.sampleRate) : dash },
        { label: "Temperature", text: formatTemperature(c.temperature) },
      ],
    },
    {
      title: "Audio Output",
      rows: [
        { label: "PDM ring", text: `${n(c.pdmRingOverruns)} over · ${n(c.pdmRingUnderruns)} under` },
        { label: "PDM DMA", text: `${n(c.pdmDmaOverruns)} over · ${n(c.pdmDmaUnderruns)} under` },
        { label: "S/PDIF", text: `${n(c.spdifOverruns)} over · ${n(c.spdifUnderruns)} under` },
        { label: "USB Ring", text: `${n(c.usbRingOverruns)} over` },
      ],
    },
  ];
  if (c.starvationTotal) {
    out.push({
      title: "S/PDIF DMA Starvation",
      rows: [{ label: "Total", text: n(c.starvationTotal) }, ...s.starvation.map((v, i) => ({ label: `S/PDIF ${i + 1}`, text: n(v) }))],
    });
  }
  const r = s.ring;
  if (r?.present) {
    out.push({
      title: "Output Ring Health",
      rows: [
        { label: "Target", text: `${n(r.targetFrames)} frames (${formatUs(r.targetUs)})` },
        { label: "Batch", text: `${n(r.batchFrames)} frames` },
        { label: "Underrun episodes", text: n(r.underrunEpisodes), band: r.underrunEpisodes ? "bad" : "good" },
        { label: "Refused blocks", text: n(r.refusedBlocks), band: r.refusedBlocks ? "warn" : "good" },
        { label: "Lowest fill", text: r.lowestFill == null ? dash : `${r.lowestFill} frames` },
        { label: "Worst input gap", text: formatUs(r.worstInputGapUs) },
        { label: "Worst USB gap", text: formatUs(r.worstUsbGapUs) },
        { label: "Over-budget steps", text: n(r.overBudgetSteps), band: r.overBudgetSteps ? "warn" : "good" },
      ],
    });
  }
  const rx = s.spdifRx;
  if (rx) {
    const locked = rx.state === 2;
    out.push({
      title: "S/PDIF Input",
      rows: [
        { label: "State", text: RX_STATES[rx.state] ?? `Unknown (${rx.state})`, band: locked ? "good" : rx.state === 0 ? "off" : "warn" },
        { label: "Sample Rate", text: locked && rx.sampleRate ? khz(rx.sampleRate) : dash },
        { label: "Pin", text: rx.pin ? `GPIO ${rx.pin}` : dash },
        { label: "Locks / losses", text: `${rx.lockCount} / ${rx.lossCount}` },
        { label: "Parity errors", text: String(rx.parityErrors), band: rx.parityErrors ? "warn" : undefined },
        { label: "FIFO", text: `${rx.fifoFillPct}%` },
      ],
    });
  }
  if (s.i2s && s.i2s.state !== 0) {
    out.push({
      title: "I2S Input (Slave Clock)",
      rows: [
        { label: "Detected", text: s.i2s.detectedRate ? khz(s.i2s.detectedRate) : dash },
        { label: "Measured", text: s.i2s.measuredHz ? `${s.i2s.measuredHz} Hz` : dash },
        { label: "Locks / losses", text: `${s.i2s.lockCount} / ${s.i2s.lossCount}` },
      ],
    });
  }
  if (s.adat?.enabled) {
    out.push({
      title: "ADAT Bulk Output",
      rows: [
        { label: "Active", text: yesNo(s.adat.active), band: s.adat.active ? "good" : "off" },
        { label: "Rate", text: s.adat.rateOk ? "OK" : "Not carried", band: s.adat.rateOk ? undefined : "warn" },
        { label: "Pin", text: `GPIO ${s.adat.pin}` },
        { label: "Resyncs / slips", text: `${s.adat.resyncCount} / ${s.adat.slipCount}` },
      ],
    });
  }
  if (s.lg?.enabled) {
    out.push({
      title: "LG Sound Sync",
      rows: [
        { label: "TV", text: s.lg.present ? "Connected" : "Disconnected", band: s.lg.present ? "good" : "off" },
        { label: "Volume", text: s.lg.volume === 0xff ? dash : String(s.lg.volume) },
        { label: "Muted", text: yesNo(s.lg.muted) },
      ],
    });
  }
  if (s.clockDiag?.length) {
    out.push({
      title: "Clock Diagnostics",
      rows: s.clockDiag.map((f) => ({ label: f.name, text: f.value })),
      note: "Shown because this firmware was built with clock diagnostics. Values as the device reports them, latched once a second.",
    });
  }
  const u = s.usbErrors;
  if (u && u.total > 0) {
    out.push({
      title: "USB Errors",
      rows: [
        { label: "Total", text: String(u.total), band: "warn" },
        { label: "CRC", text: String(u.crc) },
        { label: "Bit stuffing", text: String(u.bitstuff) },
        { label: "RX overflow", text: String(u.rxOverflow) },
        { label: "RX timeout", text: String(u.rxTimeout) },
        { label: "Data sequence", text: String(u.dataSeq) },
      ],
      note: "Shown because the device has counted USB errors.",
    });
  }
  return out;
}

/** The monitor's lines with `more` added, the oldest dropped past the limit. */
export function appendLines(lines: MonitorEntryRecord[], more: MonitorEntryRecord[], limit = MONITOR_LIMIT): MonitorEntryRecord[] {
  if (!more.length) return lines;
  const all = lines.concat(more);
  return all.length > limit ? all.slice(all.length - limit) : all;
}

export function eventCount(n: number): string {
  return n === 1 ? "1 event" : `${n} events`;
}

/** A line's time as the monitor shows it: hours to milliseconds, local time. */
export function lineTime(ms: number): string {
  const d = new Date(ms);
  const p = (v: number, w = 2) => String(v).padStart(w, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

/** The lines as Copy puts them on the clipboard, oldest first. */
export function copyText(lines: MonitorEntryRecord[]): string {
  return lines.map((l) => `${lineTime(l.timeMs)}  ${l.text}`).join("\n");
}
