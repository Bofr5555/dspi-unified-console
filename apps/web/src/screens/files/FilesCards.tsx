// WP WB6K: the Files screen's filter files (Export Filters, Import Filters… with the channel
// picker, M28), Save Master Volume and Save Output Configuration (M30), and the AutoEQ database
// (Update Database: Download…, Import File…, Reset to Built-in; Favorite Profiles and Clear
// Favorites, M31, M32). The formats and checks are the core's (./model.ts).

import { useEffect, useRef, useState } from "preact/hooks";
import type { FilterFileInspect } from "../../core/protocol";
import { useDesk } from "../snapshots/Bars";
import { download, type SnapshotDesk } from "../snapshots/model";
import type { Session } from "../../ui/session";
import {
  FILTERS_FILE,
  autoEqLibrary,
  chosenWires,
  databaseLine,
  downloadAddress,
  exportFilters,
  failureWords,
  importFilters,
  inspectFilters,
  inspectProblem,
  type AutoEqLibrary,
  type LibraryState,
} from "./model";

/** A filter file chosen, waiting for its channels. */
export interface PendingFilters {
  text: string;
  name: string;
  inspect: FilterFileInspect;
}

/** The Filters card: Export Filters, Import Filters… */
export function FiltersCard(props: { desk: SnapshotDesk; onChosen: (p: PendingFilters) => void }) {
  const { desk } = props;
  const st = useDesk(desk);
  const busy = !!st.busy;
  const [problem, setProblem] = useState("");
  const file = useRef<HTMLInputElement>(null);
  const open = !!desk.session.state.snapshot && !desk.session.document;

  const choose = async (input: HTMLInputElement) => {
    const f = input.files?.[0];
    input.value = "";
    if (!f) return;
    try {
      const text = await f.text();
      const inspect = await inspectFilters(desk.session, text);
      const why = inspectProblem(inspect);
      if (why) {
        setProblem(`${f.name}: ${why}`);
        return;
      }
      setProblem("");
      props.onChosen({ text, name: f.name, inspect });
    } catch (e) {
      setProblem(`${f.name}: ${failureWords(e)}`);
    }
  };

  // -- WP WEB7 -- Qt's words and order: Import Filters…, then Export Filters…
  return (
    <div class="card" role="region" aria-label="Filters">
      <h3>Filters</h3>
      <p class="note">The bands of the channels you choose, as a text file that other programs read too.</p>
      {problem ? (
        <p class="error" role="alert">
          {problem}
        </p>
      ) : null}
      <div class="files-row">
        <button type="button" disabled={busy || !open || !!desk.readOnly} onClick={() => file.current?.click()}>
          Import Filters…
        </button>
        <input
          ref={file}
          type="file"
          hidden
          accept=".txt,text/plain"
          aria-label="Filter file to import"
          onChange={(e) => void choose(e.target as HTMLInputElement)}
        />
        <button
          type="button"
          disabled={busy || !open}
          onClick={() =>
            void exportFilters(desk).then((t) => {
              if (t) download(t, FILTERS_FILE);
            })
          }
        >
          Export Filters…
        </button>
      </div>
    </div>
  );
}

/** The channel picker of a filter import: which channels take the file. */
export function FilterPicker(props: { desk: SnapshotDesk; pending: PendingFilters; onDone: () => void }) {
  const { desk, pending } = props;
  const st = useDesk(desk);
  const channels = pending.inspect.channels;
  const [ticked, setTicked] = useState(() => new Set(channels.filter((c) => c.checked).map((c) => c.wire)));
  const wires = chosenWires(channels, ticked);
  const i = pending.inspect;
  const what =
    i.kind === "rew"
      ? `A filter list of ${i.filters ?? 0} filters${typeof i.preamp === "number" ? `, preamp ${i.preamp.toFixed(1)} dB (inputs only)` : ""}.`
      : "A DSPi filter file.";

  return (
    <div class="card import-preview" role="region" aria-label="Import filters">
      <h3>Import filters from “{pending.name}”</h3>
      <p class="note">{what}</p>
      {i.newer ? <p class="note">A newer version wrote this file: what this version does not know is left out.</p> : null}
      <fieldset class="import-blocks">
        <legend>Channels</legend>
        {channels.map((c) => (
          <label key={c.wire} class="check">
            <input
              type="checkbox"
              name={`filters-${c.wire}`}
              checked={ticked.has(c.wire)}
              onChange={(e) =>
                setTicked((t) => {
                  const n = new Set(t);
                  if ((e.target as HTMLInputElement).checked) n.add(c.wire);
                  else n.delete(c.wire);
                  return n;
                })
              }
            />
            {c.label} <span class="note">({c.bands} bands)</span>
          </label>
        ))}
      </fieldset>
      {!wires.length ? <p class="note">Nothing is chosen.</p> : null}
      <div class="snap-actions">
        <button
          type="button"
          disabled={!!desk.readOnly || !!st.busy || !wires.length}
          onClick={() => void importFilters(desk, pending.text, pending.name, wires).then((ok) => ok && props.onDone())}
        >
          Import
        </button>
        <button type="button" onClick={props.onDone}>
          Cancel
        </button>
      </div>
    </div>
  );
}

