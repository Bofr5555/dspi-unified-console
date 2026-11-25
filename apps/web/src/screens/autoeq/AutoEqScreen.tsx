// -- WP WEB7 -- Library › AutoEQ, as Qt Modern's (AutoEqPanel.qml's Modern body, DL3a/INTC): the
// headphones as rows at the left (the ear or the headphones, the name, "source · form factor", the
// heart on hover and on a favourite), the chosen one at the right under "Headphone profile": where it
// goes (Apply to: the pair) and Apply, and a frame with what it would do (the core's preview, nothing
// written). The search, its count and the ▾ (Clear Favorites, Update Database…) are the page's tools.
// The database, the favourites and the search are the page's AutoEQ library (screens/files/model.ts),
// the preview and the apply the core's (`autoeqPreview`, `autoeqApply`: one undo step, through the
// limits, the state before kept to download), as the Equaliser's AutoEQ panel uses them.
//
// The web's core lists no profiles for an empty search (it finds, it does not browse 3 800 rows), so
// before a search the list shows the favourites, or an empty state that says to search.

import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import type { AutoEqHit, AutoEqPreviewRecord } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import { EmptyState, Glyph, NoticeRow } from "../../ui/design";
import { ActionMenuButton } from "../../ui/menus/ActionMenu";
import { menuItems } from "../../ui/menus/actions";
import type { ScreenProps } from "../../ui/screens";
import { PageTools } from "../../ui/shell/Parts";
import { noticeOf } from "../../ui/texts";
import { levelLine } from "../eq/AutoEq";
import { AutoEqDatabasePanel, useLibrary } from "../files/FilesCards";
import { autoEqLibrary, failureWords } from "../files/model";
import { Dialog } from "../main/Dialog";
import { inputRows, liveInputs, outputRows } from "../main/model";
import { download } from "../snapshots/model";
import { PLOT, curvePath, dbSpan, hitIcon, hitLine, pairsOf, previewCurves, xOf, yOf } from "./model";
import "../snapshots/rows.css";
import "./autoeq.css";

const TICKS_HZ = [100, 1000, 10000];

