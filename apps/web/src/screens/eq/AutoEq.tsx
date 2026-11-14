// WP P1WB2: AutoEQ with a preview before Apply (PRO1 T6, the core's P1AQ). The page holds the
// profile: one entry of the AutoEQ database as a JSON file (the database search arrives with
// WB6k). The installer picks the channel pair (any pair of inputs or of outputs), previews,
// and sees the result as a dashed curve on the graph (the core's `autoeqPreview`, nothing
// written) with where the level goes, the suggested trim, and what the limits held. Apply is
// one undo step (P1UG) through the limits (P1LM); an output pair keeps its calibrated gain
// unless asked. The state before is kept as a snapshot document, to download.

import { useEffect, useRef, useState } from "preact/hooks";
import type { AutoEqPreviewRecord } from "../../core/protocol";
import type { Session } from "../../ui/session";
import { noticeOf } from "../../ui/texts";
import { download } from "../snapshots/model";

/** A channel as the EQ screen lists it. */
export interface PairChannel {
  wire: number;
  descriptor: string;
  name: string;
  output: boolean;
}

/** The pairs AutoEQ may take: neighbours among the inputs, and among the outputs. */
export function autoEqPairs(channels: PairChannel[]): { left: PairChannel; right: PairChannel; label: string }[] {
  const pairs: { left: PairChannel; right: PairChannel; label: string }[] = [];
  for (const side of [false, true]) {
    const list = channels.filter((c) => c.output === side);
    for (let i = 0; i + 1 < list.length; i += 2) {
      const left = list[i]!;
      const right = list[i + 1]!;
      pairs.push({ left, right, label: `${left.descriptor} + ${right.descriptor}` });
    }
  }
  return pairs;
}

/** The preview's level line, in words. */
export function levelLine(p: AutoEqPreviewRecord): string {
  const trim = `${p.suggestedTrimDb > 0 ? "+" : ""}${p.suggestedTrimDb.toFixed(1)} dB`;
  if (p.levelTarget === "input_preamp") return `The inputs' preamp is set to the profile's ${trim}.`;
  if (p.levelTarget === "output_gain") return `The outputs' gain is set to the profile's ${trim}.`;
  return `The outputs' gain is kept. Suggested trim: ${trim}, if the correction clips.`;
}

