// The matrix mixer of one session (WP WB6c, B10), -- WP MXW -- as DESREV's redesign and MX1's Qt
// panel draw it: inputs in rows feed outputs in columns, and the corner says so. A connected
// crosspoint is a fixed tile in the input's colour with its gain inside and a ø when its phase is
// inverted; an empty one is a dot. Each row ends with the outputs it feeds, each column with its
// gain, delay and the inputs that feed it; enable and mute sit in the column's head, an input's trim
// in its row's. The keys move, connect, type a gain, step it, invert, mute and enable; the
// inspector beside the page follows the cell, the column or the row chosen. "Link L/R pairs"
// mirrors an edit of a stereo pair's crosspoint to its partner (`mirrorOf`) as one undo step;
// Direct 1:1 and Clear are one step each, and Clear asks before more than four crosspoints go.
// Enabling an output that shares Core 1 with the PDM sub asks first, in the Console's words, and
// switches over as the Console does. A column's menu renames the output, identifies it
// (-- WP WBCLIP -- and copies or pastes its parameters).
//
// Every change is the core's, read back: crosspoints by `crosspoint` (the registry's `mix`), the
// rest by `write`. The delay follows the owner's rule (SL1): typed values kept, device values shown
// as they are, the wheel to the next whole ms. On a phone the grid scrolls inside its box, never
// the page. A read-only session shows it all, the controls off.