export function AutoEqScreen(props: ScreenProps) {
  const { session } = props;
  const st = useSessionState(session);
  const s = st.snapshot?.state;
  const lib = autoEqLibrary();
  const libState = useLibrary(lib);
  const ro = session.readOnlyReason;

  const [query, setQuery] = useState("");
  const [found, setFound] = useState<{ hits: AutoEqHit[]; total: number } | null>(null);
  const [favs, setFavs] = useState<AutoEqHit[]>([]);
  const [chosen, setChosen] = useState<{ hit: AutoEqHit; entry: unknown } | null>(null);
  const [pairAt, setPairAt] = useState(0);
  const [applyLevel, setApplyLevel] = useState(false);
  const [preview, setPreview] = useState<AutoEqPreviewRecord | null>(null);
  const [outcome, setOutcome] = useState<{ lines: string[]; before: string; tone: "info" | "warning" | "error" } | null>(null);
  const [busy, setBusy] = useState(false);
  const [dbOpen, setDbOpen] = useState(false);
  const [freqs, setFreqs] = useState<number[] | null>(null);
  const [usbActive, setUsbActive] = useState<number | null>(null);
  const search = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let alive = true;
    void lib.ready(session);
    session.core.call("curveGrid", undefined).then(
      (g) => alive && setFreqs(g.freqs),
      () => undefined,
    );
    session.core.call("meters", undefined).then(
      (m) => alive && setUsbActive(m.activeInputChannels),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [session]);

  useEffect(() => {
    let alive = true;
    void lib.favouriteHits(session).then((h) => alive && setFavs(h));
    return () => {
      alive = false;
    };
  }, [libState.favourites, libState.info]);

  useEffect(() => {
    let alive = true;
    const q = query;
    const t = setTimeout(() => {
      lib.search(session, q).then(
        (r) => alive && setFound(q.trim() ? r : null),
        (e: unknown) => alive && setOutcome({ lines: [failureWords(e)], before: "", tone: "error" }),
      );
    }, 150);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [query, libState.info]);

  const inputs = s?.numInputChannels ?? 0;
  const pairs = useMemo(() => {
    if (!s) return [];
    const ins = inputRows(s, liveInputs(s, usbActive)).map((r) => ({ wire: r.wire, descriptor: r.descriptor, name: r.name, output: false }));
    const outs = outputRows(s, st.snapshot?.outputLabels).map((r) => ({ wire: r.wire, descriptor: r.descriptor, name: r.name, output: true }));
    return pairsOf([...ins, ...outs], inputs);
  }, [JSON.stringify([s?.channelNames, s?.outputEnabled, s?.numInputChannels, s?.inputSource, s?.i2sInputChannels, usbActive])]);
  const pair = pairs[pairAt] ?? pairs[0];
  const outputs = !!pair?.left.output;

  const params = (entry: unknown) => ({ profile: entry, left: pair!.left.wire, right: pair!.right.wire, ...(outputs ? { applyLevel } : {}) });
  const failed = (e: unknown) => {
    const n = noticeOf(e);
    setOutcome({ lines: [n ? `${n.title}: ${n.reason} ${n.detail}`.trim() : failureWords(e)], before: "", tone: "error" });
  };

  // What the chosen headphone would do on the chosen pair: asked again when either changes.
  useEffect(() => {
    let alive = true;
    setPreview(null);
    if (!chosen || !pair || !s) return;
    session.core.call("autoeqPreview", params(chosen.entry)).then(
      (p) => alive && setPreview(p),
      (e: unknown) => alive && failed(e),
    );
    return () => {
      alive = false;
    };
  }, [chosen, pair?.left.wire, pair?.right.wire, applyLevel]);

  const choose = async (h: AutoEqHit) => {
    try {
      setOutcome(null);
      setChosen({ hit: h, entry: await lib.entry(session, h.id) });
    } catch (e) {
      failed(e);
    }
  };

  const apply = async (c = chosen) => {
    if (!c || !pair || busy || ro) return;
    setBusy(true);
    try {
      const r = await session.core.call("autoeqApply", params(c.entry));
      await session.refresh();
      const p = r.preview;
      setOutcome({
        lines: [`Applied ${c.hit.name} to ${pair.title}.`, levelLine(p), ...p.notes, ...r.notes],
        before: r.snapshotDocument,
        tone: p.held.length || p.notes.length ? "warning" : "info",
      });
    } catch (e) {
      failed(e);
    } finally {
      setBusy(false);
    }
  };

  const list = found ? found.hits : favs;
  const count = found ? found.total : (libState.info?.entryCount ?? 0);
  const curves = previewCurves(preview);
  const span = dbSpan(curves.map((c) => c.curve));

  const row = (h: AutoEqHit) => {
    const fav = libState.favourites.includes(h.id);
    const selected = chosen?.hit.id === h.id;
    return (
      <li key={h.id} class={`lib-row aeq-row${fav ? " fav" : ""}${selected ? " selected" : ""}`} aria-label={`${h.name}, ${hitLine(h)}`}>
        <span class="lib-lead">
          <Glyph name={hitIcon(h)} />
        </span>
        <button
          type="button"
          class="lib-text aeq-choose"
          aria-pressed={selected}
          title={`${h.filters} filters, preamp ${h.preampDb.toFixed(1)} dB`}
          onClick={() => void choose(h)}
          onDblClick={() => void lib.entry(session, h.id).then((entry) => apply({ hit: h, entry }), failed)}
        >
          <span class="lib-title">{h.name}</span>
          <span class="lib-sub">{hitLine(h)}</span>
        </button>
        <span class="lib-actions aeq-heart">
          <button type="button" aria-pressed={fav} aria-label={`Favorite ${h.name}`} title={fav ? "Remove from favorites" : "Add to favorites"} onClick={() => void lib.toggleFavourite(session, h.id)}>
            <span aria-hidden="true">{fav ? "♥" : "♡"}</span>
          </button>
        </span>
      </li>
    );
  };

  return (
    <section class="lib-page aeq-screen" aria-label="AutoEQ">
      <PageTools label="AutoEQ tools">
        <span class="aeq-search">
          <input
            ref={search}
            type="search"
            name="autoeq-search"
            value={query}
            placeholder="Search headphones…"
            aria-label="Search headphones"
            onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
          />
        </span>
        <span class="aeq-count" aria-live="polite">
          {count} headphones
        </span>
        <ActionMenuButton
          label="More"
          items={() =>
            menuItems("autoeq", {
              "autoeq.clearFavorites": { run: () => void lib.clearFavourites(session), enabled: libState.favourites.length > 0 },
              "autoeq.updateDatabase": { run: () => setDbOpen(true) },
            })
          }
        >
          <span class="visually-hidden">More</span>
        </ActionMenuButton>
      </PageTools>
      <div class="aeq-columns">
        <div class="aeq-list">
          {libState.problem && !libState.info ? (
            <EmptyState icon="warning" title="The headphone database did not load" text={libState.problem} />
          ) : list.length ? (
            <>
              <ul class="lib-rows" aria-label={found ? "Headphones" : "Favorite Profiles"}>
                {list.map(row)}
              </ul>
              {found && found.total > found.hits.length ? <p class="lib-caption">{found.total - found.hits.length} more: search more narrowly.</p> : null}
            </>
          ) : found ? (
            <EmptyState
              icon="headphones"
              title="No headphones found"
              text={`Nothing matches “${query.trim()}”.`}
              action={{
                label: "Clear Search",
                run: () => {
                  setQuery("");
                  search.current?.focus();
                },
              }}
            />
          ) : (
            <EmptyState icon="headphones" title="Search the headphones" text="Type a maker or a model in the search above. Your favorites are listed here." />
          )}
        </div>
        <div class="aeq-side" role="group" aria-label="Headphone profile">
          <h4 class="section-label">Headphone profile</h4>
          {chosen ? <p class="aeq-name">{chosen.hit.name}</p> : null}
          <p class={chosen ? "lib-sub" : "aeq-hint"}>{chosen ? hitLine(chosen.hit) : "Select a headphone to apply its EQ profile"}</p>
          <div class="aeq-apply">
            <label class="aeq-pair">
              Apply to
              <select
                name="autoeq-pair"
                value={String(pairAt)}
                disabled={!pairs.length}
                onChange={(e) => {
                  setPairAt(Number((e.target as HTMLSelectElement).value));
                  setOutcome(null);
                }}
              >
                {pairs.map((p, i) => (
                  <option key={p.title} value={String(i)}>
                    {p.title}
                  </option>
                ))}
              </select>
            </label>
            <button type="button" class="primary" disabled={!chosen || !pair || busy || !!ro} title={ro || "Put the profile on the chosen channels"} onClick={() => void apply()}>
              Apply
            </button>
          </div>
          {outputs ? (
            <label class="aeq-check">
              <input type="checkbox" checked={applyLevel} onChange={(e) => setApplyLevel((e.target as HTMLInputElement).checked)} /> Set the outputs' gain too
            </label>
          ) : null}
          <figure class="aeq-frame" aria-label="The profile's response on the chosen channels">
            {curves.length && freqs ? (
              <svg viewBox={`0 0 ${PLOT.w} ${PLOT.h}`} preserveAspectRatio="none" role="img" aria-label={`The response of ${chosen?.hit.name ?? ""}`}>
                {TICKS_HZ.map((f) => (
                  <line key={f} class="aeq-grid" x1={xOf(f)} x2={xOf(f)} y1={0} y2={PLOT.h} />
                ))}
                {[-span / 2, span / 2].map((d) => (
                  <line key={d} class="aeq-grid" x1={0} x2={PLOT.w} y1={yOf(d, span)} y2={yOf(d, span)} />
                ))}
                <line class="aeq-zero" x1={0} x2={PLOT.w} y1={yOf(0, span)} y2={yOf(0, span)} />
                {curves.map((c, i) => (
                  <path key={c.channel} class={`aeq-curve${i ? " second" : ""}`} d={curvePath(freqs, c.curve, 0, span)} vector-effect="non-scaling-stroke" />
                ))}
              </svg>
            ) : (
              <p class="aeq-empty">Select a headphone to see what it would do</p>
            )}
            {curves.length && freqs ? (
              <div class="aeq-ticks" aria-hidden="true">
                {TICKS_HZ.map((f) => (
                  <span key={f} class="aeq-hz" style={{ left: `${(xOf(f) / PLOT.w) * 100}%` }}>
                    {f >= 1000 ? `${f / 1000}k` : f}
                  </span>
                ))}
                {[span / 2, -span / 2].map((d) => (
                  <span key={d} class="aeq-dbl" style={{ top: `${(yOf(d, span) / PLOT.h) * 100}%` }}>
                    {d > 0 ? `+${d}` : `−${-d}`}
                  </span>
                ))}
              </div>
            ) : null}
          </figure>
          {preview ? (
            <div class="aeq-preview" role="status" aria-label="AutoEQ preview">
              <p>{levelLine(preview)}</p>
              {preview.held.map((h, i) => (
                <p key={`h${i}`}>{h}</p>
              ))}
              {preview.notes.map((n, i) => (
                <p key={`n${i}`}>{n}</p>
              ))}
            </div>
          ) : null}
          {outcome ? (
            <NoticeRow
              kind={outcome.tone}
              role={outcome.tone === "error" ? "alert" : "status"}
              action={outcome.before ? { label: "Download the State Before", run: () => download(outcome.before, "before-autoeq.dspipreset") } : undefined}
            >
              <span aria-label="AutoEQ result">{outcome.lines.join(" ")}</span>
            </NoticeRow>
          ) : null}
        </div>
      </div>
      {dbOpen ? (
        <Dialog title="Update Database" buttons={[{ label: "Done", onClick: () => setDbOpen(false) }]} onCancel={() => setDbOpen(false)}>
          <AutoEqDatabasePanel session={session} />
        </Dialog>
      ) : null}
    </section>
  );
}
