// The input pages of one session (WP WB6g): the live inputs by pair, each with
// its trim (the registry's `pre`, "Input trim") and the pair's L/R link (M48)
// with which side to keep when the two differ; and the input sources' names,
// which the DSPi keeps. The link's header (`InputHeader`) also heads an
// input's page on the EQ screen, as the Console's `InputChannelHeader` heads
// its input pages, and the matrix's 8-channel layout carries the trims
// (`InputTrimField`).
//
// Every write is the core's: a linked pair's trim is one call and one undo
// step (`inputPairPreamp`), a lone input's `write` of `pre`, a name `write`
// of `in.name`. A read-only session shows it all, the controls off.

import { useEffect, useState } from "preact/hooks";
import type { DspState, ParamInfo } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { Ask, ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { ValueField } from "../eq/Field";
import { inputColour } from "../eq/model"; // -- WP WEB2 --
import { dragProps, useLiveWrite } from "../main/live";
import { inputRows, liveInputs, type ChannelRow } from "../main/model";
import { clamp, compactGain, parseLevel, stepGrid } from "../outputs/model";
import {
  NAME_MAX,
  builtInTitle,
  cleanName,
  inputLinks,
  linkPair,
  mismatchQuestion,
  pairAvailable,
  pairLabel,
  pairMismatch,
  sourceOptions,
  sourceTitle,
  writeTrim,
} from "./model";
import { EmptyState, rangeFill } from "../../ui/design"; // -- WP WEB --
import { Pill } from "../../ui/Pill"; // -- WP WEB3 --
import "./inputs.css"; // -- WP WEB3 --

/** The registry's `pre` range until `describe` answers. */
export const TRIM_RANGE: [number, number] = [-60, 24];

/** Re-render on the session's link changes; answers the links' mask. */
export function useLinks(session: Session): number {
  const links = inputLinks(session);
  const [, bump] = useState(0);
  useEffect(() => links.watch(() => bump((n) => n + 1)), [links]);
  return links.mask;
}

/** The trim's range, the registry's. */
function useTrimRange(session: Session): [number, number] {
  const [r, setR] = useState<[number, number]>(TRIM_RANGE);
  useEffect(() => {
    let alive = true;
    session.core.call("describe", { path: "pre" }).then(
      (i: ParamInfo) => alive && i.min !== undefined && i.max !== undefined && setR([i.min, i.max]),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [session]);
  return r;
}

/** How many inputs USB streams (the device's status), read once. */
function useUsbActive(session: Session): number | null {
  const [n, setN] = useState<number | null>(null);
  useEffect(() => {
    let alive = true;
    session.core.call("meters", undefined).then(
      (m) => alive && setN(m.activeInputChannels),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [session]);
  return n;
}

/**
 * Link or unlink `input`'s pair, as the Console's Link button: unlinking only
 * clears the link; linking a pair that differs asks which side to keep first.
 */
export async function toggleLink(session: Session, ask: Ask, input: number): Promise<void> {
  const links = inputLinks(session);
  const s = session.state.snapshot?.state;
  if (!s || !pairAvailable(s, input) || session.readOnlyReason) return;
  if (links.isLinked(input)) {
    links.set(input, false);
    return;
  }
  const m = pairMismatch(s, input);
  if (!m.bands && !m.preamp) {
    await linkPair(session, input, null);
    return;
  }
  const q = mismatchQuestion(input, m);
  const a = await ask({
    title: q.title,
    body: <p>{q.text}</p>,
    buttons: [
      { label: q.keepThis, value: "this" },
      { label: q.keepOther, value: "other" },
      { label: "Cancel", value: "cancel" },
    ],
  });
  if (a === "this") await linkPair(session, input, input);
  else if (a === "other") await linkPair(session, input, input ^ 1);
}

/** The pair's Link toggle ("Link 1/2"). */
function LinkButton(props: { session: Session; ask: Ask; input: number; linked: boolean; disabled: boolean }) {
  const label = pairLabel(props.input);
  return (
    <button
      type="button"
      class={props.linked ? "tool-chip input-link on" : "tool-chip input-link"}
      aria-pressed={props.linked}
      aria-label={`Link ${label}`}
      title={props.linked ? `Unlink ${label} (trim and PEQ stay independent)` : `Link ${label} (trim and PEQ edits mirrored)`}
      disabled={props.disabled}
      onClick={() => void toggleLink(props.session, props.ask, props.input)}
    >
      Link {label}
    </button>
  );
}

/** One input's trim: a slider and its typed field, through the link. */
function TrimRow(props: { session: Session; row: ChannelRow; db: number; range: [number, number]; disabled: boolean }) {
  const { session, row, range, disabled } = props;
  const live = useLiveWrite((db) => writeTrim(session, row.index, db), session);
  const db = live.draft ?? props.db;
  const value = (e: Event) => Number((e.target as HTMLInputElement).value);
  const reset = (e: Event) => {
    e.preventDefault();
    if (!disabled) live.release(0);
  };
  return (
    <div class="input-trim" data-input={row.index}>
      <Pill text={row.descriptor} colour={inputColour(row.index)} class="ch-tag" />
      <span class="ch-name-text">{row.name}</span>
      <input
        type="range"
        class="ch-gain"
        aria-label={`${row.descriptor} ${row.name} Input trim`}
        aria-valuetext={`${db.toFixed(1)} dB`}
        min={range[0]}
        max={range[1]}
        step={0.1}
        value={db}
        style={rangeFill(db, range[0], range[1])}
        disabled={disabled}
        {...dragProps(live)}
        onInput={(e) => live.move(value(e))}
        onChange={(e) => live.release(value(e))}
        onDblClick={reset}
        onContextMenu={reset}
      />
      <ValueField
        label={`${row.descriptor} trim`}
        text={`${db.toFixed(1)} dB`}
        parse={(t) => parseLevel(t, "db")}
        disabled={disabled}
        onCommit={(v) => live.release(clamp(Math.round(v * 10) / 10, range))}
      />
    </div>
  );
}

/**
 * The head of an input's page (the EQ screen with an input chosen): the
 * pair's link and the input's trim, as the Console's `InputChannelHeader`.
 */
export function InputHeader(props: { session: Session; ask: Ask; s: DspState; input: number; name: string; disabled: boolean; title?: string; colour?: string }) {
  const { session, s, input } = props;
  const mask = useLinks(session);
  const range = useTrimRange(session);
  const linked = (mask & (1 << (input >> 1))) !== 0;
  const row: ChannelRow = { wire: input, index: input, descriptor: `IN${input + 1}`, name: props.name };
  return (
    <section class="input-header ch-strip" aria-label={`Input settings, ${props.name}`}>
      {/* -- WP WEB3 -- Qt's channel strip: the input's pill and name first */}
      {props.title ? (
        <span class="strip-who">
          <Pill text={`IN${input + 1}`} colour={props.colour ?? inputColour(input)} />
          <span class="strip-name">{props.title}</span>
        </span>
      ) : null}
      {pairAvailable(s, input) ? <LinkButton session={session} ask={props.ask} input={input} linked={linked} disabled={props.disabled} /> : null}
      <TrimRow session={session} row={row} db={s.inputPreampDb[input] ?? 0} range={range} disabled={props.disabled} />
    </section>
  );
}

/** An input's trim in the matrix's 8-channel layout (`Matrix.row.N.trim`): typed, the wheel by 0.5 dB, through the link. */
export function InputTrimField(props: { session: Session; input: number; name: string; db: number; disabled: boolean }) {
  const { session, input } = props;
  const live = useLiveWrite((db) => writeTrim(session, input, db));
  const shown = live.draft ?? props.db;
  return (
    <span
      class="m-field m-trim"
      onWheel={(e) => {
        if (props.disabled || e.deltaY === 0) return;
        e.preventDefault();
        live.release(stepGrid(shown, e.deltaY < 0 ? 1 : -1, 0.5, TRIM_RANGE));
      }}
    >
      <ValueField
        label={`Input trim (preamp) for ${props.name}`}
        // -- WP WEB3 -- (REVD W11) a bare number as the mockup and Qt: "0", "+3.5" (the unit is the column's)
        text={compactGain(shown).replace(/ dB$/, "")}
        parse={(t) => parseLevel(t, "db")}
        disabled={props.disabled}
        onCommit={(v) => live.release(clamp(Math.round(v * 10) / 10, TRIM_RANGE))}
      />
    </span>
  );
}

/** The input sources' names: typed, kept on the DSPi (`in.name`) when the field is left or Enter is pressed. */
function SourceNames(props: { session: Session; s: DspState; disabled: boolean; names: string[] | null; supported: boolean; reload: () => void }) {
  const { session, s, names } = props;
  const [drafts, setDrafts] = useState<Record<number, string>>({});
  if (!props.supported) {
    return (
      <section class="source-names" aria-label="Source names">
        <h3 class="section-label">Source names</h3>
        <p class="note">This DSPi's firmware keeps no source names.</p>
      </section>
    );
  }
  const save = async (source: number) => {
    const text = drafts[source];
    if (text === undefined) return;
    const clean = cleanName(text);
    setDrafts((d) => {
      const n = { ...d };
      delete n[source];
      return n;
    });
    if (clean === (names?.[source] ?? "")) return;
    await session.write("in.name", { text: clean }, [source]);
    props.reload();
  };
  return (
    <section class="source-names" aria-label="Source names">
      <h3 class="section-label">Source names</h3>
      <ul class="source-rows">
        {sourceOptions(s).map((src) => {
          const title = builtInTitle(s, src);
          const stored = names?.[src] ?? "";
          return (
            <li key={src} class={src === s.inputSource ? "source-row active" : "source-row"}>
              <label>
                <span class="source-title">
                  {title}
                  {/* -- WP WEB3 -- a status chip, not accent words */}
                  {src === s.inputSource ? (
                    <span class="source-active">
                      <span class="status-dot ok" aria-hidden="true" />
                      Active
                    </span>
                  ) : null}
                </span>
                <input
                  type="text"
                  autoComplete="off"
                  spellcheck={false}
                  maxLength={NAME_MAX}
                  placeholder={title}
                  aria-label={`Name of ${title}`}
                  value={drafts[src] ?? stored}
                  disabled={props.disabled || names === null}
                  onInput={(e) => {
                    const v = (e.target as HTMLInputElement).value;
                    setDrafts((d) => ({ ...d, [src]: v }));
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void save(src);
                    else if (e.key === "Escape")
                      setDrafts((d) => {
                        const n = { ...d };
                        delete n[src];
                        return n;
                      });
                  }}
                  onBlur={() => void save(src)}
                />
              </label>
            </li>
          );
        })}
      </ul>
      <p class="note">Stored on the DSPi, up to {NAME_MAX} characters; an empty name gives the source its own name back.</p>
    </section>
  );
}

/** The session's source names, read again whenever the device's state changes (a rename from another console notifies). */
export function useSourceNames(session: Session): { names: string[] | null; supported: boolean; reload: () => void } {
  const st = useSessionState(session);
  const [r, setR] = useState<{ names: string[] | null; supported: boolean }>({ names: null, supported: true });
  const [rev, setRev] = useState(0);
  useEffect(() => {
    let alive = true;
    session.core.call("inputSourceNames", undefined).then(
      (x) => alive && setR({ names: x.names, supported: x.supported }),
      () => alive && setR((cur) => ({ names: cur.names, supported: cur.supported })),
    );
    return () => {
      alive = false;
    };
  }, [session, st.snapshot, rev]);
  return { ...r, reload: () => setRev((n) => n + 1) };
}

export function InputsScreen(props: ScreenProps) {
  const { session, ask } = props;
  const st = useSessionState(session);
  const s = st.snapshot?.state;
  const ro = session.readOnlyReason !== "";
  const mask = useLinks(session);
  const range = useTrimRange(session);
  const usbActive = useUsbActive(session);
  const names = useSourceNames(session);
  if (!s) return <EmptyState icon="chart-line" title="Reading the inputs…" text="This DSPi's inputs are not known yet." />; // -- WP WEB --
  const rows = inputRows(s, liveInputs(s, usbActive));
  const pairs: ChannelRow[][] = [];
  for (const r of rows) {
    const p = r.index >> 1;
    (pairs[p] ??= []).push(r);
  }
  return (
    <div class="inputs-screen">
      {/* -- WP WEB3 -- two sections in the 760 px column, no cards */}
      <section class="inputs-card" aria-label="Input trims">
        <div class="ds-section-head">
          <h3 class="section-label">Inputs</h3>
          <span class="inputs-source">Source: {sourceTitle(s, s.inputSource, names.names ?? [])}</span>
        </div>
        {pairs.map((pair, p) => {
          const first = pair[0]!;
          const linked = (mask & (1 << p)) !== 0;
          const both = pair.length === 2 && pairAvailable(s, first.index);
          return (
            <div key={p} class={both && linked ? "input-pair linked" : "input-pair"} role="group" aria-label={`Inputs ${pairLabel(first.index)}`}>
              {both ? <LinkButton session={session} ask={ask} input={first.index} linked={linked} disabled={ro} /> : null}
              {pair.map((r) => (
                <TrimRow key={r.wire} session={session} row={r} db={s.inputPreampDb[r.index] ?? 0} range={range} disabled={ro} />
              ))}
            </div>
          );
        })}
        <p class="note">A linked pair moves together: its trims and its PEQ edits go to both inputs, as one undo step.</p>
      </section>
      <SourceNames session={session} s={s} disabled={ro} names={names.names} supported={names.supported} reload={names.reload} />
    </div>
  );
}