import type { ComponentChildren } from "preact";
import { memo } from "preact/compat";
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import type { DspState, OutputLabel } from "../../core/protocol";
import { safeLocalStorage } from "../../fleet/local";
import { useSessionState } from "../../ui/components";
import { ChipGroup, EmptyState, Glyph, rangeFill } from "../../ui/design";
import { Inspect, InspectorSection } from "../../ui/inspector/Inspector";
import { ownRow, type MenuItem } from "../../ui/menus/actions";
import { isMenuKey, useContextMenu } from "../../ui/menus/ActionMenu";
import { readNumber } from "../../ui/number";
import type { Ask, ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { PageTools } from "../../ui/shell/Parts";
import { passes, passName } from "../eq/align";
import { ValueField } from "../eq/Field";
import { channelColour, freqText, inputColour } from "../eq/model";
import { RenameDialog } from "../main/Dialog";
import { useLiveWrite } from "../main/live";
import { Menu } from "../main/Menu";
// -- WP WB6J --
import { identifyOutput } from "../analysis/siggen";
import { inputRows, liveInputs, outputLabel, type ChannelRow } from "../main/model";
import { useSolo } from "../../ui/solo"; // -- WP GAP2 --
import { CROSSPOINT_RANGE } from "../outputs/OutputCard";
import { clamp, compactGain, delayText, enableConflict, eqWorkerRange, levelText, limiterSupported, outputSlotName, parseLevel, pdmOutput, stepGrid } from "../outputs/model";
// -- WP WB6G -- the inputs' trims
import { InputTrimField } from "../inputs/InputsScreen";
// -- WP WBCLIP -- the column menu's Copy and Paste Parameters (M45)
import { clipEntries, useClipboard } from "../clipboard/model";
import type { ChainTarget } from "../system/chain";
import { CLEAR_ASKS_ABOVE, DENSE_FROM, feedsOf, linkPairsOf, mirrorOf, moveCell, setLinkPairs, sourcesOf, stepGain, stepSign, summaryText, tailText, tileGain, type CellAt } from "./model";

/** The output gain's range until the registry says (`describe out.gain`). */
const OUT_GAIN: [number, number] = [-60, 12];
/** A second click on the chosen tile disconnects it, unless it was the start of a double click. */
const DOUBLE_MS = 260;

export function MatrixScreen(props: ScreenProps) {
  const { session, ask } = props;
  const st = useSessionState(session);
  const s = st.snapshot?.state;
  const ro = session.readOnlyReason !== "";

  const [usbActive, setUsbActive] = useState<number | null>(null);
  const [maxDelay, setMaxDelay] = useState(21);
  const [gainRange, setGainRange] = useState<[number, number]>(OUT_GAIN);
  useEffect(() => {
    let alive = true;
    session.core.call("meters", undefined).then(
      (m) => alive && setUsbActive(m.activeInputChannels),
      () => undefined,
    );
    session.core.call("outputLimits", undefined).then(
      (l) => alive && setMaxDelay(l.maxDelayMs),
      () => undefined,
    );
    // -- WP MXW -- the output gain's own range (it was the crosspoint's)
    session.core.call("describe", { path: "out.gain" }).then(
      (g) => alive && g?.min !== undefined && g.max !== undefined && setGainRange([g.min, g.max]),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [session]);

  if (!s || !Array.isArray(s.matrixRouting) || !s.matrixRouting.length)
    return <EmptyState icon="network" title="Reading the routing…" text="This DSPi's matrix is not known yet." />;
  return (
    <MatrixView
      session={session}
      ask={ask}
      s={s}
      ro={ro}
      usbActive={usbActive}
      maxDelay={maxDelay}
      gainRange={gainRange}
      labels={st.snapshot?.outputLabels}
      serial={st.snapshot?.device.serial || session.id}
      open={props.open}
    />
  );
}

type Sel = { kind: "none" } | { kind: "cell" } | { kind: "row"; i: number } | { kind: "col"; o: number };

interface Out {
  index: number;
  wire: number;
  descriptor: string;
  name: string;
  stored: string;
  slot: string;
  enabled: boolean;
  muted: boolean;
}

interface ViewProps {
  session: Session;
  ask: Ask;
  s: DspState;
  ro: boolean;
  usbActive: number | null;
  maxDelay: number;
  gainRange: [number, number];
  // -- BUG1 -- labels: the outputs' labels, the snapshot's
  labels?: OutputLabel[];
  serial: string;
  open?: (t: ChainTarget) => void;
}

const MatrixView = memo(
  function MatrixView(props: ViewProps) {
    const { session, ask, s, ro } = props;
    const inputs = inputRows(s, liveInputs(s, props.usbActive));
    const nIn = inputs.length;
    const nOut = s.numOutputChannels;
    const outs: Out[] = Array.from({ length: nOut }, (_, o) => ({
      index: o,
      wire: s.numInputChannels + o,
      descriptor: `OUT${o + 1}`,
      name: outputLabel(s, props.labels, o),
      stored: s.channelNames[s.numInputChannels + o] ?? `Output ${o + 1}`,
      slot: props.labels?.[o]?.slotName || outputSlotName(s, o),
      enabled: s.outputEnabled[o] ?? false,
      muted: s.outputMuted[o] ?? false,
    }));
    const [renaming, setRenaming] = useState<Out | null>(null);
    const board = useClipboard(session); // -- WP WBCLIP --
    const [link, setLink] = useState(() => linkPairsOf(safeLocalStorage(), props.serial));
    const [denseChosen, setDenseChosen] = useState(false);
    const dense = denseChosen || nOut >= DENSE_FROM;
    const [cur, setCur] = useState<CellAt>({ i: 0, o: 0 });
    const [sel, setSel] = useState<Sel>({ kind: "none" });
    const [editing, setEditing] = useState<{ i: number; o: number; text: string; all: boolean } | null>(null);
    const [hover, setHover] = useState<CellAt | null>(null);
    const grid = useRef<HTMLTableElement>(null);
    const wantFocus = useRef(false);
    const pendingOff = useRef<number | null>(null);
    const menu = useContextMenu();
    const openRef = useRef(props.open);
    openRef.current = props.open;
    const live = () => session.state.snapshot?.state ?? s;
    const at = (i: number, o: number) => {
      const n = live();
      return { on: n.matrixRouting[i]?.[o] ?? false, gain: n.matrixGain[i]?.[o] ?? 0, inv: n.matrixInvert[i]?.[o] ?? false };
    };

    // The cell back in focus after the keys moved it or an edit ended there
    useLayoutEffect(() => {
      if (!wantFocus.current) return;
      wantFocus.current = false;
      grid.current?.querySelector<HTMLElement>(`[data-cell="${cur.i}:${cur.o}"]`)?.focus();
    });
    useEffect(() => () => void (pendingOff.current !== null && clearTimeout(pendingOff.current)), []);

    /** One crosspoint, and with Link L/R pairs its partner as a copy of it, in one undo step. */
    const route = (i: number, o: number, p: { enabled?: boolean; gain?: number; invert?: boolean }) => {
      const c = at(i, o);
      const cell = { enabled: p.enabled ?? c.on, gain: p.gain ?? c.gain, invert: p.invert ?? c.inv };
      const one = (input: number, output: number) => session.change("crosspoint", { input, output, ...cell });
      const m = link ? mirrorOf(i, o, nIn, nOut) : null;
      if (!m) return one(i, o);
      return session.group("Linked Routing", async () => (await one(i, o)) && (await one(m.i, m.o)));
    };

    const enable = async (o: number) => {
      if (live().outputEnabled[o]) return void session.write("out.enable", { bool: false }, [o]);
      const conflict = enableConflict(live(), o);
      if (!conflict) return void session.write("out.enable", { bool: true }, [o]);
      const [a, b] = eqWorkerRange(s);
      const pdm = conflict === "pdm";
      const answer = await ask({
        title: "Warning",
        body: <p>{pdm ? `Outputs ${a + 1}-${b + 1} will be disabled. Are you sure?` : "The PDM output will be disabled. Are you sure?"}</p>,
        buttons: [
          { label: pdm ? "Enable PDM" : "Disable PDM", value: "go" },
          { label: "Cancel", value: "cancel" },
        ],
      });
      if (answer !== "go") return;
      // As the Console switches over (`switchToPDM`, `switchFromPDM`): the others off first.
      const off = pdm ? Array.from({ length: b - a + 1 }, (_, k) => a + k) : [pdmOutput(s)];
      await session.writeAll([...off.map((x) => ({ path: "out.enable", value: { bool: false }, indices: [x] })), { path: "out.enable", value: { bool: true }, indices: [o] }]);
    };
    const mute = (o: number) => void session.write("out.mute", { bool: !(live().outputMuted[o] ?? false) }, [o]);

    // -- WP P1WB2 -- each one undo step (P1UG)
    const direct = () =>
      session.group("Direct 1:1", async () => {
        const n = live();
        for (const i of inputs) {
          for (const o of outs) {
            const want = o.index === i.index && o.index !== pdmOutput(s);
            if ((n.matrixRouting[i.index]?.[o.index] ?? false) !== want && !(await session.change("crosspoint", { input: i.index, output: o.index, enabled: want, gain: n.matrixGain[i.index]?.[o.index] ?? 0, invert: n.matrixInvert[i.index]?.[o.index] ?? false })))
              return;
          }
        }
        if (live().outputEnabled[pdmOutput(s)]) await session.write("out.enable", { bool: false }, [pdmOutput(s)]);
      });
    const count = inputs.reduce((n, i) => n + feedsOf(s.matrixRouting, i.index, nOut).length, 0);
    const clearAll = async () => {
      // -- WP MXW -- asks before more than four go (MX1)
      if (count > CLEAR_ASKS_ABOVE) {
        const answer = await ask({
          title: "Clear the routing?",
          body: <p>All {count} crosspoints will be disconnected. Their gain and phase are kept.</p>,
          buttons: [
            { label: "Clear", value: "go", danger: true },
            { label: "Cancel", value: "cancel" },
          ],
        });
        if (answer !== "go") return;
      }
      await session.group("Clear routing", async () => {
        for (const i of inputs)
          for (const o of outs) {
            const c = at(i.index, o.index);
            if (c.on && !(await session.change("crosspoint", { input: i.index, output: o.index, enabled: false, gain: c.gain, invert: c.inv }))) return;
          }
      });
    };

    const go = (c: CellAt, focus = true) => {
      setCur(c);
      setSel({ kind: "cell" });
      wantFocus.current = focus;
    };
    const cancelOff = () => {
      if (pendingOff.current === null) return;
      clearTimeout(pendingOff.current);
      pendingOff.current = null;
    };
    const startEdit = (i: number, o: number, typed: string | null) => {
      cancelOff();
      if (ro) return;
      const c = at(i, o);
      if (!c.on) void route(i, o, { enabled: true });
      setCur({ i, o });
      setSel({ kind: "cell" });
      setEditing({ i, o, text: typed ?? tileGain(c.gain), all: typed === null });
    };
    const endEdit = (refocus: boolean) => {
      setEditing(null);
      wantFocus.current = refocus;
    };
    const step = (i: number, o: number, dir: 1 | -1, big: boolean) => {
      const c = at(i, o);
      if (!ro && c.on) void route(i, o, { gain: stepGain(c.gain, dir, big, CROSSPOINT_RANGE) });
    };

    const onCellClick = (i: number, o: number, e: MouseEvent) => {
      // The second click of a double click is the double click's
      if (e.detail > 1) return;
      const chosen = sel.kind === "cell" && cur.i === i && cur.o === o;
      go({ i, o });
      if (ro) return;
      const c = at(i, o);
      if (!c.on) void route(i, o, { enabled: true });
      else if (chosen) {
        cancelOff();
        pendingOff.current = window.setTimeout(() => {
          pendingOff.current = null;
          void route(i, o, { enabled: false });
        }, DOUBLE_MS);
      }
    };

    const cellMenu = (i: number, o: number): MenuItem[] => {
      const c = at(i, o);
      return [
        ownRow("matrix.connect", c.on ? "Disconnect" : "Connect", () => void route(i, o, { enabled: !c.on }), { disabled: ro }),
        ownRow("matrix.gain", "Gain…", () => startEdit(i, o, null), { disabled: ro }),
        ownRow("matrix.unity", "Gain to 0 dB", () => void route(i, o, { gain: 0 }), { disabled: ro || !c.on || c.gain === 0 }),
        ownRow("matrix.invert", "Invert Phase", () => void route(i, o, { invert: !c.inv }), { disabled: ro || !c.on, checked: c.inv }),
        "separator",
        ownRow("matrix.identify", "Identify Output", () => void identifyOutput(session, o), { disabled: ro }),
      ];
    };

    const onGridKey = (e: KeyboardEvent) => {
      const cell = (e.target as HTMLElement).dataset?.cell;
      if (!cell || editing) return;
      const mv = moveCell(cur, e.key, nIn, nOut);
      if (mv && !e.altKey) {
        e.preventDefault();
        cancelOff();
        go(mv);
        return;
      }
      const { i, o } = cur;
      if (isMenuKey(e)) return menu.open(e, `${inputs[i]?.name ?? ""} to ${outs[o]?.name ?? ""}`, cellMenu(i, o));
      if (e.key === "Escape") {
        e.preventDefault();
        setSel({ kind: "none" });
        return;
      }
      // -- WP WEB2 -- (REVD G1) + and − step only with a modifier: Alt 0.5 dB, Ctrl 3 dB (by the typed character,
      // the key's place only for a dead key: stepSign); without one they start a typed gain
      const sign = stepSign({ key: e.key, code: e.code, altGraph: e.getModifierState?.("AltGraph") }); // -- WP REVEFIX -- (REVE C2)
      if (sign && (e.ctrlKey || e.altKey) && !e.metaKey) {
        e.preventDefault();
        step(i, o, sign, e.ctrlKey);
        return;
      }
      if (e.ctrlKey || e.metaKey || e.altKey || ro) return;
      const c = at(i, o);
      if (e.key === " ") void route(i, o, { enabled: !c.on });
      else if (e.key === "Enter") startEdit(i, o, null);
      else if (/^[0-9.,+-]$/.test(e.key)) startEdit(i, o, e.key);
      else if (e.key === "i" || e.key === "I") {
        if (c.on) void route(i, o, { invert: !c.inv });
      } else if (e.key === "m" || e.key === "M") mute(o);
      else if (e.key === "e" || e.key === "E") void enable(o);
      else if (e.key === "Delete" || e.key === "Backspace") {
        if (c.on) void route(i, o, { enabled: false });
      } else return;
      e.preventDefault();
    };

    const lit = hover ?? (sel.kind === "cell" ? cur : null);
    const chosenCell = sel.kind === "cell" ? cur : null;
    const tools = (
      <PageTools label="Routing tools">
        <button
          type="button"
          class="tool-chip"
          aria-pressed={link}
          title="Mirror each change of a stereo pair's crosspoint to its partner, one column over (L→Out 1 and R→Out 2), as one undo step"
          onClick={() => {
            setLinkPairs(safeLocalStorage(), props.serial, !link);
            setLink(!link);
          }}
        >
          <Glyph name="link" />
          Link L/R pairs
        </button>
        <button type="button" disabled={ro} title="Route each input to the matching output (FL→OUT1, FR→OUT2, …) and disable the PDM sub" onClick={() => void direct()}>
          Direct 1:1
        </button>
        <button type="button" disabled={ro || count === 0} title="Disconnect every crosspoint" onClick={() => void clearAll()}>
          Clear
        </button>
        <button
          type="button"
          class="tool-chip"
          aria-pressed={dense}
          disabled={nOut >= DENSE_FROM}
          title={nOut >= DENSE_FROM ? "Dense by itself with this many outputs" : "Smaller cells, without the trims and the footer"}
          onClick={() => setDenseChosen(!denseChosen)}
        >
          Dense
        </button>
      </PageTools>
    );

    return (
      <section class={`matrix-screen${dense ? " dense" : ""}`} aria-label="Matrix Mixer">
        {tools}
        <div class="mx-scroll" role="region" aria-label="Matrix">
          <table class="mx" role="grid" aria-label="Routing matrix" aria-readonly={ro || undefined} ref={grid} onKeyDown={onGridKey} onMouseLeave={() => setHover(null)}>
            <thead>
              <tr>
                <th scope="col" class="mx-corner">
                  <span class="mx-lab" aria-hidden="true">
                    From inputs ↓
                  </span>
                  <span class="mx-lab" aria-hidden="true">
                    To outputs →
                  </span>
                  <span class="visually-hidden">Inputs, in rows, feed outputs, in columns</span>
                </th>
                <th class="mx-pair" aria-hidden="true" />
                {outs.map((o) => {
                  const conflict = !o.enabled && enableConflict(s, o.index) !== null;
                  const picked = sel.kind === "col" && sel.o === o.index;
                  return (
                    <th
                      scope="col"
                      key={o.index}
                      class={cls("mx-colhead", !o.enabled && "off", picked && "picked", lit?.o === o.index && "lit", pairEnd(o.index, nOut) && "mx-colend")}
                      style={{ "--ch": channelColour(s, o.wire) }}
                    >
                      <button type="button" class="mx-pick mx-colname" aria-pressed={picked} title={`${o.name} · ${o.slot}`} onClick={() => setSel({ kind: "col", o: o.index })}>
                        {o.name}
                      </button>
                      <span class="mx-colpill">
                        <Menu
                          label={o.descriptor}
                          disabled={ro}
                          entries={[
                            { label: "Rename…", onSelect: () => setRenaming(o) },
                            // -- WP WB6J -- Identify (M46)
                            { label: "Identify", onSelect: () => void identifyOutput(session, o.index) },
                            // -- WP WBCLIP -- Copy / Paste Parameters (M45)
                            "separator",
                            ...clipEntries(board, o.wire, ro),
                          ]}
                        />
                      </span>
                      <span class="mx-io">
                        <button
                          type="button"
                          class={cls("mx-icon", o.enabled && "on", conflict && "conflict")}
                          aria-pressed={o.enabled}
                          aria-label={`Enable ${o.name}`}
                          title={o.enabled ? "Disable output (saves CPU)" : conflict ? "Conflict: shared Core 1 resource" : "Enable output"}
                          disabled={ro}
                          onClick={() => void enable(o.index)}
                        >
                          <Glyph name="power" />
                        </button>
                        <button
                          type="button"
                          class={cls("mx-icon", o.muted && "muted")}
                          aria-pressed={o.muted}
                          aria-label={`Mute ${o.name}`}
                          title={o.muted ? "Unmute" : "Mute"}
                          disabled={ro}
                          onClick={() => mute(o.index)}
                        >
                          <Glyph name={o.muted ? "volume-x" : "volume"} />
                        </button>
                      </span>
                    </th>
                  );
                })}
                <th class="mx-tail" aria-hidden="true" />
              </tr>
            </thead>
            <tbody>
              {inputs.map((inp, r) => {
                const feeds = feedsOf(s.matrixRouting, inp.index, nOut);
                const picked = sel.kind === "row" && sel.i === inp.index;
                const partnerBelow = link && r % 2 === 0 && mirrorOf(r, 0, nIn, nOut) !== null;
                return (
                  <tr key={inp.wire} class={cls(r % 2 === 1 && "mx-stripe", r % 2 === 1 && r < nIn - 1 && "mx-pairend")}>
                    <th scope="row" class={cls("mx-rowhead", picked && "picked", lit?.i === r && "lit")}>
                      <div class="mx-rh">
                        <span class="mx-dot-in" style={{ background: channelColour(s, inp.wire) }} aria-hidden="true" />
                        <button type="button" class="mx-pick mx-rowname" aria-pressed={picked} title={`${inp.descriptor} ${inp.name}`} onClick={() => setSel({ kind: "row", i: inp.index })}>
                          {inp.name}
                        </button>
                        {dense ? null : (
                          <span class={cls("mx-trim", (s.inputPreampDb[inp.index] ?? 0) === 0 && "zero")}>
                            {/* -- WP WB6G -- each input's trim (through its pair's link), at every channel count (MX1) */}
                            <InputTrimField session={session} input={inp.index} name={`${inp.descriptor} ${inp.name}`} db={s.inputPreampDb[inp.index] ?? 0} disabled={ro} />
                          </span>
                        )}
                      </div>
                    </th>
                    <td class="mx-pair" aria-hidden="true">
                      {partnerBelow ? <Glyph name="link" class="mx-pairglyph" /> : null}
                    </td>
                    {outs.map((o) => (
                      <Cell
                        key={o.index}
                        input={inp}
                        out={o}
                        c={{ on: s.matrixRouting[inp.index]?.[o.index] ?? false, gain: s.matrixGain[inp.index]?.[o.index] ?? 0, inv: s.matrixInvert[inp.index]?.[o.index] ?? false }}
                        current={cur.i === r && cur.o === o.index}
                        chosen={chosenCell !== null && chosenCell.i === r && chosenCell.o === o.index}
                        lit={lit !== null && (lit.i === r || lit.o === o.index)}
                        colEnd={pairEnd(o.index, nOut)}
                        ro={ro}
                        editing={editing && editing.i === r && editing.o === o.index ? editing : null}
                        onClick={(e) => onCellClick(r, o.index, e)}
                        onDouble={() => startEdit(r, o.index, null)}
                        onHover={() => setHover((h) => (h && h.i === r && h.o === o.index ? h : { i: r, o: o.index }))}
                        onWheel={(e) => {
                          const d = e.deltaY || e.deltaX;
                          if (ro || d === 0 || !at(r, o.index).on) return;
                          e.preventDefault();
                          step(r, o.index, d < 0 ? 1 : -1, e.shiftKey);
                        }}
                        onMenu={(e) => {
                          go({ i: r, o: o.index }, false);
                          menu.open(e, `${inp.name} to ${o.name}`, cellMenu(r, o.index));
                        }}
                        onInvert={() => void route(r, o.index, { invert: !at(r, o.index).inv })}
                        commit={(v) => void route(r, o.index, { enabled: true, gain: clamp(v, CROSSPOINT_RANGE) })}
                        endEdit={endEdit}
                      />
                    ))}
                    <td class="mx-tail">{feeds.length ? tailText(feeds) : <span class="mx-none">{tailText(feeds)}</span>}</td>
                  </tr>
                );
              })}
            </tbody>
            {dense ? null : (
              <tfoot>
                <tr class="mx-foot mx-foot-first">
                  <th scope="row" class="mx-footlab">
                    <span class="mx-lab">Gain</span> <span class="mx-unit">dB</span>
                  </th>
                  <td class="mx-pair" aria-hidden="true" />
                  {outs.map((o) => (
                    <td key={o.index} class={cls("mx-footcell", !o.enabled && "off", pairEnd(o.index, nOut) && "mx-colend")}>
                      <CompactField
                        label={`${o.name} gain`}
                        value={s.outputGainDb[o.index] ?? 0}
                        text={(v) => compactGain(v).replace(/ dB$/, "")}
                        unit="db"
                        range={props.gainRange}
                        step={0.1}
                        disabled={ro}
                        write={(v) => session.write("out.gain", { float: v }, [o.index])}
                      />
                    </td>
                  ))}
                  <td class="mx-tail" aria-hidden="true" />
                </tr>
                <tr class="mx-foot">
                  <th scope="row" class="mx-footlab">
                    <span class="mx-lab">Delay</span> <span class="mx-unit">ms</span>
                  </th>
                  <td class="mx-pair" aria-hidden="true" />
                  {outs.map((o) => (
                    <td key={o.index} class={cls("mx-footcell", !o.enabled && "off", pairEnd(o.index, nOut) && "mx-colend")}>
                      <CompactField
                        label={`${o.name} delay`}
                        value={s.outputDelayMs[o.index] ?? 0}
                        text={footDelay}
                        unit="ms"
                        range={[0, props.maxDelay]}
                        step={1}
                        disabled={ro}
                        write={(v) => session.write("out.delay", { float: v }, [o.index])}
                      />
                    </td>
                  ))}
                  <td class="mx-tail" aria-hidden="true" />
                </tr>
                <tr class="mx-foot">
                  <th scope="row" class="mx-footlab">
                    <span class="mx-lab">Fed by</span>
                  </th>
                  <td class="mx-pair" aria-hidden="true" />
                  {outs.map((o) => {
                    const from = sourcesOf(s.matrixRouting, o.index, nIn);
                    const names = from.map((i) => inputs[i]!.name);
                    return (
                      <td key={o.index} class={cls("mx-src", !o.enabled && "off", pairEnd(o.index, nOut) && "mx-colend")} title={names.length ? `Fed by ${names.join(", ")}` : "Fed by nothing"}>
                        {from.slice(0, 3).map((i) => (
                          <i key={i} style={{ background: channelColour(s, inputs[i]!.wire) }} aria-hidden="true" />
                        ))}
                        <span>{from.length === 1 ? names[0] : from.length > 1 ? String(from.length) : "–"}</span>
                      </td>
                    );
                  })}
                  <td class="mx-tail" aria-hidden="true" />
                </tr>
              </tfoot>
            )}
          </table>
        </div>
        {/* -- WP WEB2 -- (REVD W11) the grid whole: the inspector under it when both do not fit */}
        <Inspect pageMin={gridWidth(nOut, dense)}>
          <MatrixInspector
            session={session}
            s={s}
            ro={ro}
            inputs={inputs}
            outs={outs}
            sel={sel}
            cur={cur}
            link={link}
            gainRange={props.gainRange}
            maxDelay={props.maxDelay}
            route={route}
            enable={(o) => void enable(o)}
            mute={mute}
            openEq={(wire) => openRef.current?.({ screen: "eq", wire })}
          />
        </Inspect>
        {menu.menu}
        {renaming ? (
          <RenameDialog
            title={`Rename ${renaming.name}`}
            label="Name"
            initial={renaming.stored}
            onCancel={() => setRenaming(null)}
            onSave={(name) => {
              const wire = renaming.wire;
              setRenaming(null);
              if (name.trim()) void session.write("ch.name", { text: name }, [wire]);
            }}
          />
        ) : null}
      </section>
    );
  },
  (a, b) =>
    a.session === b.session &&
    a.ask === b.ask &&
    a.ro === b.ro &&
    a.usbActive === b.usbActive &&
    a.maxDelay === b.maxDelay &&
    a.gainRange[0] === b.gainRange[0] &&
    a.gainRange[1] === b.gainRange[1] &&
    a.serial === b.serial &&
    matrixKey(a.s) === matrixKey(b.s) &&
    JSON.stringify(a.labels) === JSON.stringify(b.labels),
);

/** -- WP WEB2 -- (REVD W11) The grid's width at `nOut` outputs: row heads, the pair column, the cells, the tails (CSS's sizes). */
function gridWidth(nOut: number, dense: boolean): number {
  return dense ? 120 + 12 + nOut * 44 + 80 : 168 + 12 + nOut * 56 + 90;
}

/** -- WP WEB2 -- (REVD W11) A delay in the footer: none is "0" as the mockup and Qt, else as the delay fields ("4.0"). */
function footDelay(ms: number): string {
  return Math.round(ms * 1e4) === 0 ? "0" : delayText(ms).replace(/ ms$/, "");
}

function cls(...names: (string | false | null | undefined)[]): string {
  return names.filter(Boolean).join(" ");
}

/** A hairline after each output pair (DESREV), not after the last column. */
function pairEnd(o: number, n: number): boolean {
  return o % 2 === 1 && o < n - 1;
}

/** What the matrix shows; the meters and the rest of the state do not reach it. */
function matrixKey(s: DspState): string {
  return JSON.stringify([
    s.matrixRouting,
    s.matrixGain,
    s.matrixInvert,
    s.outputEnabled,
    s.outputGainDb,
    s.outputDelayMs,
    s.outputMuted,
    s.channelNames,
    s.numInputChannels,
    s.numOutputChannels,
    s.inputSource,
    s.i2sInputChannels,
    s.platformId,
    s.outputTypes,
    s.inputPreampDb, // -- WP WB6G -- the trims
    // -- WP MXW -- the inspector's output line
    s.limiterEnabled,
    s.limiterThresholdDb,
    s.xover,
  ]);
}

/** A crosspoint: a tile when connected (its gain inside, ø when inverted), a dot when not; never another size. */
const Cell = memo(function Cell(props: {
  input: ChannelRow;
  out: Out;
  c: { on: boolean; gain: number; inv: boolean };
  /** The grid's one tab stop. */
  current: boolean;
  /** Chosen for the inspector. */
  chosen: boolean;
  lit: boolean;
  colEnd: boolean;
  ro: boolean;
  editing: { text: string; all: boolean } | null;
  onClick: (e: MouseEvent) => void;
  onDouble: () => void;
  onHover: () => void;
  onWheel: (e: WheelEvent) => void;
  onMenu: (e: MouseEvent) => void;
  onInvert: () => void;
  commit: (v: number) => void;
  endEdit: (refocus: boolean) => void;
}) {
  const { input, out, c } = props;
  const where = `${input.name} to ${out.name}`;
  const state = c.on ? `connected, ${compactGain(c.gain)}${c.inv ? ", phase inverted" : ""}` : "not connected";
  const colour = inputColour(input.index); // -- WP WEB2 --
  const tint = `color-mix(in srgb, ${colour} 22%, var(--panel))`;
  return (
    <td
      role="gridcell"
      data-cell={`${input.index}:${out.index}`}
      data-connected={c.on}
      tabIndex={props.current ? 0 : -1}
      aria-selected={props.chosen}
      aria-label={`${where}, ${state}`}
      title={`${input.name} → ${out.name}${c.on ? `: ${compactGain(c.gain)}${c.inv ? ", inverted" : ""}` : ""}`}
      class={cls("mx-cell", c.on && "on", !out.enabled && "off", props.lit && "lit", props.chosen && "chosen", props.colEnd && "mx-colend")}
      onClick={props.onClick}
      onDblClick={props.onDouble}
      onMouseEnter={props.onHover}
      onWheel={props.onWheel}
      onContextMenu={props.onMenu}
    >
      {props.editing ? (
        <span class="mx-tile" style={{ background: tint }}>
          <TileEditor label={`${where} level`} text={props.editing.text} all={props.editing.all} commit={props.commit} end={props.endEdit} />
        </span>
      ) : c.on ? (
        <span class="mx-tile" style={{ background: tint }}>
          <span class="mx-bar" style={{ background: colour }} />
          <span class="mx-g">{tileGain(c.gain)}</span>
          <span
            class={cls("mx-inv", c.inv && "on")}
            title={c.inv ? "Phase inverted (I)" : "Invert phase (I)"}
            onClick={(e) => {
              e.stopPropagation();
              if (!props.ro) props.onInvert();
            }}
            onDblClick={(e) => e.stopPropagation()}
          >
            ø
          </span>
        </span>
      ) : (
        <span class="mx-empty" aria-hidden="true" />
      )}
    </td>
  );
});

/** The gain typed in its tile: all of it chosen (or what was typed so far), Enter or leaving keeps it, Escape the old one. */
function TileEditor(props: { label: string; text: string; all: boolean; commit: (v: number) => void; end: (refocus: boolean) => void }) {
  const ref = useRef<HTMLInputElement>(null);
  const [text, setText] = useState(props.text);
  const [bad, setBad] = useState(false);
  const done = useRef(false);
  // -- WP INTE -- (REVE C1) only typed text is written: opened by a typed character it is typed; by Gain…, Enter
  // or a double click (the gain shown, all chosen) not until something is typed
  const typed = useRef(!props.all);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    if (props.all) el.select();
    else el.setSelectionRange(el.value.length, el.value.length);
  }, []);
  const finish = (keep: boolean, refocus: boolean) => {
    if (done.current) return;
    if (keep && typed.current) {
      const v = readNumber(text, ["db"]);
      if (v === null && refocus) {
        setBad(true);
        return;
      }
      done.current = true;
      if (v !== null) props.commit(v);
    } else done.current = true;
    props.end(refocus);
  };
  return (
    <input
      ref={ref}
      class="mx-edit"
      type="text"
      inputMode="decimal"
      autoComplete="off"
      spellcheck={false}
      aria-label={props.label}
      aria-invalid={bad || undefined}
      value={text}
      onInput={(e) => {
        setBad(false);
        typed.current = true;
        setText((e.target as HTMLInputElement).value);
      }}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") {
          e.preventDefault();
          finish(true, true);
        } else if (e.key === "Escape") {
          e.preventDefault();
          finish(false, true);
        }
      }}
      onBlur={() => finish(true, false)}
      onClick={(e) => e.stopPropagation()}
      onDblClick={(e) => e.stopPropagation()}
    />
  );
}

