// The Linkwitz Transform panel (WP WB6c, M49), after the Console's
// `LinkwitzTransformEditor` (Components.swift): the driver's f0 and Q0 and the
// target fp and Qp, edited as a draft and sent with Apply (Revert drops the
// draft), with the low-frequency boost it implies and the Console's warning
// above +15 dB. Apply writes `eq.freq`, `eq.q`, `eq.fp` and `eq.qp` (the
// band's edits, read back).

import { useState } from "preact/hooks";
import type { FilterParams } from "../../core/protocol";
import { Dialog } from "../main/Dialog";
import { ValueField } from "./Field";
import { LT_FP, LT_Q, clamp, dcBoost, freqText, parseFreq, parseNumber, qText, round3, roundFreq } from "./model";

/** Above this the boost is drawn as a warning. */
const BOOST_WARN_DB = 15;

export function LinkwitzDialog(props: { band: number; p: FilterParams; disabled: boolean; apply: (p: FilterParams) => void; onClose: () => void }) {
  const [draft, setDraft] = useState(props.p);
  const p = props.p;
  // The Console's tolerances (f 0.05 Hz, Q0 0.0005, Qp 1/512).
  const dirty =
    Math.abs(draft.freq - p.freq) > 0.05 || Math.abs(draft.gain - p.gain) > 0.05 || Math.abs(draft.q - p.q) > 0.0005 || Math.abs(draft.qp - p.qp) > 1 / 512;
  const boost = dcBoost(draft.freq, draft.gain);
  const set = (patch: Partial<FilterParams>) => setDraft((d) => ({ ...d, ...patch }));
  const off = props.disabled;
  const buttons = [
    { label: "Apply", onClick: () => dirty && !off && props.apply(draft) },
    { label: "Revert", onClick: () => setDraft(p) },
    { label: "Close", onClick: props.onClose },
  ];
  return (
    <Dialog title="Linkwitz Transform" onCancel={props.onClose} buttons={buttons}>
      <p class="note">Re-align a sealed woofer's rolloff from the driver's (f0, Q0) to a target (fp, Qp).</p>
      <div class="lt-grid" role="group" aria-label={`Band ${props.band + 1}`}>
        <span class="lt-row">Driver</span>
        <ValueField label="f0" text={freqText(draft.freq)} parse={parseFreq} disabled={off} onCommit={(v) => set({ freq: roundFreq(clamp(v, LT_FP)) })} />
        <ValueField label="Q0" text={qText(draft.q)} parse={(t) => parseNumber(t, "")} disabled={off} onCommit={(v) => set({ q: round3(clamp(v, LT_Q)) })} />
        <span class="lt-row">Target</span>
        <ValueField label="fp" text={freqText(draft.gain)} parse={parseFreq} disabled={off} onCommit={(v) => set({ gain: roundFreq(clamp(v, LT_FP)) })} />
        <ValueField label="Qp" text={qText(draft.qp)} parse={(t) => parseNumber(t, "")} disabled={off} onCommit={(v) => set({ qp: round3(clamp(v, LT_Q)) })} />
      </div>
      <p class={boost > BOOST_WARN_DB ? "lt-boost warn" : "lt-boost"}>
        {boost > BOOST_WARN_DB ? <span aria-hidden="true">⚠ </span> : null}
        DC boost {`${boost >= 0 ? "+" : ""}${boost.toFixed(1)} dB`}
      </p>
      <p class="note">
        Real low-frequency gain (40 x log10(f0/fp)). It uses driver excursion and amp headroom - reduce preamp or master volume to match.
      </p>
      <p class={dirty ? "lt-status pending" : "lt-status"} aria-live="polite">
        {dirty ? "Not applied yet" : "Applied"}
      </p>
    </Dialog>
  );
}
