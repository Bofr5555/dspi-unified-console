// -- WP WEBM1 -- The sections, in the rail (or, arranged so, as a row across the top; the same
// buttons, placed by the stylesheet), and the tabs of one section's pages.

import { useEffect, useRef } from "preact/hooks";
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
  return (
    <nav class="rail" aria-label="Sections" ref={nav}>
      <div class="rail-list">
        {props.sections.map((s) => (
          <button
            type="button"
            key={s.id}
            class={s.foot ? "rail-foot" : undefined}
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
