// -- WP WEBM1 -- The sections, in the rail (or, arranged so, as a row across the top; the same
// buttons, placed by the stylesheet), and the tabs of one section's pages.

import { useContext, useEffect, useRef } from "preact/hooks";
import { RailTopContext } from "../arrangement/Appearance"; // -- WP WEBM4 --
import { SectionIcon } from "./icons";
import { tabKeys } from "./tab-keys";
import type { Page, Section, SectionId } from "./sections";

export function Rail(props: { sections: Section[]; current: SectionId; onSection: (s: Section) => void }) {
  const nav = useRef<HTMLElement>(null);
  // As a row (a narrow console) it scrolls sideways: the section on show stays in sight. Its own
  // scroll only, never the page's.
  useEffect(() => {
    const row = nav.current;
    const on = row?.querySelector<HTMLElement>('[aria-current="page"]');
    if (!row || !on || row.scrollWidth <= row.clientWidth) return;
    row.scrollLeft = on.offsetLeft - (row.clientWidth - on.offsetWidth) / 2;
  }, [props.current, props.sections.length]);
  // -- WP WEBM4 -- the open devices' tabs at its top, when so arranged
  const top = useContext(RailTopContext);
  const stop = props.sections.some((s) => s.id === props.current) ? props.current : props.sections[0]?.id; // -- WP UWN --
  return (
    <nav class="rail" aria-label="Sections" ref={nav}>
      {top ? <div class="rail-devices">{top}</div> : null}
      {/* -- WP UWN -- (UXP A15) one Tab stop (the section on show) and the arrow keys between sections, as the page tabs */}
      <div class="rail-list" onKeyDown={railKeys}>
        {props.sections.map((s) => (
          <button
            type="button"
            key={s.id}
            class={s.foot ? "rail-foot" : undefined}
            tabIndex={s.id === stop ? 0 : -1}
            aria-current={s.id === props.current ? "page" : undefined}
            title={s.title}
            onClick={() => props.onSection(s)}
          >
            <SectionIcon id={s.id} />
            <span class="rail-text">{s.title}</span>
          </button>
        ))}
      </div>
    </nav>
  );
}

// -- WP UWN -- (UXP A15)
/** The rail's arrow keys: Up/Left and Down/Right move the focus between sections (it wraps), Home and End to the ends; Enter or Space opens one. */
export function railKeys(e: KeyboardEvent): void {
  const list = e.currentTarget as HTMLElement;
  const buttons = [...list.querySelectorAll<HTMLButtonElement>(":scope > button")];
  const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
  if (at < 0 || !buttons.length) return;
  const n = buttons.length;
  const to =
    e.key === "ArrowDown" || e.key === "ArrowRight" ? (at + 1) % n : e.key === "ArrowUp" || e.key === "ArrowLeft" ? (at + n - 1) % n : e.key === "Home" ? 0 : e.key === "End" ? n - 1 : -1;
  if (to < 0) return;
  e.preventDefault();
  buttons[to]!.focus();
}

/** The tabs of a section's pages; none for a section with one. */
export function PageTabs(props: { pages: Page[]; current: string; onPage: (id: string) => void }) {
  const { pages } = props;
  if (pages.length < 2) return null;
  const at = pages.findIndex((p) => p.id === props.current);
  return (
    <div class="page-tabs" role="tablist" aria-label="Pages" onKeyDown={tabKeys(pages.length, at, (i) => props.onPage(pages[i]!.id))}>
      {pages.map((p, i) => (
        <button type="button" role="tab" key={p.id} aria-selected={i === at} tabIndex={i === Math.max(0, at) ? 0 : -1} onClick={() => props.onPage(p.id)}>
          {p.label}
        </button>
      ))}
    </div>
  );
}