/** The library's state, kept fresh. */
export function useLibrary(lib: AutoEqLibrary): LibraryState {
  const [, bump] = useState(0);
  useEffect(() => lib.watch(() => bump((n) => n + 1)), [lib]);
  return lib.state;
}

/**
 * -- WP WEB7 -- The AutoEQ database (Library › AutoEQ's ▾ › Update Database…, as Qt's dialog): what is
 * loaded, Download…, Import File…, Reset to Built-in, the favourites and Clear Favorites.
 */
export function AutoEqDatabasePanel(props: { session: Session; library?: AutoEqLibrary }) {
  const session = props.session;
  const lib = props.library ?? autoEqLibrary();
  const st = useLibrary(lib);
  const [url, setUrl] = useState("");
  const [result, setResult] = useState<{ text: string; tone: "info" | "error" } | null>(null);
  const [names, setNames] = useState<{ id: string; name: string }[]>([]);
  const file = useRef<HTMLInputElement>(null);
  const busy = !!st.busy;

  useEffect(() => void lib.ready(session), [session]);
  useEffect(() => {
    let alive = true;
    void lib.favouriteHits(session).then((h) => alive && setNames(h.map((x) => ({ id: x.id, name: x.name }))));
    return () => {
      alive = false;
    };
  }, [st.favourites, st.info]);

  const done = (what: string, p: Promise<{ entryCount: number }>) =>
    p.then(
      (info) => setResult({ text: `${what}: ${info.entryCount} profiles.`, tone: "info" }),
      (e: unknown) => setResult({ text: `${what}: not done. ${failureWords(e)}`, tone: "error" }),
    );

  const importFile = async (input: HTMLInputElement) => {
    const f = input.files?.[0];
    input.value = "";
    if (!f) return;
    await done("Import File", f.text().then((t) => lib.importFile(session, t)));
  };

  const fetchIt = async () => {
    const address = downloadAddress(url);
    if (!address) {
      setResult({ text: "Download: give an http or https address.", tone: "error" });
      return;
    }
    await done("Download", lib.download(session, address));
  };

  return (
    <div class="autoeq-db" role="region" aria-label="AutoEQ database">
      <p role="status" aria-label="Current database">
        {st.problem && !st.info ? st.problem : databaseLine(st.info)}
      </p>
      {st.info?.problem ? <p class="note">Your own database could not be read ({st.info.problem}); the built-in one is used.</p> : null}
      <div class="files-row">
        <label>
          Address of a database file
          <input name="autoeq-url" type="url" value={url} placeholder="https://…/autoeq_database.json" onInput={(e) => setUrl((e.target as HTMLInputElement).value)} />
        </label>
        <button type="button" disabled={busy || !url.trim()} onClick={() => void fetchIt()}>
          Download
        </button>
      </div>
      <div class="files-row">
        <button type="button" disabled={busy} onClick={() => file.current?.click()}>
          Import File…
        </button>
        <input ref={file} type="file" hidden accept=".json,application/json" aria-label="AutoEQ database file" onChange={(e) => void importFile(e.target as HTMLInputElement)} />
        {st.info?.hasUserDatabase ? (
          <button type="button" disabled={busy} onClick={() => void done("Reset to Built-in", lib.resetToBuiltIn(session))}>
            Reset to Built-in
          </button>
        ) : null}
      </div>
      <p class="note">Rebuild from GitHub is not offered here: download a ready-made database file or import one instead.</p>
      {result ? (
        <p class={`autoeq-outcome ${result.tone}`} role={result.tone === "error" ? "alert" : "status"} aria-label="Database result">
          {result.text}
        </p>
      ) : null}
      <h4 class="section-label">Favorite Profiles</h4>
      {names.length ? (
        <ul class="change-list" aria-label="Favorite Profiles">
          {names.map((n) => (
            <li key={n.id}>
              {n.name}{" "}
              <button type="button" class="dismiss" aria-label={`Remove ${n.name} from the favorites`} onClick={() => void lib.toggleFavourite(session, n.id)}>
                ×
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <p class="note">No favorites yet. Mark a headphone with its heart in Library › AutoEQ.</p>
      )}
      <button type="button" disabled={!st.favourites.length} onClick={() => void lib.clearFavourites(session)}>
        Clear Favorites
      </button>
    </div>
  );
}
