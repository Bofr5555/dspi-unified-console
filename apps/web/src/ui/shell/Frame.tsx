// -- WP WEBM1 -- The frame of a session's view: its bar, the rail, the page under the status
// strip, the foot and the dock. `screens` marks the frame of an open device (the keys act on it).

import type { ComponentChildren, RefObject } from "preact";
import { PageTabs, Rail } from "./Rail";
import { pageToOpen, type Section, type SectionId } from "./sections";
// -- WP WEBM2 -- the inspector beside the page
import { InspectorArea } from "../inspector/Inspector";
// -- WP WEBM3 -- a page's parts as tabs in the head
import { PartsArea, PartsSlot } from "./Parts";

// -- WP UWN -- (UXP A15)
/** A skip link's jump: the focus to the first element of the shell matching `selector`. */
function skipTo(e: MouseEvent, selector: string): void {
  e.preventDefault();
  const shell = (e.currentTarget as HTMLElement).closest(".shell");
  shell?.querySelector<HTMLElement>(selector)?.focus();
}

export function Frame(props: {
  root?: RefObject<HTMLDivElement | null>;
  screens?: boolean;
  /** The top bar, above the rail and the page. */
  bar: ComponentChildren;
  /** What limits what can be done now, above the page. */
  strip: ComponentChildren;
  sections: Section[];
  /** The page on show. */
  page: string;
  /** The last page of each section. */
  last: ReadonlyMap<SectionId, string>;
  onPage: (id: string) => void;
  /** Beside the tabs. */
  tools?: ComponentChildren;
  children: ComponentChildren;
  foot?: ComponentChildren;
  dock?: ComponentChildren;
}) {
  const section = props.sections.find((s) => s.pages.some((p) => p.id === props.page)) ?? props.sections[0]!;
  return (
    <div class={`shell${props.screens ? " session-screens" : ""}`} ref={props.root}>
      {/* -- WP UWN -- (UXP A15) skip links: the first Tab stops, shown when focused */}
      <nav class="skip-links" aria-label="Skip links">
        <a class="skip-link" href="#" onClick={(e) => skipTo(e, ".page")}>
          Skip to the page
        </a>
        {props.dock ? (
          <a class="skip-link" href="#" onClick={(e) => skipTo(e, ".dock input, .dock button")}>
            Skip to the volume
          </a>
        ) : null}
      </nav>
      {props.bar /* -- WP REVEFIX -- the top bar itself (Qt's TopBar), not a session bar under it */}
      <div class="shell-body">
        <Rail sections={props.sections} current={section.id} onSection={(s) => props.onPage(pageToOpen(s, props.last))} />
        <div class="shell-main">
          <div class="status-strip">{props.strip}</div>
          {/* -- WP WEBM3 -- the page's parts (ui/shell/Parts.tsx) in its head */}
          <PartsArea>
            <div class="page-head">
              <h2 class="page-title">{section.title}</h2>
              <PageTabs pages={section.pages} current={props.page} onPage={props.onPage} />
              <PartsSlot />
              {props.tools}
            </div>
            {/* -- WP WEBM2 -- a page may fill the inspector beside it (ui/inspector) */}
            <InspectorArea>
              <div class="page" data-page={props.page} tabIndex={-1 /* -- WP UWN -- the skip link's target */}>
                {props.children}
              </div>
            </InspectorArea>
          </PartsArea>
          {props.foot}
        </div>
      </div>
      {props.dock}
    </div>
  );
}
