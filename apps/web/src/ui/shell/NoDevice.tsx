// -- WP REVEFIX -- (REVD D12, REVE §3.1) The console with no DSPi open, as Qt's: the whole rail, the
// status strip saying so ("No DSPi is connected…" with Open Demo), and on every page its title, its
// tabs and an empty state whose action opens the devices' page. With nothing open at all the console
// starts on that page (Devices › Open a DSPi); a tab whose DSPi failed or went away starts on the
// Overview, as Qt's no-device window does.

import type { ComponentChildren } from "preact";
import { useEffect, useMemo, useState } from "preact/hooks";
import { EmptyState, Glyph } from "../design";
import { screensFor } from "../screens";
import type { ShellProps } from "../components";
import { Frame } from "./Frame";
import { NoDeviceSettings } from "../../screens/settings/SettingsScreen"; // -- WP UWN --
import { CONNECT, FLEET, sectionOf, sectionsFor, type SectionId } from "./sections";

const NO_PAGES: ReadonlyMap<SectionId, string> = new Map();

// -- WP UB6 -- (UXP B6)
/** What a hub-served page's strip says of its hub: how many DSPis it shares now (null: not known yet), and the way to them. */
export interface StripHub {
  shared: number | null;
  onOpen: (() => void) | null;
}

/** "This hub shares 2 DSPis." */
export function hubSharesText(shared: number | null): string {
  if (shared === null) return "Asking the hub which DSPis it shares…";
  return shared === 0 ? "This hub shares no DSPi right now." : `This hub shares ${shared} ${shared === 1 ? "DSPi" : "DSPis"}.`;
}

/** The strip's line while no DSPi is open: Qt's words, with the demo one press away; -- WP UB6 -- on a hub's page, what the hub shares. */
export function NoDeviceStrip(props: { openDemo: (() => void) | null; hub?: StripHub | null }) {
  if (props.hub)
    return (
      <div class="banner banner-info no-device-strip" role="status">
        <Glyph name="network" class="banner-icon" />
        <span>{hubSharesText(props.hub.shared)}</span>
        {props.hub.onOpen ? (
          <button type="button" onClick={props.hub.onOpen}>
            Open…
          </button>
        ) : null}
      </div>
    );
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
  /** -- WP UB6 -- or made with the way to a page (the hub strip's Open… goes to the Fleet) */
  strip: ComponentChildren | ((go: (page: string) => void) => ComponentChildren);
  /** The page shown first: the Overview's first unless said. */
  start?: string;
  /** On the way to a device (connecting, a PIN, a hub's list): the Devices section alone, as before a device answers. */
  connecting?: boolean;
  empty?: (overview: boolean) => ComponentChildren;
  /** -- WP UWN -- (UXP A16) over the devices' page: why the last connect failed, with its way out. */
  connectNote?: ComponentChildren;
  /** -- WP UB6 -- (UXP B6, D13) a hub-served console: Devices' first page, the hub's DSPis with none open. */
  fleet?: ComponentChildren;
}) {
  const { shell } = props;
  const connecting = props.connecting === true;
  const fleet = props.fleet !== undefined && !connecting;
  const sections = useMemo(() => sectionsFor(connecting ? [] : screensFor("advanced", false), fleet), [connecting, fleet]);
  const [chosen, setChosen] = useState(connecting ? CONNECT : (props.start ?? "main"));
  // -- WP UB6 -- the page to start on may come later (the hub's list is read after the page shows): followed until a page is chosen
  const [moved, setMoved] = useState(false);
  useEffect(() => {
    if (!moved && !connecting && props.start) setChosen(props.start);
  }, [props.start]);
  const page = shell.picking ? CONNECT : chosen === FLEET && !fleet ? CONNECT : chosen;
  const go = (id: string) => {
    setMoved(true);
    setChosen(id);
    shell.onPicking(id === CONNECT);
  };
  const overview = sectionOf(page) === "overview";
  return (
    <Frame bar={shell.top()} strip={typeof props.strip === "function" ? props.strip(go) : props.strip} sections={sections} page={page} last={NO_PAGES} onPage={go}>
      {page === FLEET ? (
        props.fleet
      ) : page === CONNECT && !(connecting && !shell.picking) ? (
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
