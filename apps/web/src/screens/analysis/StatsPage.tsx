// -- WP WEB7 -- Stats for nerbs as Qt Modern's StatsPanel (REVE §3.1, "Stats as Qt's sections with
// buffer bars"): stats-page.ts arranges the readings into Qt's three columns; this draws them. The
// polls are WB6j's (the slow read every 2 s, the buffers every 250 ms, a failed read keeps the last).

import type { ComponentChildren } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import type { StatsRead, StatsTickRecord } from "../../core/tools-protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { OUTPUT_COLOURS, PDM_COLOUR } from "../eq/model";
import { FAST_MS, SLOW_MS, mergeSlow, starvationDelta, type Row } from "./stats";
import { audioCounters, elapsed, fillRows, otherSections, pdmCounters, ringRows, systemRows, withHistory, type CounterRow, type FillRow } from "./stats-page";
import "./stats-page.css";

/** Run `fn` every `ms` while mounted and the page is seen, first at once (as ToolsScreen's). */
function usePoll(fn: () => Promise<void>, ms: number, on: boolean, deps: unknown[]): void {
  useEffect(() => {
    if (!on) return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const run = async () => {
      if (!alive) return;
      if (document.visibilityState !== "hidden") await fn().catch(() => undefined);
      if (alive) timer = setTimeout(run, ms);
    };
    void run();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [on, ms, ...deps]);
}

function Sec(props: { title: string; tools?: ComponentChildren; children: ComponentChildren }) {
  return (
    <section class="nerb-sec" aria-label={props.title}>
      <div class="nerb-head">
        <h4 class="section-label">{props.title}</h4>
        {props.tools}
      </div>
      {props.children}
    </section>
  );
}

function Rows(props: { rows: Row[] }) {
  return (
    <dl class="nerb-rows">
      {props.rows.map((r) => (
        <div key={r.label} class="nerb-row">
          <dt>{r.label}</dt>
          <dd class={r.band ? `nerb-${r.band}` : undefined}>
            {r.band ? <span class={`stat-dot ${r.band}`} aria-hidden="true" /> : null}
            {r.text}
          </dd>
        </div>
      ))}
    </dl>
  );
}

function Counters(props: { rows: CounterRow[] }) {
  return (
    <div class="nerb-counters">
      {props.rows.map((c) => (
        <div key={c.label} class="nerb-counter" aria-label={`${c.label}: ${c.over ?? "-"} over${c.under === undefined ? "" : `, ${c.under ?? "-"} under`}`}>
          <span class="nerb-counter-name">
            <span>{c.label}</span>
            <span class="nerb-path">{c.path}</span>
          </span>
          <span class={`nerb-count${c.over ? " nerb-warn" : ""}`}>
            <b>{c.over ?? "-"}</b>
            <span class="nerb-path">over</span>
          </span>
          {c.under === undefined ? null : (
            <span class={`nerb-count${c.under ? " nerb-bad" : ""}`}>
              <b>{c.under ?? "-"}</b>
              <span class="nerb-path">under</span>
            </span>
          )}
        </div>
      ))}
    </div>
  );
}

/** Qt's StatsFillRow: the dot, the title, the watermarks and the fill now over the last 15 s. */
function FillBar(props: { row: FillRow; history: number[]; colour: string }) {
  const { row, history, colour } = props;
  const W = 300;
  const H = 48;
  const top = 17;
  const bottom = H - 2;
  const y = (pct: number) => bottom - (Math.max(0, Math.min(100, pct)) / 100) * (bottom - top);
  const step = history.length > 1 ? W / (history.length - 1) : 0;
  const trace = history.length > 1 ? history.map((v, i) => `${i ? "L" : "M"}${(i * step).toFixed(1)},${y(v).toFixed(1)}`).join("") : "";
  return (
    <div class="nerb-fill" role="img" aria-label={`${row.title} ${Math.round(row.fill)}%, ${row.min}–${row.max}% since the watermarks were reset`} data-band={row.band} title={`${row.title} fill level over the last 15 seconds`}>
      <svg class="nerb-trace" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" aria-hidden="true">
        <rect x={0} y={y(row.max)} width={W} height={Math.max(1, y(row.min) - y(row.max))} fill={colour} opacity="0.14" />
        <line x1={0} x2={W} y1={y(50)} y2={y(50)} class="nerb-guide" />
        {trace ? <path d={trace} fill="none" stroke={colour} stroke-width="1.25" stroke-dasharray={row.dashed ? "3 2" : undefined} vector-effect="non-scaling-stroke" /> : null}
      </svg>
      <div class="nerb-fill-head">
        <span class="nerb-fill-dot" style={{ background: row.hollow ? "transparent" : colour, borderColor: colour }} aria-hidden="true" />
        <span class="nerb-fill-title">{row.title}</span>
        <span class="nerb-fill-range" title="Lowest and highest fill since the watermarks were reset">
          {row.min}–{row.max}%
        </span>
        <span class={`nerb-fill-now nerb-${row.band}`}>{Math.round(row.fill)}%</span>
      </div>
    </div>
  );
}

export function StatsPage(props: ScreenProps) {
  const { session } = props;
  const st = useSessionState(session);
  const open = st.status === "open";
  const [slow, setSlow] = useState<StatsRead | null>(null);
  const [fast, setFast] = useState<StatsTickRecord | null>(null);
  const [error, setError] = useState("");
  const [history, setHistory] = useState<Map<string, number[]>>(new Map());
  const [, tick] = useState(0);
  const starv = useRef<{ last: number | null; delta: number; at: number | null; before: number | null }>({ last: null, delta: 0, at: null, before: null });

  usePoll(
    async () => {
      try {
        const r = await session.core.call("statsRead", undefined);
        const total = r.counters.starvationTotal ?? null;
        if (typeof total === "number") {
          const d = starvationDelta(starv.current.last, total);
          starv.current.delta = d;
          if (d > 0) starv.current = { ...starv.current, before: starv.current.at, at: Date.now() };
          starv.current.last = total;
        }
        setSlow((prev) => mergeSlow(prev, r));
        setError("");
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    SLOW_MS,
    open,
    [session],
  );
  const labels = st.snapshot?.outputLabels;
  const i2s = (pair: number) => (labels?.[2 * pair]?.slotName ?? "").startsWith("I2S");
  // -- WP INTE -- (owner 2026-11-24) a pair whose two outputs nothing is routed to is left out of this view
  // -- WP FIN015 -- (REVF R1) `hidden`: not while the generator plays on one of them
  const unused = (pair: number) => labels?.[2 * pair]?.hidden === true && labels?.[2 * pair + 1]?.hidden === true;
  const unusedNow = useRef(unused); // the poll's closure reads the labels of now
  unusedNow.current = unused;
  usePoll(
    async () => {
      const t = await session.core.call("statsTick", undefined);
      setFast(t);
      setHistory((h) => withHistory(h, fillRows(t, i2s, (pair) => unusedNow.current(pair))));
    },
    FAST_MS,
    open,
    [session],
  );
  // "Time since last event" counts on between the slow reads
  useEffect(() => {
    const id = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(id);
  }, []);

  const d = st.snapshot?.device;
  const reset = async () => {
    try {
      await session.core.call("statsResetWatermarks", undefined);
    } catch (e) {
      session.fail(e);
    }
  };
  const b = fast?.buffers;
  const fills = fast ? fillRows(fast, i2s, unused) : [];
  const colourOf = (r: FillRow) => (r.series === "pdm" ? PDM_COLOUR : (OUTPUT_COLOURS[r.series] ?? OUTPUT_COLOURS[0]!));
  const total = slow?.counters.starvationTotal;
  const slots = slow ? slow.starvation.slice(0, b?.numSpdif ?? slow.starvation.length) : [];
  const now = Date.now();
  const s = starv.current;
  const ring = slow ? ringRows(slow) : [];
  const others = slow ? otherSections(slow) : [];
  return (
    <section class="stats nerbs" aria-label="Stats for nerbs">
      {error ? <p class="error">{error}</p> : null}
      <div class="nerb-columns">
        <div class="nerb-col">
          {d ? (
            <Sec title="Device Information">
              <Rows
                rows={[
                  { label: "Platform", text: d.platform },
                  { label: "Firmware", text: d.firmware ? `v${d.firmware.replace(/^v/, "")}` : "-" },
                  { label: "Serial", text: d.serial || "-" },
                ]}
              />
            </Sec>
          ) : null}
          {slow ? (
            <>
              <Sec title="System Information">
                <Rows rows={systemRows(slow)} />
              </Sec>
              <Sec title="Audio Output">
                <Counters rows={audioCounters(slow)} />
              </Sec>
              {ring.length ? (
                <Sec title="Output Ring Health">
                  <Rows rows={ring} />
                </Sec>
              ) : null}
              <Sec title="PDM (Subwoofer)">
                <Counters rows={pdmCounters(slow)} />
              </Sec>
            </>
          ) : null}
        </div>
        <div class="nerb-col">
          {slow ? (
            <Sec title="SPDIF DMA Starvation">
              <dl class="nerb-rows">
                <div class="nerb-row nerb-total">
                  <dt>Total</dt>
                  <dd>
                    {s.delta > 0 ? <span class="nerb-delta">+{s.delta}</span> : null}
                    <b>{typeof total === "number" ? total : "-"}</b>
                  </dd>
                </div>
                {slots.map((v, i) =>
                  unused(i) ? null : (
                    <div key={i} class="nerb-row nerb-sub">
                      <dt>
                        Out {2 * i + 1}/{2 * i + 2}
                      </dt>
                      <dd>{typeof v === "number" ? v : "-"}</dd>
                    </div>
                  ),
                )}
                <div class="nerb-row nerb-sub">
                  <dt>Time since last event</dt>
                  <dd>{s.at ? elapsed(now - s.at) : "-"}</dd>
                </div>
                <div class="nerb-row nerb-sub">
                  <dt>Time between last two</dt>
                  <dd>{s.at && s.before ? elapsed(s.at - s.before) : "-"}</dd>
                </div>
              </dl>
            </Sec>
          ) : null}
          {b ? (
            <Sec
              title="Buffer Fill Levels"
              tools={
                <button type="button" disabled={!open || !!session.readOnlyReason} onClick={() => void reset()}>
                  Reset Watermarks
                </button>
              }
            >
              <div class="nerb-legend">
                <span>
                  <span class={`stat-dot ${b.streaming ? "good" : "off"}`} aria-hidden="true" />
                  Audio Streaming<span class="visually-hidden">: {b.streaming ? "yes" : "no"}</span>
                </span>
                <span>
                  <span class={`stat-dot ${b.pdmActive ? "good" : "off"}`} aria-hidden="true" />
                  PDM Active<span class="visually-hidden">: {b.pdmActive ? "yes" : "no"}</span>
                </span>
              </div>
              <div class="nerb-fills">
                {fills.map((r) => (
                  <FillBar key={r.key} row={r} history={history.get(r.key) ?? []} colour={colourOf(r)} />
                ))}
              </div>
            </Sec>
          ) : null}
        </div>
        {others.length ? (
          <div class="nerb-col">
            {others.map((sec) => (
              <Sec key={sec.title} title={sec.title}>
                {sec.note ? <p class="nerb-note">{sec.note}</p> : null}
                <Rows rows={sec.rows} />
              </Sec>
            ))}
          </div>
        ) : null}
      </div>
      <div class="nerb-foot">
        <span>
          <span class={`stat-dot ${open ? "good" : "off"}`} aria-hidden="true" />
          {open ? "Connected" : "Disconnected"}
        </span>
        <span>Updated every 2 seconds</span>
      </div>
    </section>
  );
}
