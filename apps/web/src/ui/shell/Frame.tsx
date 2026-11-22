// -- WP WEBM1 -- The frame of a session's view: its bar, the rail, the page under the status
// strip, the foot and the dock. `screens` marks the frame of an open device (the keys act on it).

import type { ComponentChildren, RefObject } from "preact";
import { PageTabs, Rail } from "./Rail";
import { pageToOpen, type Section, type SectionId } from "./sections";

export function Frame(props: {
  root?: RefObject<HTMLDivElement | null>;
  screens?: boolean;
  /** The session's bar, above the rail and the page. */
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
      <div class="session-bar">{props.bar}</div>
      <div class="shell-body">
        <Rail sections={props.sections} current={section.id} onSection={(s) => props.onPage(pageToOpen(s, props.last))} />
        <div class="shell-main">
          <div class="status-strip">{props.strip}</div>
          <div class="page-head">
            <h2 class="page-title">{section.title}</h2>
            <PageTabs pages={section.pages} current={props.page} onPage={props.onPage} />
            {props.tools}
          </div>
          <div class="page" data-page={props.page}>
            {props.children}
          </div>
          {props.foot}
        </div>
      </div>
      {props.dock}
    </div>
  );
}
