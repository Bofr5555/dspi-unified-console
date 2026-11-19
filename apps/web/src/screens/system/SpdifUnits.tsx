// WP ATWEB: the S/PDIF transport in the Installer's "Clock and audio links" panel
// (docs/integrator/audio-link.md "S/PDIF", WP ATHUBM): a unit fed by S/PDIF from a DSPi source,
// and the source's time-base slots and D. Between DSPi units S/PDIF carries the time base in its
// user bits; foreign S/PDIF carries none, and the follower then says so ("this S/PDIF source does
// not carry a DSPi time base"). The edits are staged in the panel's draft like every other; the
// hub validates the model (no speaker on a source's time-base slot, no follower of a follower)
// and `system.align` writes the units.

import { useState } from "preact/hooks";
import { noticeRow } from "../../ui/texts";
import type { SystemFeed } from "./feed";
import { useFeed } from "./SystemScreen";
import { isSpdifFed, isUsbDirect, type SpdifSeen, type SystemModel, type Unit } from "./model";

/** The notice of a follower that hears no DSPi time base. */
export const NO_TIME_BASE = "spdif_no_time_base";

/** Output slots: bit n is outputs 2n + 1 and 2n + 2 (1-based). */
const SLOTS = [0, 1, 2, 3];

export function isSpdifUnit(u: Unit): boolean {
  return isSpdifFed(u) || !!u.spdifSource;
}

export function spdifStateWord(s: SpdifSeen | null | undefined): string {
  if (!s) return "Not read";
  switch (s.state) {
    case "locked":
      return "Locked";
    case "acquiring":
      return "Acquiring";
    case "anchoring":
      return "Anchoring";
    case "holdover":
      return "Holdover";
    case "lock_error":
      return "Lock error";
    case "none":
      return "No S/PDIF time base";
    default:
      return s.state || "Unknown";
  }
}

/** The follower hears no DSPi time base: other equipment's S/PDIF, or a DSPi not set as a source. */
export function noTimeBase(s: SpdifSeen | null | undefined): boolean {
  return !!s && s.mode === "follower" && (s.foreign || (s.lockError && !s.superframe));
}

/** `model` with unit `id` following S/PDIF source `source` (what S/PDIF cannot carry removed). */
export function withSpdifFollower(model: SystemModel, id: string, source: string): SystemModel {
  return {
    ...model,
    units: model.units.map((u) => {
      if (u.id !== id) return u;
      const { slots: _s, hopDelayFrames: _h, usbMode: _m, repeater: _r, output: _o, spdifSource: _src, spdifTxMask: _t, spdifD: _d, ...rest } = u;
      return { ...rest, transport: "spdif", upstream: source };
    }),
  };
}

/** `model` with unit `id` an S/PDIF time-base source on slot mask `mask` (default: outputs 7 and 8). */
export function withSpdifSource(model: SystemModel, id: string, mask = 0b1000): SystemModel {
  return { ...model, units: model.units.map((u) => (u.id === id ? { ...u, spdifSource: true, spdifTxMask: mask } : u)) };
}

export function withSpdifMask(model: SystemModel, id: string, mask: number): SystemModel {
  return { ...model, units: model.units.map((u) => (u.id === id ? { ...u, spdifTxMask: mask } : u)) };
}

export function withSpdifD(model: SystemModel, id: string, d: number | null): SystemModel {
  return {
    ...model,
    units: model.units.map((u) => {
      if (u.id !== id) return u;
      const { spdifD: _d, ...rest } = u;
      return d == null ? rest : { ...rest, spdifD: d };
    }),
  };
}

/** `model` with unit `id` no longer on S/PDIF: back on the audio link, no source role. */
export function withoutSpdif(model: SystemModel, id: string, upstream: string): SystemModel {
  return {
    ...model,
    units: model.units.map((u) => {
      if (u.id !== id) return u;
      const { spdifSource: _s, spdifTxMask: _t, spdifD: _d, transport: _tr, ...rest } = u;
      if (isSpdifFed(u)) return { ...rest, upstream: u.spdifSource ? u.upstream || upstream : upstream };
      return u.transport ? { ...rest, transport: u.transport } : rest;
    }),
  };
}

function slotsWord(mask: number | undefined): string {
  const on = SLOTS.filter((n) => ((mask ?? 0) >> n) & 1).map((n) => `${2 * n + 1}-${2 * n + 2}`);
  return on.length ? `Outputs ${on.join(", ")}` : "None";
}

