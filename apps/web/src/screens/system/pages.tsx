// -- WP WEB7 -- The System section's pages as Qt Modern's (SectionTable.qml: Units | Clock and Links |
// Installation, then Events): each its own tab, and on a device this console holds itself (not on
// a hub) one empty state with the way to the hubs, as Qt's SystemPanel. The web's signal chain
// (WB6d), which Qt has no page for, is the section's last tab.

import { useEffect } from "preact/hooks";
import type { ScreenProps } from "../../ui/screens";
import { EmptyState, NoticeRow } from "../../ui/design";
import { CONNECT } from "../../ui/shell/sections";
import { AlinkClock } from "./AlinkClock";
import type { SystemFeed } from "./feed";
import { InstallationTab } from "./installation/InstallationTab";
import { SignalChain } from "./SignalChain";
import { useFeed } from "./SystemScreen";
import "./system.css";

/** Qt's words when the session is not on a hub: Units, Clock and Links and Installation say the same. */
export function NotOnHub(props: { go?: (screen: string) => void; error?: string }) {
  return (
    <div class="sys-page sys-empty">
      <EmptyState
        icon="network"
        title="Not on a hub"
        text={props.error || "Open a DSPi on a hub with an audio link to see its installation here."}
        action={props.go ? { label: "Hubs and Their Devices", run: () => props.go?.(CONNECT) } : undefined}
      />
    </div>
  );
}

/** While a page shows the feed (it reads a device on its own only then). */
export function useShown(feed: SystemFeed | undefined): void {
  useEffect(() => feed?.show(), [feed]);
}

/** Clock and Links: the audio link's delays, slots, alignment and stats (AlinkClock), the hub's Admin edits. */
export function ClockScreen(props: ScreenProps) {
  const feed = props.system;
  if (!feed) return null;
  if (!feed.onHub) return <NotOnHub go={props.go} />;
  return <ClockPage feed={feed} ask={props.ask} />;
}

function ClockPage(props: { feed: SystemFeed; ask: ScreenProps["ask"] }) {
  const { feed } = props;
  useShown(feed);
  const st = useFeed(feed);
  if (!st.model) return <p class="sys-note">{st.error || "Reading the system…"}</p>;
  if (st.kind === "single")
    return (
      <div class="sys-page sys-empty">
        <EmptyState icon="timer" title="No audio link installation" text="This hub has no audio link installation: the audio link's alignment is set by a hub with an installation." />
      </div>
    );
  return (
    <div class="sys-page">
      {st.error ? <NoticeRow kind="warning">{st.error}</NoticeRow> : null}
      <AlinkClock feed={feed} ask={props.ask} />
    </div>
  );
}

/** Installation: setting the installation up on the hub (INSTW), Advanced and Installer. */
export function InstallationScreen(props: ScreenProps) {
  const feed = props.system;
  if (!feed) return null;
  if (!feed.onHub) return <NotOnHub go={props.go} />;
  return (
    <div class="sys-page">
      <InstallationTab feed={feed} session={props.session} ask={props.ask} />
    </div>
  );
}

/** The web's own: this device's signal chain, read only; a block opens the page that edits it. */
export function ChainScreen(props: ScreenProps) {
  return <SignalChain session={props.session} open={props.open} />;
}

/** The System section's pages after Units (ui/screens.tsx lists them; a function, so a module cycle reads it whole). */
export function isSystemPage(id: string): boolean {
  return id === "system-clock" || id === "system-installation" || id === "chain";
}
