// The main window's channels (WP WB6a): the inputs and the enabled outputs as
// the Console's sidebar lists them, each with its name (rename), its level trim
// (an input's `pre`, an output's `out.gain`) and, for an output, its mute. Rows
// keep one height whatever their state, so nothing moves as values change.

import { memo } from "preact/compat";
import { useState } from "preact/hooks";
import type { ParamInfo } from "../../core/protocol";
import type { Session } from "../../ui/session";
import { RenameDialog } from "./Dialog";
import { dragProps, useLiveWrite } from "./live";
import { type ChannelRow, sameItems } from "./model";
// -- WP WB6G -- an input's trim through its pair's link
import { writeTrim } from "../inputs/model";
import { rangeFill } from "../../ui/design"; // -- WP WEB2 --

export const ChannelList = memo(
  function ChannelList(props: {
    session: Session;
    title: string;
    rows: ChannelRow[];
    /** Per row, its trim in dB. */
    gains: number[];
    /** Per row, muted; null for inputs, which have no mute. */
    mutes: boolean[] | null;
    /** `pre` or `out.gain`, as the registry gives it; null until known. */
    gain: ParamInfo | null;
    disabled: boolean;
  }) {
    const [renaming, setRenaming] = useState<ChannelRow | null>(null);
    return (
      <section class="card channels-card" aria-label={props.title}>
        <h3>{props.title}</h3>
        <ul class="channel-rows">
          {props.rows.map((r, i) => (
            <Row
              key={r.wire}
              session={props.session}
              row={r}
              gain={props.gains[i] ?? 0}
              muted={props.mutes ? (props.mutes[i] ?? false) : null}
              info={props.gain}
              disabled={props.disabled}
              onRename={setRenaming}
            />
          ))}
        </ul>
        {renaming ? (
          <RenameDialog
            title={`Rename ${renaming.name}`}
            label="Name"
            initial={renaming.stored ?? renaming.name}
            onCancel={() => setRenaming(null)}
            onSave={(name) => {
              const wire = renaming.wire;
              setRenaming(null);
              void props.session.write("ch.name", { text: name }, [wire]);
            }}
          />
        ) : null}
      </section>
    );
  },
  (a, b) =>
    a.session === b.session &&
    a.title === b.title &&
    a.gain === b.gain &&
    a.disabled === b.disabled &&
    sameItems(a.gains, b.gains) &&
    (a.mutes === b.mutes || (!!a.mutes && !!b.mutes && sameItems(a.mutes, b.mutes))) &&
    a.rows.length === b.rows.length &&
    a.rows.every((r, i) => r.wire === b.rows[i]!.wire && r.name === b.rows[i]!.name && r.index === b.rows[i]!.index),
);

const Row = memo(
  function Row(props: {
  session: Session;
  row: ChannelRow;
  gain: number;
  muted: boolean | null;
  info: ParamInfo | null;
  disabled: boolean;
  onRename: (r: ChannelRow) => void;
}) {
  const { session, row, info, disabled } = props;
  const path = info?.path ?? "";
  // -- WP WB6G -- an input's trim (`pre`) goes through its pair's link: both sides, one undo step
  const live = useLiveWrite((db) => (path === "pre" ? writeTrim(session, row.index, db) : session.write(path, { float: db }, [row.index])), session);
  const db = live.draft ?? props.gain;
  const value = (e: Event) => Number((e.target as HTMLInputElement).value);
  const reset = (e: Event) => {
    e.preventDefault();
    if (!disabled && info) live.release(0);
  };
  return (
    <li class={props.muted ? "channel-row muted" : "channel-row"} data-wire={row.wire}>
      <span class="ch-tag" aria-hidden="true">
        {row.descriptor}
      </span>
      <button type="button" class="ch-name" title={row.detail ? `${row.detail}: rename` : "Rename"} disabled={disabled} onClick={() => props.onRename(row)}>
        <span class="visually-hidden">Rename </span>
        {row.name}
      </button>
      <output class="ch-value">{db.toFixed(1)} dB</output>
      {props.muted !== null ? (
        <button
          type="button"
          class="toggle ch-mute"
          aria-pressed={props.muted}
          aria-label={`Mute ${row.name}`}
          disabled={disabled}
          onClick={() => void session.write("out.mute", { bool: !props.muted }, [row.index])}
        >
          M
        </button>
      ) : (
        <span class="ch-mute-gap" aria-hidden="true" />
      )}
      <input
        type="range"
        class="ch-gain"
        aria-label={`${row.name} ${info?.label ?? "gain"}`}
        aria-valuetext={`${db.toFixed(1)} dB`}
        min={info?.min ?? -60}
        max={info?.max ?? 0}
        step={0.1}
        value={db}
        style={rangeFill(db, info?.min ?? -60, info?.max ?? 0)}
        disabled={disabled || !info}
        {...dragProps(live)}
        onInput={(e) => live.move(value(e))}
        onChange={(e) => live.release(value(e))}
        onDblClick={reset}
        onContextMenu={reset}
      />
    </li>
  );
  },
  (a, b) =>
    a.session === b.session &&
    a.gain === b.gain &&
    a.muted === b.muted &&
    a.info === b.info &&
    a.disabled === b.disabled &&
    a.onRename === b.onRename &&
    a.row.wire === b.row.wire &&
    a.row.index === b.row.index &&
    a.row.name === b.row.name,
);
