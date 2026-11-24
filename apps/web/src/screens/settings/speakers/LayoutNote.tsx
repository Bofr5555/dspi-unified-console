// WP ADETW: the speaker layout's state on the main screen (FUNC2c section 3: an unconfirmed setup
// was invisible there). A kept layout that is not confirmed says so in one line, auto-detected or
// unconfirmed, and where to confirm it; a confirmed one, or none kept yet, shows nothing.

import { useEffect, useState } from "preact/hooks";
import { safeLocalStorage } from "../../../fleet/local";
import type { Session } from "../../../ui/session";
import { bannerOf, peekLayout, type LayoutCaller, type LayoutView } from "./model";
import { serialOf } from "./use-layout";

export function LayoutNote(props: { session: Session }) {
  const { session } = props;
  const serial = serialOf(session);
  const [view, setView] = useState<LayoutView | null>(null);
  useEffect(() => {
    let alive = true;
    void peekLayout(session.core as unknown as LayoutCaller, serial, safeLocalStorage()).then(
      (v) => alive && setView(v),
      () => alive && setView(null),
    );
    return () => {
      alive = false;
    };
  }, [session, serial]);
  const banner = view?.stored ? bannerOf(view) : null;
  if (!banner) return null;
  return (
    <div class="banner main-layout-note" role="status" aria-label="Speaker layout" data-kind={banner.kind} style={{ flexWrap: "wrap", margin: 0 }}>
      <span style={{ flex: "1 1 14em", minWidth: 0 }}>{banner.text}</span>
    </div>
  );
}