export function AutoEqPanel(props: {
  session: Session;
  channels: PairChannel[];
  /** The channel the graph edits: the preview is drawn for it when it is in the pair. */
  wire: number;
  disabled: boolean;
  onPreview: (p: AutoEqPreviewRecord | null) => void;
}) {
  const { session } = props;
  const pairs = autoEqPairs(props.channels);
  const near = Math.max(
    0,
    pairs.findIndex((p) => p.left.wire === props.wire || p.right.wire === props.wire),
  );
  const [pair, setPair] = useState(near);
  const [profile, setProfile] = useState<{ name: string; entry: unknown } | null>(null);
  const [applyLevel, setApplyLevel] = useState(false);
  const [preview, setPreview] = useState<AutoEqPreviewRecord | null>(null);
  const [outcome, setOutcome] = useState<{ lines: string[]; before: string; tone: "info" | "warn" | "error" } | null>(null);
  const [busy, setBusy] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  const chosen = pairs[pair] ?? pairs[0];
  const outputs = !!chosen?.left.output;
  useEffect(() => () => props.onPreview(null), []);

  const show = (p: AutoEqPreviewRecord | null) => {
    setPreview(p);
    props.onPreview(p);
  };
  const params = () => ({ profile: profile!.entry, left: chosen!.left.wire, right: chosen!.right.wire, ...(outputs ? { applyLevel } : {}) });
  const failed = (e: unknown) => {
    const n = noticeOf(e);
    setOutcome({ lines: [n ? `${n.title}: ${n.reason} ${n.detail}`.trim() : (e as Error).message ?? String(e)], before: "", tone: "error" });
  };

  const load = async (input: HTMLInputElement) => {
    const f = input.files?.[0];
    input.value = "";
    if (!f) return;
    try {
      const entry = JSON.parse(await f.text()) as { manufacturer?: string; model?: string };
      setProfile({ name: [entry.manufacturer, entry.model].filter(Boolean).join(" ") || f.name, entry });
      show(null);
      setOutcome(null);
    } catch {
      setOutcome({ lines: [`${f.name} is not an AutoEQ profile (JSON).`], before: "", tone: "error" });
    }
  };

  const doPreview = async () => {
    if (!profile || !chosen || busy) return;
    setBusy(true);
    try {
      show(await session.core.call("autoeqPreview", params()));
      setOutcome(null);
    } catch (e) {
      show(null);
      failed(e);
    } finally {
      setBusy(false);
    }
  };

  const doApply = async () => {
    if (!profile || !chosen || busy || props.disabled) return;
    setBusy(true);
    try {
      const r = await session.core.call("autoeqApply", params());
      await session.refresh();
      show(null);
      const p = r.preview;
      setOutcome({
        lines: [`Applied ${profile.name} to ${chosen.label}.`, levelLine(p), ...p.notes, ...r.notes],
        before: r.snapshotDocument,
        tone: p.held.length || p.notes.length ? "warn" : "info",
      });
    } catch (e) {
      failed(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <section class="card autoeq" aria-label="AutoEQ">
      <h3>AutoEQ</h3>
      <div class="autoeq-row">
        <button type="button" onClick={() => file.current?.click()}>
          Profile…
        </button>
        <input ref={file} type="file" hidden accept=".json,application/json" aria-label="AutoEQ profile" onChange={(e) => void load(e.target as HTMLInputElement)} />
        <span class="autoeq-name">{profile ? profile.name : "No profile chosen"}</span>
      </div>
      <div class="autoeq-row">
        <label>
          Channel pair{" "}
          <select
            value={String(pair)}
            onChange={(e) => {
              setPair(Number((e.target as HTMLSelectElement).value));
              show(null);
            }}
          >
            {pairs.map((p, i) => (
              <option key={p.label} value={String(i)}>
                {p.label}
              </option>
            ))}
          </select>
        </label>
        {outputs ? (
          <label>
            <input
              type="checkbox"
              checked={applyLevel}
              onChange={(e) => {
                setApplyLevel((e.target as HTMLInputElement).checked);
                show(null);
              }}
            />{" "}
            Set the outputs' gain too
          </label>
        ) : null}
      </div>
      <div class="autoeq-row">
        <button type="button" disabled={!profile || busy} onClick={() => void doPreview()}>
          Preview
        </button>
        <button type="button" disabled={!profile || busy || props.disabled} onClick={() => void doApply()}>
          Apply
        </button>
      </div>
      {preview ? (
        <div class="autoeq-preview" role="status" aria-label="AutoEQ preview">
          <p>
            Preview of {profile?.name} on {chosen?.label}: dashed on the graph. Nothing is written until Apply.
          </p>
          <p>{levelLine(preview)}</p>
          {preview.held.length ? (
            <ul aria-label="Held by the limits">
              {preview.held.map((h, i) => (
                <li key={i}>{h}</li>
              ))}
            </ul>
          ) : null}
          {preview.notes.map((n, i) => (
            <p key={i} class="note">
              {n}
            </p>
          ))}
        </div>
      ) : null}
      {outcome ? (
        <div class={`autoeq-outcome ${outcome.tone}`} role={outcome.tone === "error" ? "alert" : "status"} aria-label="AutoEQ result">
          {outcome.lines.map((l, i) => (
            <p key={i}>{l}</p>
          ))}
          {outcome.before ? (
            <button type="button" onClick={() => download(outcome.before, "before-autoeq.dspipreset")}>
              Download the state before
            </button>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
