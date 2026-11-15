// WP WB6d: the System view's signal chain of the session's device: inputs with
// their meters, the routing summary, each output's processing blocks with
// their on or off state, and the outputs with their meters, gain, mute and
// delay. Read only: a block opens the screen that edits it. Everything comes
// from the core: the session's snapshot (`DspState`, the output labels), the
// meters (`meters`, polled while the page is seen) and the input source's
// name (the registry's choices for `in.source`).

import type { ComponentChildren } from "preact";
import { memo } from "preact/compat";
import { useEffect, useState } from "preact/hooks";
import type { DspState, OutputLabel, SystemStatus } from "../../core/protocol";
import { staleText, useSessionState } from "../../ui/components";
import type { Session } from "../../ui/session";
import { liveInputs } from "../main/model";
import { blockText, chainView, meterOf, meterPct, outputText, type ChainBlock, type ChainTarget } from "./chain";

/** How often the chain's meters are read while it is shown. */
const METER_MS = 150;

/** The meters, polled only while the page is visible and the session open. */
function useMeters(session: Session): SystemStatus | null {
  const [m, setM] = useState<SystemStatus | null>(null);
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      if (!alive) return;
      if (document.visibilityState === "visible" && session.state.status === "open") {
        try {
          const r = await session.core.call("meters", undefined);
          if (alive) setM(r);
        } catch {
          // The last reading stands; the session's strip shows its state.
        }
      }
      if (alive) timer = setTimeout(tick, METER_MS);
    };
    void tick();
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
    };
  }, [session]);
  return m;
}

