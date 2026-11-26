// -- WP GAP1 -- Crossfeed's output pair map (Qt's CrossfeedPairMap.qml): which two outputs each
// crossfeed pair processes, on firmware with the extension's pair map. The Outputs menu chooses
// Default, the K340's one DAC per ear, or Custom, which opens the editor; a write sends the whole
// map (the core checks it and reads it back, one undo step) and the page shows what the device
// reports. After the K340 choice a question offers to crossfeed both of its pairs, as the factory
// mask covers the woofers only: an explicit click, never automatic. Without the pair map nothing
// of this is shown and the pairs are the default ones. The rules are the core's
// (`crates/dspi-wasm/src/pair_map.rs`); this only shows them.

import { useEffect, useState } from "preact/hooks";
import type { CrossfeedPair, CrossfeedPairMapView, DspState } from "../../core/protocol";
import { NoticeRow } from "../../ui/design";
import { ActionMenuButton } from "../../ui/menus/ActionMenu";
import { ownRow } from "../../ui/menus/actions";
import type { Session } from "../../ui/session";

/** The Outputs menu's words for a choice (Qt's). */
export function choiceTitle(choice: CrossfeedPairMapView["choice"]): string {
  return choice === "k340" ? "One DAC per ear (K340)" : choice === "custom" ? "Custom" : "Default (1/2, 3/4, …)";
}

/** A pair chip's words: "1: Out 3 / Out 1" with the map's outputs (Qt's `pairLabel`). */
export function pairChipText(p: number, pair: CrossfeedPair): string {
  return `${p + 1}: Out ${pair.left + 1} / Out ${pair.right + 1}`;
}

/** Whether to ask to crossfeed both K340 pairs: after that choice, while the mask leaves one out. */
export function offersK340Mask(view: CrossfeedPairMapView, mask: number): boolean {
  return view.choice === "k340" && (mask & view.k340Mask) !== view.k340Mask;
}

/** The pair map of the open device, read again when the mirror's map changes; null while unknown or on another effect. */
export function usePairMap(session: Session, s: DspState, on: boolean): CrossfeedPairMapView | null {
  const [view, setView] = useState<CrossfeedPairMapView | null>(null);
  const key = JSON.stringify([s.crossfeedPairMap, s.platformId, session.state.snapshot?.device.serial]);
  useEffect(() => {
    if (!on) return;
    let alive = true;
    session.core.call("crossfeedPairMap", undefined).then(
      (v) => alive && setView(v),
      () => alive && setView(null),
    );
    return () => {
      alive = false;
    };
  }, [session, key, on]);
  return on ? view : null;
}

export function PairMapBlock(props: {
  session: Session;
  view: CrossfeedPairMapView;
  /** The session takes no writes. */
  ro: boolean;
  /** The crossfeed's pair mask now, and its write (the K340 question). */
  mask: number;
  writeMask: ((mask: number) => void) | null;
  outputName: (o: number) => string;
}) {
  const { session, view } = props;
  const writable = !props.ro && view.supported;
  const [editing, setEditing] = useState(false);
  const [offer, setOffer] = useState(false);
  const [draft, setDraft] = useState<CrossfeedPair[]>([]);
  const [valid, setValid] = useState(true);
  // Once writes stop, the editor closes
  useEffect(() => {
    if (!writable) setEditing(false);
  }, [writable]);
  // The core judges every draft
  useEffect(() => {
    if (!editing) return;
    let alive = true;
    session.core.call("crossfeedPairsValid", { pairs: draft }).then(
      (ok) => alive && setValid(ok),
      () => alive && setValid(false),
    );
    return () => {
      alive = false;
    };
  }, [session, editing, JSON.stringify(draft)]);

  const pick = (key: "default" | "k340" | "custom") => {
    if (key === "custom") {
      setDraft(view.pairs.map((p) => ({ ...p })));
      setValid(true);
      setEditing(true);
      return;
    }
    setOffer(key === "k340");
    void session.change("setCrossfeedPairMap", { choice: key });
  };
  const setSide = (pair: number, side: "left" | "right", output: number) =>
    setDraft((d) => d.map((p, i) => (i === pair ? { ...p, [side]: output } : p)));
  const apply = async () => {
    if (valid && (await session.change("setCrossfeedPairMap", { pairs: draft }))) setEditing(false);
  };
  const outputs = Array.from({ length: 2 * view.pairCount }, (_, o) => o);
  const warning = "Each pair needs two different outputs, and no output can be in two pairs.";

  return (
    <div class="fx-pairmap" role="group" aria-label="Crossfeed outputs">
      <div class="fx-pairmap-row">
        <span class="fx-title">Outputs</span>
        <ActionMenuButton
          label={choiceTitle(view.choice)}
          title="Which two outputs each crossfeed pair processes"
          disabled={!writable}
          items={() => [
            ownRow("fx.cf.pairmap.default", "Default (1/2, 3/4, …)", () => pick("default")),
            ownRow("fx.cf.pairmap.k340", "One DAC per ear (K340)", () => pick("k340")),
            ownRow("fx.cf.pairmap.custom", "Custom…", () => pick("custom")),
          ]}
        />
      </div>
      <p class="fx-help">Which two outputs each pair crossfeeds. An output can belong to one pair only.</p>
      {offer && offersK340Mask(view, props.mask) ? (
        <div class="fx-pairmap-offer">
          <span class="fx-help">Crossfeed the woofers and the tweeters?</span>
          <button type="button" onClick={() => setOffer(false)}>
            Not Now
          </button>
          <button
            type="button"
            disabled={!writable || !props.writeMask}
            onClick={() => {
              props.writeMask?.(props.mask | view.k340Mask);
              setOffer(false);
            }}
          >
            Enable Pairs 1 and 2
          </button>
        </div>
      ) : null}
      {editing && writable ? (
        <div class="fx-pairmap-editor" role="group" aria-label="Crossfeed pair map editor">
          {draft.map((p, i) => (
            <div class="fx-pairmap-pair" key={i}>
              <span class="fx-pairmap-name">Pair {i + 1}</span>
              <select aria-label={`Pair ${i + 1}, one side`} value={String(p.left)} onChange={(e) => setSide(i, "left", Number((e.target as HTMLSelectElement).value))}>
                {outputs.map((o) => (
                  <option key={o} value={String(o)}>
                    {`${o + 1} — ${props.outputName(o)}`}
                  </option>
                ))}
              </select>
              <span class="fx-help" aria-hidden="true">
                ↔
              </span>
              <select aria-label={`Pair ${i + 1}, other side`} value={String(p.right)} onChange={(e) => setSide(i, "right", Number((e.target as HTMLSelectElement).value))}>
                {outputs.map((o) => (
                  <option key={o} value={String(o)}>
                    {`${o + 1} — ${props.outputName(o)}`}
                  </option>
                ))}
              </select>
            </div>
          ))}
          {valid ? null : <NoticeRow kind="warning">{warning}</NoticeRow>}
          <div class="fx-pairmap-buttons">
            <button type="button" onClick={() => setEditing(false)}>
              Cancel
            </button>
            <button type="button" class="primary" disabled={!valid || !writable} onClick={() => void apply()}>
              Apply
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
