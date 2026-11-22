// -- WP WEBM4 -- The command palette (MODLAY 3.1, the desktop's LAY6): everything the console can
// do or show, found by typing. Opened by Ctrl+K (Cmd+K on a Mac) and the top bar's search. Its
// entries are made as it opens (ui/palette/entries.ts); `search` finds them by the desktop's rules.
//
// A modal dialog, so nothing behind it takes a key (the plain B of A/B types here). ARIA's
// combobox with a list box: the field keeps the focus, Up and Down move through the entries that
// can be chosen (round the end), Enter runs one, Escape closes. A greyed entry is listed with why,
// never chosen. What was chosen runs once the palette is closed, with the focus back where it was.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "preact/hooks";
import { keyText } from "../../screens/shortcuts/table";
import { appleKeys } from "../undo";
import type { PaletteEntry } from "../menus/board";
import { search } from "./search";

/** The next entry from `from` that can be chosen, round the end; -1 for none. */
export function nextChoosable(rows: readonly { enabled: boolean }[], from: number, step: 1 | -1): number {
  const n = rows.length;
  for (let i = 1; i <= n; ++i) {
    const at = (((from + i * step) % n) + n) % n;
    if (rows[at]!.enabled) return at;
  }
  return -1;
}

let paletteIds = 0;

export function Palette(props: { entries: PaletteEntry[]; onDone: (chosen: PaletteEntry | null) => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const field = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const [query, setQuery] = useState("");
  const rows = useMemo(() => search(props.entries, query), [props.entries, query]);
  const [at, setAt] = useState(() => nextChoosable(rows, -1, 1));
  const base = useMemo(() => `palette-${++paletteIds}`, []);
  const mac = appleKeys(typeof navigator === "undefined" ? "" : (navigator.platform ?? ""));

  useLayoutEffect(() => {
    const d = dialog.current;
    if (d && !d.open) d.showModal();
    field.current?.focus();
    return () => {
      if (d?.open) d.close();
    };
  }, []);
  // A new query: its best entry is picked
  useEffect(() => setAt(nextChoosable(rows, -1, 1)), [rows]);
  useEffect(() => {
    list.current?.querySelector<HTMLElement>(`#${base}-${at}`)?.scrollIntoView({ block: "nearest" });
  }, [at, base]);

  const choose = (i: number) => {
    const e = rows[i];
    if (e?.enabled) props.onDone(e);
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown") setAt(nextChoosable(rows, at < 0 ? -1 : at, 1));
    else if (e.key === "ArrowUp") setAt(nextChoosable(rows, at < 0 ? 0 : at, -1));
    else if (e.key === "Enter") choose(at);
    else if (e.key === "Home" && e.ctrlKey) setAt(nextChoosable(rows, -1, 1));
    else if (e.key === "End" && e.ctrlKey) setAt(nextChoosable(rows, 0, -1));
    else return;
    e.preventDefault();
  };

  return (
    <dialog
      ref={dialog}
      class="palette"
      aria-label="Command palette"
      onCancel={(e) => {
        e.preventDefault();
        props.onDone(null);
      }}
      // A press beside the list (on the backdrop) closes it
      onClick={(e) => e.target === dialog.current && props.onDone(null)}
    >
      <input
        ref={field}
        class="palette-field"
        type="text"
        role="combobox"
        aria-label="Search settings, channels, commands"
        aria-expanded="true"
        aria-controls={`${base}-list`}
        aria-autocomplete="list"
        aria-activedescendant={at >= 0 ? `${base}-${at}` : undefined}
        placeholder="Search settings, channels, commands…"
        autocomplete="off"
        spellcheck={false}
        value={query}
        onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
        onKeyDown={onKey}
      />
      <ul ref={list} id={`${base}-list`} class="palette-list" role="listbox" aria-label="Results">
        {rows.map((r, i) => (
          <li
            key={r.id}
            id={`${base}-${i}`}
            role="option"
            aria-selected={i === at}
            aria-disabled={r.enabled ? undefined : "true"}
            class={`palette-row${i === at ? " on" : ""}${r.enabled ? "" : " off"}`}
            data-entry={r.id}
            title={r.reason || undefined}
            onMouseMove={() => r.enabled && i !== at && setAt(i)}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => choose(i)}
          >
            <span class="palette-title">
              {r.title}
              {r.matched ? <span class="palette-matched"> · {r.matched}</span> : null}
            </span>
            <span class="palette-where">{r.reason && !r.enabled ? r.reason : r.where}</span>
            {r.keys ? (
              <kbd class="palette-keys" aria-hidden="true">
                {keyText(r.keys, mac)}
              </kbd>
            ) : null}
          </li>
        ))}
      </ul>
      {rows.length ? null : (
        <p class="palette-none" role="status">
          Nothing found
        </p>
      )}
    </dialog>
  );
}
