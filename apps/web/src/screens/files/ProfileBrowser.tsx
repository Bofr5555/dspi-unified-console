// WP WB6K: Browse Profiles and Favorite Profiles (M31) in the EQ's AutoEQ panel. The search,
// the favourites file and the entries are the core's, over the database the page keeps
// (./model.ts); choosing a profile hands its entry to the panel's Preview and Apply.

import { useEffect, useState } from "preact/hooks";
import type { AutoEqHit } from "../../core/protocol";
import type { Session } from "../../ui/session";
import { useLibrary } from "./FilesCards";
import { autoEqLibrary, failureWords, type AutoEqLibrary } from "./model";

export function ProfileBrowser(props: {
  session: Session;
  /** The id of the profile the panel holds, "" for none or one from a file. */
  chosen: string;
  onChoose: (name: string, id: string, entry: unknown) => void;
  onError: (text: string) => void;
  library?: AutoEqLibrary;
}) {
  const { session } = props;
  const lib = props.library ?? autoEqLibrary();
  const st = useLibrary(lib);
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<{ hits: AutoEqHit[]; total: number } | null>(null);
  const [favs, setFavs] = useState<AutoEqHit[]>([]);

  useEffect(() => {
    let alive = true;
    void lib.favouriteHits(session).then((h) => alive && setFavs(h));
    return () => {
      alive = false;
    };
  }, [st.favourites, st.info]);

  useEffect(() => {
    let alive = true;
    const q = query;
    const t = setTimeout(() => {
      lib.search(session, q).then(
        (r) => alive && setFound(q.trim() ? r : null),
        (e: unknown) => alive && props.onError(failureWords(e)),
      );
    }, 150);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [query, st.info]);

  const choose = async (h: AutoEqHit) => {
    try {
      props.onChoose(h.name, h.id, await lib.entry(session, h.id));
    } catch (e) {
      props.onError(failureWords(e));
    }
  };

  const row = (h: AutoEqHit) => {
    const fav = st.favourites.includes(h.id);
    return (
      <li key={h.id} class="autoeq-hit">
        <button type="button" aria-pressed={props.chosen === h.id} onClick={() => void choose(h)} title={`${h.filters} filters, preamp ${h.preampDb.toFixed(1)} dB`}>
          {h.name}
        </button>{" "}
        <span class="note">
          {h.source}
          {h.formFactor ? ` · ${h.formFactor}` : ""}
        </span>{" "}
        <button type="button" class="heart" aria-pressed={fav} aria-label={`Favorite ${h.name}`} onClick={() => void lib.toggleFavourite(session, h.id)}>
          <span aria-hidden="true">{fav ? "♥" : "♡"}</span>
        </button>
      </li>
    );
  };

  return (
    <div class="autoeq-browse">
      <div class="autoeq-row">
        <label>
          Search profiles{" "}
          <input type="search" name="autoeq-search" value={query} placeholder="Manufacturer or model" onInput={(e) => setQuery((e.target as HTMLInputElement).value)} />
        </label>
      </div>
      {st.problem && !st.info ? (
        <p class="error" role="alert" aria-label="AutoEQ database">
          {st.problem}
        </p>
      ) : null}
      {found ? (
        found.hits.length ? (
          <>
            <ul class="autoeq-hits" aria-label="Profiles found">
              {found.hits.map(row)}
            </ul>
            {found.total > found.hits.length ? <p class="note">{found.total - found.hits.length} more: search more narrowly.</p> : null}
          </>
        ) : (
          <p class="note">No profile matches “{query.trim()}”.</p>
        )
      ) : favs.length ? (
        <ul class="autoeq-hits" aria-label="Favorite Profiles">
          {favs.map(row)}
        </ul>
      ) : null}
    </div>
  );
}