/**
 * An output's gain or delay in the matrix's footer (`CompactGainField`, `CompactDelayField`):
 * typed values kept as typed (within the range), the wheel to the next grid value, the newest
 * write winning.
 */
function CompactField(props: {
  label: string;
  value: number;
  text: (v: number) => string;
  unit: string;
  range: [number, number];
  step: number;
  disabled: boolean;
  write: (v: number) => Promise<boolean>;
}) {
  const live = useLiveWrite(props.write);
  const shown = live.draft ?? props.value;
  return (
    <span
      class={cls("m-field", shown === 0 && "zero")}
      onWheel={(e) => {
        if (props.disabled || e.deltaY === 0) return;
        e.preventDefault();
        live.release(stepGrid(shown, e.deltaY < 0 ? 1 : -1, props.step, props.range));
      }}
    >
      <ValueField
        label={props.label}
        text={props.text(shown)}
        parse={(t) => parseLevel(t, props.unit)}
        disabled={props.disabled}
        onCommit={(v) => live.release(clamp(v, props.range))}
      />
    </span>
  );
}

// ---------------------------------------------------------------- the inspector

function Pill(props: { colour: string; text: string }) {
  return (
    <span class="mx-pill" style={{ background: props.colour }}>
      {props.text}
    </span>
  );
}