/** The registry's name for the selected input source ("USB", "S/PDIF"), "" until read. */
function useSourceName(session: Session, source: number | undefined): string {
  const [choices, setChoices] = useState<{ value: number; label: string }[]>([]);
  useEffect(() => {
    let alive = true;
    session.core.call("describe", { path: "in.source" }).then(
      (p) => alive && setChoices(p.choices ?? []),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [session]);
  return choices.find((c) => c.value === source)?.label ?? "";
}

export function SignalChain(props: { session: Session; open?: (t: ChainTarget) => void }) {
  const st = useSessionState(props.session);
  const snap = st.snapshot;
  const meters = useMeters(props.session);
  const source = useSourceName(props.session, snap?.state.inputSource);
  if (!snap) return null;
  return <ChainView s={snap.state} labels={snap.outputLabels} meters={meters} source={source} open={props.open} />;
}

function ChainView(props: { s: DspState; labels?: OutputLabel[]; meters: SystemStatus | null; source: string; open?: (t: ChainTarget) => void }) {
  const { s, meters, open } = props;
  const chain = chainView(s, props.labels, liveInputs(s, meters?.activeInputChannels ?? null));
  const stale = meters?.stale === true;
  return (
    <section class="sys-chain" aria-label="Signal chain">
      <h3>Signal chain</h3>
      <div class="sys-chain-top">
        <Go open={open} to={{ screen: "main" }} label={`Volume ${chain.userMute ? "muted" : `${chain.userVolumeDb.toFixed(1)} dB`}`}>
          <span class="sys-kind">Volume</span> {chain.userMute ? "Muted" : `${chain.userVolumeDb.toFixed(1)} dB`}
        </Go>
        {chain.bypass ? (
          <span class="sys-chain-bypass" role="status">
            Bypass is on: the processing below is not applied.
          </span>
        ) : null}
      </div>
      {stale ? (
        <p class="sys-note" role="status" aria-label="Chain meters stale">
          Meters stale since {staleText(Date.now() - (meters?.ageMs ?? 0))}: no fresh reading from the DSPi.
        </p>
      ) : null}
      {chain.deviceEffects.length ? (
        <div class="sys-chain-fx">
          <h4>Effects</h4>
          <Blocks blocks={chain.deviceEffects} open={open} label="Effects" />
        </div>
      ) : null}
      <div class={`sys-chain-cols${stale ? " stale" : ""}${chain.bypass ? " bypassed" : ""}`}>
        <section class="sys-chain-col" aria-label="Chain inputs">
          <h4>
            Inputs{props.source ? <span class="sys-state"> · {props.source}</span> : null}
          </h4>
          {chain.inputs.map((i) => (
            <div class="sys-chain-row" key={i.wire} role="group" aria-label={`Chain ${i.descriptor} ${i.name}`}>
              <Go open={open} to={{ screen: "main" }} label={`${i.descriptor} ${i.name}`} cls="sys-chain-name">
                <span class="sys-kind">{i.descriptor}</span> {i.name}
              </Go>
              <span class="sys-state" title="Trim">
                {i.trimDb.toFixed(1)} dB
              </span>
              <Blocks blocks={i.blocks} open={open} label={`Processing of ${i.descriptor}`} />
              <ChainMeter name={`${i.descriptor} ${i.name}`} {...meterOf(meters, i.wire)} />
            </div>
          ))}
        </section>
        <section class="sys-chain-col" aria-label="Chain outputs">
          <h4>Routing and outputs</h4>
          {chain.outputs.map((o) => {
            const t = outputText(o);
            return (
              <div
                class={`sys-chain-row${o.enabled ? "" : " sys-chain-off"}`}
                key={o.wire}
                role="group"
                aria-label={`Chain ${o.descriptor}`}
                data-enabled={o.enabled}
              >
                <Go open={open} to={{ screen: "matrix" }} label={`Routing of ${o.descriptor}: ${o.from}`} cls="sys-chain-from">
                  <span aria-hidden="true">{o.from} →</span>
                </Go>
                <Go open={open} to={o.open} label={`${o.descriptor} ${o.name}`} cls="sys-chain-name">
                  <span class="sys-kind">{o.descriptor}</span> {o.name}
                </Go>
                {o.enabled ? null : <span class="sys-state">Disabled</span>}
                <Blocks blocks={o.blocks} open={open} label={`Processing of ${o.descriptor}`} />
                <span class="sys-chain-level" aria-label={`Level of ${o.descriptor}`}>
                  <span title="Gain">{t.gain}</span>
                  {o.muted ? <span class="sys-chain-mute">Muted</span> : null}
                  <span title="Delay">{t.delay}</span>
                </span>
                <ChainMeter name={`${o.descriptor} ${o.name}`} {...meterOf(meters, o.wire)} />
              </div>
            );
          })}
        </section>
      </div>
    </section>
  );
}

/** A link to the screen that edits a thing; plain text while nothing can open it. */
function Go(props: { open?: (t: ChainTarget) => void; to: ChainTarget | null; label: string; cls?: string; children: ComponentChildren }) {
  const { open, to } = props;
  if (!open || !to) return <span class={props.cls}>{props.children}</span>;
  return (
    <button type="button" class={`sys-chain-go ${props.cls ?? ""}`} aria-label={props.label} title={`Open: ${props.label}`} onClick={() => open(to)}>
      {props.children}
    </button>
  );
}

/** The blocks of a channel, as a list. */
function Blocks(props: { blocks: ChainBlock[]; open?: (t: ChainTarget) => void; label: string }) {
  return (
    <span class="sys-blocks" role="list" aria-label={props.label}>
      {props.blocks.map((b) => (
        <Block key={b.id} b={b} open={props.open} />
      ))}
    </span>
  );
}

/** One block: its name, its state (said, and shown by its style) and what it does; a button when a screen edits it. */
const Block = memo(function Block(props: { b: ChainBlock; open?: (t: ChainTarget) => void }) {
  const { b, open } = props;
  const text = blockText(b);
  const cls = `sys-block ${b.on ? "sys-block-on" : "sys-block-off"}`;
  const body = (
    <>
      {b.label}
      <span class="visually-hidden">{b.on ? ": on" : ": off"}</span>
      {b.on && b.detail ? <span class="sys-block-detail"> {b.detail}</span> : null}
    </>
  );
  const to = b.open;
  return (
    <span role="listitem" class="sys-block-item">
      {open && to ? (
        <button type="button" class={cls} title={`${text}. Opens its screen.`} data-block={b.id} data-on={b.on} onClick={() => open(to)}>
          {body}
        </button>
      ) : (
        <span class={cls} title={text} data-block={b.id} data-on={b.on}>
          {body}
        </span>
      )}
    </span>
  );
});

function ChainMeter(props: { name: string; db: number; clip: boolean }) {
  return (
    <span class={`sys-chain-meter${props.clip ? " clip" : ""}`} role="meter" aria-valuemin={-90} aria-valuemax={0} aria-valuenow={Math.round(props.db)} aria-label={`Meter ${props.name}`}>
      <span style={{ width: `${meterPct(props.db)}%` }} />
    </span>
  );
}
