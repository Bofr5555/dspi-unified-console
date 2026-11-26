// -- WP REVEFIX -- (REVD D12, REVE §3.1) The console with no DSPi open, as Qt's: the whole rail, the
// status strip saying so ("No DSPi is connected…" with Open Demo), and on every page its title, its
// tabs and an empty state whose action opens the devices' page. With nothing open at all the console
// starts on that page (Devices › Open a DSPi); a tab whose DSPi failed or went away starts on the
// Overview, as Qt's no-device window does.

import type { ComponentChildren } from "preact";
import { useMemo, useState } from "preact/hooks";
import { EmptyState, Glyph } from "../design";
import { screensFor } from "../screens";
import type { ShellProps } from "../components";
import { Frame } from "./Frame";
import { NoDeviceSettings } from "../../screens/settings/SettingsScreen"; // -- WP UWN --
import { CONNECT, sectionOf, sectionsFor, type SectionId } from "./sections";

const NO_PAGES: ReadonlyMap<SectionId, string> = new Map();

/** The strip's line while no DSPi is open: Qt's words, with the demo one press away. */
export function NoDeviceStrip(props: { openDemo: (() => void) | null }) {
  return (
    <div class="banner banner-info no-device-strip" role="status">
      <Glyph name="unplug" class="banner-icon" />
      <span>No DSPi is connected. Connect one over USB, or try the demo device.</span>
      {props.openDemo ? (
        <button type="button" onClick={props.openDemo}>
          Open Demo
        </button>
      ) : null}
    </div>
  );
}

/** A page's empty state with no DSPi: the Overview says what it would show, the others what to do. */
export function NoDeviceEmpty(props: { overview: boolean; hub?: boolean; onChoose: () => void }) {
  const action = { label: "Choose a Device…", primary: true, run: props.onChoose };
  return props.overview ? (
    <EmptyState icon="plug" title="No Device Connected" text="Open a DSPi, a hub's device or the demo to see its channels here." action={action} />
  ) : (
    <EmptyState icon="unplug" title="No device" text={props.hub ? "Open the hub's DSPi again, or another one." : "Connect one over USB or open the demo."} action={action} />
  );
}

/**
 * The frame with no DSPi: every section of the Advanced mode in the rail, each page's tabs, and
 * `empty` on each (the Overview's own words when `overview`); Devices › Open a DSPi is the page's.
 */
export function NoDeviceFrame(props: {
  shell: ShellProps;
  strip: ComponentChildren;
  /** The page shown first: the Overview's first unless said. */
  start?: string;
  /** On the way to a device (connecting, a PIN, a hub's list): the Devices section alone, as before a device answers. */
  connecting?: boolean;
  empty?: (overview: boolean) => ComponentChildren;
  /** -- WP UWN -- (UXP A16) over the devices' page: why the last connect failed, with its way out. */
  connectNote?: ComponentChildren;
}) {
  const { shell } = props;
  const connecting = props.connecting === true;
  const sections = useMemo(() => sectionsFor(connecting ? [] : screensFor("advanced", false)), [connecting]);
  const [chosen, setChosen] = useState(connecting ? CONNECT : (props.start ?? "main"));
  const page = shell.picking ? CONNECT : chosen;
  const go = (id: string) => {
    setChosen(id);
    shell.onPicking(id === CONNECT);
  };
  const overview = sectionOf(page) === "overview";
  return (
    <Frame bar={shell.top()} strip={props.strip} sections={sections} page={page} last={NO_PAGES} onPage={go}>
      {page === CONNECT && !(connecting && !shell.picking) ? (
        <>
          {props.connectNote}
          {shell.connect}
        </>
      ) : page === "settings" && !connecting ? (
        // -- WP UWN -- (UXP A16) Settings' Application and Display parts need no DSPi
        <NoDeviceSettings />
      ) : props.empty ? (
        props.empty(overview)
      ) : (
        <NoDeviceEmpty overview={overview} onChoose={() => go(CONNECT)} />
      )}
    </Frame>
  );
}
