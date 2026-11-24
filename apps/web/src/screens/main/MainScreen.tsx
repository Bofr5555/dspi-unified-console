// The Overview's dashboard of one session (WP WB6a; WEB2/WEB3's DL6; INTD): the tiles (the device,
// its effects and its connection), the channel cards, the analyser's strip and the speaker
// layout's note. It lives inside its session's view; a fleet page holds several at once, so
// nothing here is the page's. -- WP INTD -- The presets are the session bar's popover (DL5), the
// levels are Equaliser › Inputs and Delay & Gain's, and the cards wrap by width (no layout picker),
// as on Qt's Overview.

import { useEffect, useState } from "preact/hooks";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
// -- WP WB6J -- the analyser's bar strip (M22)
import { SpectrumStrip, tapNames } from "../analysis/SpectrumStrip";
import { LayoutNote } from "../settings/speakers/LayoutNote"; // -- WP ADETW --
import { DashboardTiles } from "./Tiles"; // -- WP WEB2 --
import { ChannelCards } from "../overview/Cards"; // -- WP WEB3 -- (DL6) the channel cards under the tiles

// -- WP WB6b -- the questions and the unsaved-changes prompt are the session
// view's now (src/ui/screens.tsx), so every screen shares them.
export type { Ask, Question } from "../../ui/screens";

export function MainScreen(props: ScreenProps) {
  const { session } = props;
  const st = useSessionState(session);
  const snap = st.snapshot;

  // How many inputs USB streams, from the device's status (the Console lists
  // those): read now and every 2 s while the page is seen; a redraw only when
  // the count changes.
  const [usbActive, setUsbActive] = useState<number | null>(null);
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = async () => {
      if (!alive) return;
      if (document.visibilityState === "visible" && session.state.status === "open") {
        try {
          const n = (await session.core.call("meters", undefined)).activeInputChannels;
          if (alive) setUsbActive(n);
        } catch {
          // Kept: the last count stands.
        }
      }
      if (alive) timer = setTimeout(read, 2000);
    };
    void read();
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
    };
  }, [session]);

  const s = snap?.state;
  if (!s || !snap) return null;
  return (
    <div class="main-screen">
      {/* -- WP WEB2 -- (DL6) the device, its effects and its connection first, as Qt's Overview */}
      <DashboardTiles session={session} open={props.open} />
      <ChannelCards session={session} snap={snap} usbActive={usbActive} open={props.open} />
      <div class="main-spectrum">
        <SpectrumStrip session={session} names={tapNames(s, snap.outputLabels)} />
      </div>
      {/* -- WP ADETW -- a kept speaker layout that is not confirmed, auto-detected or not */}
      <LayoutNote session={session} />
    </div>
  );
}
