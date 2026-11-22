// The web console as a library (WP WB5).
//
//   import { mount, LocalBackend } from "./dspi-web.js";
//   const c = mount(document.getElementById("console"), { backend: new LocalBackend() });
//   ...
//   c.unmount();
//
// A fleet server passes its own FleetBackend (its device list, its sign-in, its
// roles) and serves `worker.js` beside the library. Several consoles can be
// mounted on one page; each keeps its own sessions and Workers. Styles are
// scoped under `.dspi-web` (dspi-web.css).

import { render } from "preact";
import { CoreClient } from "./core/client";
import { LocalBackend } from "./fleet/local";
import type { FleetBackend } from "./fleet/types";
import { App, type ConsoleOptions } from "./ui/App";
// -- WP WB6b --
import { UnsavedWatch } from "./ui/unsaved";
// -- WP WEBM1 --
import { loadNavigation, type Navigation } from "./ui/shell/sections";
import { safeLocalStorage } from "./fleet/local";

export interface MountOptions {
  /** Default: a LocalBackend (standalone). */
  backend?: FleetBackend;
  /** Where worker.js is. Default: beside this module. */
  workerUrl?: string | URL;
  title?: string;
  /** Link to this build's source (GPL). Default: the build's own (`DSPI_WEB_SOURCE_URL`). */
  sourceUrl?: string | null;
  /** -- WP WB7 -- A link to the page of the hub that serves this one, shown in the header. */
  hubPage?: string | null;
  readOnly?: boolean;
  /** Advanced: replaces the Worker, for tests or another runtime. */
  spawnCore?: () => CoreClient;
  // -- WP WB6b --
  /**
   * Leaving the page while something is unsaved asks the browser's own
   * question. Default true; a host page that asks in its own way sets false
   * and uses `unsaved()` and `onUnsavedChange`.
   */
  leavePrompt?: boolean;
  // -- WP WEBM1 --
  /** Where the sections are: in a rail, or as tabs across the top. Default: as this viewer has it (a rail). */
  navigation?: Navigation;
}

// -- WP WB7 -- set at build time (tools/build.mjs, DSPI_WEB_SOURCE_URL)
declare const __DSPI_SOURCE_URL__: string | undefined;
const BUILD_SOURCE_URL = typeof __DSPI_SOURCE_URL__ === "string" ? __DSPI_SOURCE_URL__ : "";

export interface MountedConsole {
  unmount(): void;
  // -- WP WB6b --
  /** Whether any of this console's open sessions has changes not saved to its preset. */
  unsaved(): boolean;
  /** Called with the new answer whenever `unsaved()` changes; the result stops it. */
  onUnsavedChange(fn: (unsaved: boolean) => void): () => void;
}

export function mount(el: HTMLElement, opts: MountOptions = {}): MountedConsole {
  const workerUrl = opts.workerUrl ?? new URL("./worker.js", import.meta.url);
  const options: ConsoleOptions = {
    backend: opts.backend ?? new LocalBackend(),
    spawnCore: opts.spawnCore ?? (() => CoreClient.spawn(workerUrl)),
    title: opts.title ?? "uConsole for DSPi",
    sourceUrl: opts.sourceUrl ?? (BUILD_SOURCE_URL || null),
    hubPage: opts.hubPage ?? null,
    readOnly: opts.readOnly ?? false,
    unsaved: new UnsavedWatch(opts.leavePrompt ?? true),
    navigation: opts.navigation ?? loadNavigation(safeLocalStorage()), // -- WP WEBM1 --
  };
  const watch = options.unsaved!;
  render(<App options={options} />, el);
  return {
    unmount: () => render(null, el),
    unsaved: () => watch.unsaved,
    onUnsavedChange: (fn) => watch.onChange(fn),
  };
}

export { CoreClient } from "./core/client";
export { CoreCallError } from "./core/protocol";
export type { CoreEvent, DeviceSnapshot, DeviceTarget, LinkPhase } from "./core/protocol";
export { LocalBackend, normaliseHubUrl } from "./fleet/local";
export { VIEW_ONLY_REASON } from "./fleet/types";
export type { Access, FleetBackend, FleetUser, Installation, Source } from "./fleet/types";
// -- WP P1WB2 --
export type { PinnedMode } from "./fleet/types";
// -- WP WEBM1 --
export type { Navigation } from "./ui/shell/sections";