function Switch(props: { label: string; on: boolean; disabled: boolean; toggle: () => void }) {
  return (
    <div class="insp-row">
      <span class="insp-label" aria-hidden="true">
        {props.label}
      </span>
      <button type="button" role="switch" class="switch" aria-checked={props.on} aria-label={props.label} disabled={props.disabled} onClick={props.toggle} />
    </div>
  );
}

function Row(props: { label: string; children: ComponentChildren }) {
  return (
    <div class="insp-row">
      <span class="insp-label">{props.label}</span>
      {props.children}
    </div>
  );
}

/** The inspector's three states and the summary with nothing chosen (MX1's `RoutingInspector`). */
function MatrixInspector(props: {
  session: Session;
  s: DspState;
  ro: boolean;
  inputs: ChannelRow[];
  outs: Out[];
  sel: Sel;
  cur: CellAt;
  link: boolean;
  gainRange: [number, number];
  maxDelay: number;
  route: (i: number, o: number, p: { enabled?: boolean; gain?: number; invert?: boolean }) => Promise<boolean>;
  enable: (o: number) => void;
  mute: (o: number) => void;
  openEq: (wire: number) => void;
}) {
  const { session, s, ro, inputs, outs, sel } = props;
  const solo = useSolo(session); // -- WP GAP2 --
  const nIn = inputs.length;
  const nOut = outs.length;
  const outLine = (o: Out) =>
    [o.enabled ? "on" : "off", compactGain(s.outputGainDb[o.index] ?? 0), delayText(s.outputDelayMs[o.index] ?? 0), o.muted ? "muted" : ""].filter(Boolean).join(" · ");
  const limiter = (o: number) => (limiterSupported(s) ? ((s.limiterEnabled[o] ?? false) ? `Limiter ${levelText(s.limiterThresholdDb[o] ?? -1, "dBFS")}` : "Limiter off") : "");
  const crossover = (wire: number) => {
    const both = passes(s.xover?.[wire] ?? []);
    return [both.hp ? `HP ${passName(both.hp)} ${freqText(both.hp.freq)}` : "", both.lp ? `LP ${passName(both.lp)} ${freqText(both.lp.freq)}` : ""].filter(Boolean).join(", ");
  };
  const openEq = (wire: number) => (
    <button type="button" onClick={() => props.openEq(wire)}>
      Open in Equaliser
    </button>
  );

  if (sel.kind === "cell" && inputs[props.cur.i] && outs[props.cur.o]) {
    const { i, o } = props.cur;
    const inp = inputs[i]!;
    const out = outs[o]!;
    const on = s.matrixRouting[i]?.[o] ?? false;
    const gain = s.matrixGain[i]?.[o] ?? 0;
    const inv = s.matrixInvert[i]?.[o] ?? false;
    const m = props.link ? mirrorOf(i, o, nIn, nOut) : null;
    const more = [crossover(out.wire), limiter(o)].filter(Boolean).join(" · ");
    return (
      <div class="mx-insp" aria-label={`${inp.name} to ${out.name}`}>
        <div class="insp-head mx-insp-head">
          <Pill colour={inputColour(inp.index)} text={inp.descriptor} /> <span class="insp-name">{inp.name}</span> <span aria-hidden="true">→</span> <Pill colour={channelColour(s, out.wire)} text={out.descriptor} />{" "}
          <span class="insp-name">{out.name}</span>
        </div>
        <InspectorSection title="Crosspoint" summary={on ? `${compactGain(gain)}${inv ? " · inverted" : ""}` : "off"}>
          <Switch label="Connected" on={on} disabled={ro} toggle={() => void props.route(i, o, { enabled: !on })} />
          {on ? (
            <>
              <Row label="Gain">
                <input
                  type="range"
                  aria-label="Crosspoint gain slider"
                  min={CROSSPOINT_RANGE[0]}
                  max={CROSSPOINT_RANGE[1]}
                  step={0.5}
                  value={gain}
                  style={rangeFill(gain, CROSSPOINT_RANGE[0], CROSSPOINT_RANGE[1])}
                  disabled={ro}
                  onChange={(e) => void props.route(i, o, { gain: Number((e.target as HTMLInputElement).value) })}
                />
                <ValueField label="Crosspoint gain" text={compactGain(gain)} parse={(t) => readNumber(t, ["db"])} disabled={ro} onCommit={(v) => void props.route(i, o, { gain: clamp(v, CROSSPOINT_RANGE) })} />
              </Row>
              <Switch label="Invert phase" on={inv} disabled={ro} toggle={() => void props.route(i, o, { invert: !inv })} />
            </>
          ) : (
            <p class="note">Not connected. Space connects it at 0 dB; Enter or a digit types its gain.</p>
          )}
          {m ? (
            <p class="note">
              Link L/R pairs: changes here are made on {inputs[m.i]!.name} → {outs[m.o]!.name} too.
            </p>
          ) : null}
        </InspectorSection>
        <InspectorSection title={`${inp.name} also feeds`}>
          <ChipGroup
            label={`${inp.name} also feeds`}
            chips={outs.filter((x) => x.index !== o).map((x) => ({ key: x.index, text: x.name, on: s.matrixRouting[i]?.[x.index] ?? false, disabled: ro }))}
            onToggle={(k) => void props.route(i, Number(k), { enabled: !(s.matrixRouting[i]?.[Number(k)] ?? false) })}
          />
        </InspectorSection>
        <InspectorSection title={out.name} summary={outLine(out)}>
          {more ? <p class="note">{more}</p> : null}
          {openEq(out.wire)}
        </InspectorSection>
      </div>
    );
  }

  if (sel.kind === "col" && outs[sel.o]) {
    const out = outs[sel.o]!;
    const o = out.index;
    const from = sourcesOf(s.matrixRouting, o, nIn);
    const xo = crossover(out.wire);
    return (
      <div class="mx-insp" aria-label={out.name}>
        <div class="insp-head mx-insp-head">
          <Pill colour={channelColour(s, out.wire)} text={out.descriptor} /> <span class="insp-name">{out.name}</span>
        </div>
        <InspectorSection title="Output" summary={outLine(out)}>
          <Switch label="Enabled" on={out.enabled} disabled={ro} toggle={() => props.enable(o)} />
          <Row label="Gain">
            <ValueField
              label="Output gain"
              text={compactGain(s.outputGainDb[o] ?? 0)}
              parse={(t) => readNumber(t, ["db"])}
              disabled={ro}
              onCommit={(v) => void session.write("out.gain", { float: clamp(v, props.gainRange) }, [o])}
            />
          </Row>
          <Row label="Delay">
            <ValueField
              label="Output delay"
              text={delayText(s.outputDelayMs[o] ?? 0)}
              parse={(t) => readNumber(t, ["ms"])}
              disabled={ro}
              onCommit={(v) => void session.write("out.delay", { float: clamp(v, [0, props.maxDelay]) }, [o])}
            />
          </Row>
          <Switch label="Mute" on={out.muted} disabled={ro} toggle={() => props.mute(o)} />
          {/* -- WP GAP2 -- only this output plays, until it is switched off (not saved) */}
          <Switch label="Solo" on={solo.output === o} disabled={!!solo.reason || !out.enabled} toggle={() => void solo.toggle(o)} />
        </InspectorSection>
        <InspectorSection title="Fed by" summary={from.length === 1 ? "1 input" : `${from.length} inputs`}>
          <ChipGroup
            label={`${out.name} fed by`}
            chips={inputs.map((x) => ({ key: x.index, text: x.name, on: s.matrixRouting[x.index]?.[o] ?? false, disabled: ro }))}
            onToggle={(k) => void props.route(Number(k), o, { enabled: !(s.matrixRouting[Number(k)]?.[o] ?? false) })}
          />
        </InspectorSection>
        {xo || limiter(o) ? <p class="note">{[xo ? `Crossover ${xo}` : "", limiter(o)].filter(Boolean).join(" · ")}</p> : null}
        {openEq(out.wire)}
      </div>
    );
  }

  if (sel.kind === "row" && inputs[sel.i]) {
    const inp = inputs[sel.i]!;
    const feeds = feedsOf(s.matrixRouting, inp.index, nOut);
    return (
      <div class="mx-insp" aria-label={inp.name}>
        <div class="insp-head mx-insp-head">
          <Pill colour={inputColour(inp.index)} text={inp.descriptor} /> <span class="insp-name">{inp.name}</span>
        </div>
        <InspectorSection title="Input">
          <Row label="Trim (preamp)">
            <InputTrimField session={session} input={inp.index} name={`${inp.descriptor} ${inp.name} (inspector)`} db={s.inputPreampDb[inp.index] ?? 0} disabled={ro} />
          </Row>
        </InspectorSection>
        <InspectorSection title="Feeds" summary={feeds.length === 1 ? "1 output" : `${feeds.length} outputs`}>
          <ChipGroup
            label={`${inp.name} feeds`}
            chips={outs.map((x) => ({ key: x.index, text: x.name, on: s.matrixRouting[inp.index]?.[x.index] ?? false, disabled: ro }))}
            onToggle={(k) => void props.route(inp.index, Number(k), { enabled: !(s.matrixRouting[inp.index]?.[Number(k)] ?? false) })}
          />
        </InspectorSection>
        {openEq(inp.wire)}
      </div>
    );
  }

  return (
    <div class="mx-insp" aria-label="Routing">
      <div class="insp-head mx-insp-head">
        <span class="insp-name">Routing</span>
      </div>
      <p class="mx-summary">{summaryText(s.matrixRouting, s.outputEnabled, nIn, nOut)}</p>
      <p class="note">Choose a cell, an input's row or an output's column in the matrix to see it here.</p>
    </div>
  );
}