export function SpdifUnitsTable(props: { feed: SystemFeed; model: SystemModel; admin: boolean; onEdit: (m: SystemModel) => void }) {
  const st = useFeed(props.feed);
  const { model, admin } = props;
  const units = model.units.filter(isSpdifUnit);
  const sources = model.units.filter((u) => u.spdifSource);
  const candidates = model.units.filter((u) => !isSpdifUnit(u) && !isUsbDirect(u));
  const [addUnit, setAddUnit] = useState("");
  const [addAs, setAddAs] = useState("");
  const firstUpstream = model.senders[0]?.id ?? "";
  if (!units.length && !admin) return null;
  const nameOf = (u: Unit) => u.name || u.id;
  const add = () => {
    if (!addUnit || !addAs) return;
    props.onEdit(addAs === "source" ? withSpdifSource(model, addUnit) : withSpdifFollower(model, addUnit, addAs));
    setAddUnit("");
    setAddAs("");
  };
  return (
    <>
      <h4>S/PDIF units</h4>
      {units.length ? (
        <div class="sys-scroll">
          <table class="sys-table" aria-label="S/PDIF units">
            <thead>
              <tr>
                <th>Unit</th>
                <th>
                  <abbr title="A source stamps its frame counter in the S/PDIF user bits; a follower plays each sample its D after it arrived">Role</abbr>
                </th>
                <th>
                  <abbr title="The output slots of a source that carry the time base (never a speaker)">Time base on</abbr>
                </th>
                <th>
                  <abbr title="The domain's D, samples (a source's; absent: the hub's)">D</abbr>
                </th>
                <th>State</th>
                {admin ? <th /> : null}
              </tr>
            </thead>
            <tbody>
              {units.map((u) => {
                const seen = st.status?.observed[u.serial]?.spdif ?? null;
                const name = nameOf(u);
                const follower = isSpdifFed(u) && !u.spdifSource;
                const notice = noTimeBase(seen) ? noticeRow(NO_TIME_BASE) : null;
                return (
                  <tr key={u.id} aria-label={`S/PDIF clock of ${name}`}>
                    <th scope="row">{name}</th>
                    <td>
                      {follower ? (
                        admin ? (
                          <select aria-label={`S/PDIF source of ${name}`} value={u.upstream} onChange={(e) => props.onEdit(withSpdifFollower(model, u.id, (e.currentTarget as HTMLSelectElement).value))}>
                            {sources.map((s) => (
                              <option key={s.id} value={s.id}>
                                Follows {nameOf(s)}
                              </option>
                            ))}
                          </select>
                        ) : (
                          <span>Follows {nameOf(sources.find((s) => s.id === u.upstream) ?? u)}</span>
                        )
                      ) : (
                        <span>Source</span>
                      )}
                    </td>
                    <td>
                      {u.spdifSource && admin ? (
                        <span style={{ display: "inline-flex", gap: "6px", flexWrap: "wrap" }}>
                          {SLOTS.map((n) => (
                            <label key={n}>
                              <input
                                type="checkbox"
                                aria-label={`${name} sends the time base on outputs ${2 * n + 1}-${2 * n + 2}`}
                                checked={!!(((u.spdifTxMask ?? 0) >> n) & 1)}
                                onChange={(e) => props.onEdit(withSpdifMask(model, u.id, (e.currentTarget as HTMLInputElement).checked ? (u.spdifTxMask ?? 0) | (1 << n) : (u.spdifTxMask ?? 0) & ~(1 << n)))}
                              />{" "}
                              {2 * n + 1}-{2 * n + 2}
                            </label>
                          ))}
                        </span>
                      ) : u.spdifSource ? (
                        slotsWord(u.spdifTxMask)
                      ) : (
                        "–"
                      )}
                    </td>
                    <td>
                      {u.spdifSource && admin ? (
                        <input
                          type="number"
                          aria-label={`S/PDIF D of ${name}`}
                          min={129}
                          max={399}
                          placeholder="The hub's"
                          value={u.spdifD ?? ""}
                          onChange={(e) => {
                            const t = (e.currentTarget as HTMLInputElement).value;
                            const v = t === "" ? null : Math.round(Number(t));
                            if (v === null || (Number.isFinite(v) && v >= 0 && v <= 65535)) props.onEdit(withSpdifD(model, u.id, v));
                          }}
                        />
                      ) : (
                        <span>{u.spdifD ?? seen?.d ?? "–"}</span>
                      )}
                    </td>
                    <td>
                      <span class={seen?.lockError ? "sys-bad" : undefined}>{st.status?.observed[u.serial]?.online === false ? "Offline" : spdifStateWord(seen)}</span>
                      {notice ? (
                        <div class="sys-issue" role="note" data-key={NO_TIME_BASE}>
                          <strong>{notice.title}</strong> {name}: {notice.reason} {notice.action}
                        </div>
                      ) : null}
                    </td>
                    {admin ? (
                      <td>
                        <button type="button" onClick={() => props.onEdit(withoutSpdif(model, u.id, firstUpstream))}>
                          Off S/PDIF
                        </button>
                      </td>
                    ) : null}
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      ) : null}
      {admin && candidates.length ? (
        <div class="sys-save" role="group" aria-label="Add an S/PDIF unit">
          <select aria-label="Unit to put on S/PDIF" value={addUnit} onChange={(e) => setAddUnit((e.currentTarget as HTMLSelectElement).value)}>
            <option value="">Unit…</option>
            {candidates.map((u) => (
              <option key={u.id} value={u.id}>
                {nameOf(u)}
              </option>
            ))}
          </select>
          <select aria-label="As" value={addAs} onChange={(e) => setAddAs((e.currentTarget as HTMLSelectElement).value)}>
            <option value="">As…</option>
            <option value="source">S/PDIF time-base source</option>
            {sources.map((s) => (
              <option key={s.id} value={s.id}>
                Fed by S/PDIF from {nameOf(s)}
              </option>
            ))}
          </select>
          <button type="button" disabled={!addUnit || !addAs} onClick={add}>
            Add
          </button>
        </div>
      ) : null}
    </>
  );
}
