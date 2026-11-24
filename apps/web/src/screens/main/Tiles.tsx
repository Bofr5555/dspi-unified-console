// -- WP WEB2 -- (DL6 on the web) The Overview's tiles, as Qt's Modern Overview: the device (its name,
// board, firmware and preset), its effects (each a chip with its on dot that opens its page) and the
// connection it is reached through. Read only; every value is the snapshot's.

import type { Session } from "../../ui/session";
import { useSessionState } from "../../ui/components";
import type { ChainTarget } from "../system/chain";
import { EFFECTS, isOn } from "../effects/model";
import { presetName } from "./model";

/** Qt's tab words for the effects (the Overview's chips). */
const CHIP_WORDS: Record<string, string> = { psybass: "Psy Bass" };

export function DashboardTiles(props: { session: Session; open?: (t: ChainTarget) => void }) {
  const ss = useSessionState(props.session);
  const snap = ss.snapshot;
  if (!snap) return null;
  const s = snap.state;
  const kind = props.session.source.kind;
  const through = kind === "hub" ? `Hub ${ss.hubName || ""}`.trim() : kind === "usb" ? "USB" : kind === "document" ? "A file" : "Demo";
  const effects = EFFECTS.filter((e) => e.on.read(s) !== null);
  const open = props.open;
  return (
    <div class="ov-tiles">
      <section class="card ov-tile" aria-label="Device">
        <h3>Device</h3>
        <dl class="ov-facts">
          <dt>Name</dt>
          <dd>{snap.device.name || "DSPi"}</dd>
          <dt>Board</dt>
          <dd>DSPi · {snap.device.platform}</dd>
          <dt>Firmware</dt>
          <dd>{snap.device.firmware}</dd>
          <dt>Preset</dt>
          <dd>{presetName(s, s.activePresetSlot)}</dd>
        </dl>
      </section>
      <section class="card ov-tile" aria-label="Effects">
        <h3>Effects</h3>
        <div class="chips ov-fx" role="list">
          {effects.map((e) => {
            const on = isOn(e, s);
            const words = CHIP_WORDS[e.id] ?? e.label;
            return (
              <span role="listitem" key={e.id}>
                <button
                  type="button"
                  class="chip ov-fx-chip"
                  disabled={!open}
                  aria-label={`${words}: ${on ? "on" : "off"}`}
                  title={`${words} is ${on ? "on" : "off"}. Opens its page.`}
                  onClick={() => open?.({ screen: "effects", effect: e.id })}
                >
                  <span class={`part-dot${on ? " on" : ""}`} aria-hidden="true" />
                  {words}
                </button>
              </span>
            );
          })}
        </div>
        <p class="ov-hint">Choose one to open its page.</p>
      </section>
      <section class="card ov-tile" aria-label="Connection">
        <h3>Connection</h3>
        <dl class="ov-facts">
          <dt>Through</dt>
          <dd>{through}</dd>
          {snap.device.serial ? (
            <>
              <dt>Serial</dt>
              <dd class="mono">{snap.device.serial}</dd>
            </>
          ) : null}
        </dl>
      </section>
    </div>
  );
}
