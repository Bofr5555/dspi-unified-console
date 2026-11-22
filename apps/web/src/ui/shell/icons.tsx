// -- WP WEBM1 -- The shell's icons, as MODLAY draws them: 24 px, stroked in the text's colour.

import type { ComponentChildren } from "preact";
import type { SectionId } from "./sections";

function Icon(props: { children: ComponentChildren }) {
  return (
    <svg class="shell-icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      {props.children}
    </svg>
  );
}

const SECTION: Record<SectionId, ComponentChildren> = {
  overview: (
    <>
      <rect x="3" y="3" width="7" height="9" rx="1.5" />
      <rect x="14" y="3" width="7" height="5" rx="1.5" />
      <rect x="14" y="12" width="7" height="9" rx="1.5" />
      <rect x="3" y="16" width="7" height="5" rx="1.5" />
    </>
  ),
  equaliser: (
    <>
      <path d="M2 14c3 0 3-7 6-7s3 10 6 10 3-6 8-6" />
      <circle cx="8" cy="7" r="1.6" />
      <circle cx="14" cy="17" r="1.6" />
    </>
  ),
  routing: (
    <>
      <circle cx="5" cy="6" r="2" />
      <circle cx="5" cy="18" r="2" />
      <circle cx="19" cy="6" r="2" />
      <circle cx="19" cy="18" r="2" />
      <path d="M7 6h10M7 18h10M7 7l10 10" />
    </>
  ),
  effects: (
    <>
      <path d="M3 18v-3a9 9 0 0 1 18 0v3" />
      <rect x="2" y="15" width="5" height="6" rx="1.5" />
      <rect x="17" y="15" width="5" height="6" rx="1.5" />
    </>
  ),
  measure: (
    <>
      <rect x="9" y="2" width="6" height="12" rx="3" />
      <path d="M5 11a7 7 0 0 0 14 0M12 18v4" />
    </>
  ),
  analyse: <path d="M2 12h4l3-8 4 16 3-8h6" />,
  library: (
    <>
      <path d="M12 3 2 8l10 5 10-5-10-5z" />
      <path d="m2 13 10 5 10-5" />
      <path d="m2 17.5 10 5 10-5" opacity="0.5" />
    </>
  ),
  system: (
    <>
      <rect x="9" y="2" width="6" height="5" rx="1" />
      <rect x="2" y="17" width="6" height="5" rx="1" />
      <rect x="16" y="17" width="6" height="5" rx="1" />
      <path d="M12 7v5M5 17v-2.5h14V17" />
    </>
  ),
  devices: (
    <>
      <rect x="5" y="7" width="14" height="10" rx="2" />
      <path d="M9 17v4h6v-4M9 3v4M15 3v4" />
    </>
  ),
  settings: (
    <>
      <path d="M4 7h9M17 7h3M4 17h3M11 17h9" />
      <circle cx="15" cy="7" r="2" />
      <circle cx="9" cy="17" r="2" />
    </>
  ),
};

export function SectionIcon(props: { id: SectionId }) {
  return <Icon>{SECTION[props.id]}</Icon>;
}

export function SaveIcon() {
  return (
    <Icon>
      <path d="M5 3h11l3 3v13a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z" />
      <path d="M7 3v5h8V3M7 21v-7h10v7" />
    </Icon>
  );
}

export function LockIcon(props: { open: boolean }) {
  return (
    <Icon>
      <rect x="4" y="11" width="16" height="10" rx="2" />
      <path d={props.open ? "M8 11V7a4 4 0 0 1 7.8-1.2" : "M8 11V7a4 4 0 0 1 8 0v4"} />
    </Icon>
  );
}

export function SpeakerIcon() {
  return (
    <Icon>
      <path d="M11 5 6 9H2v6h4l5 4V5z" />
      <path d="M15.5 8.5a5 5 0 0 1 0 7M19 5a10 10 0 0 1 0 14" />
    </Icon>
  );
}
