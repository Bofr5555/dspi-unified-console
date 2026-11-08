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

export interface MountOptions {
  /** Default: a LocalBackend (standalone). */
  backend?: FleetBackend;
  /** Where worker.js is. Default: beside this module. */
  workerUrl?: string | URL;
  title?: string;
  sourceUrl?: string | null;
  readOnly?: boolean;
  /** Advanced: replaces the Worker, for tests or another runtime. */
  spawnCore?: () => CoreClient;
}

export interface MountedConsole {
  unmount(): void;
}

export function mount(el: HTMLElement, opts: MountOptions = {}): MountedConsole {
  const workerUrl = opts.workerUrl ?? new URL("./worker.js", import.meta.url);
  const options: ConsoleOptions = {
    backend: opts.backend ?? new LocalBackend(),
    spawnCore: opts.spawnCore ?? (() => CoreClient.spawn(workerUrl)),
    title: opts.title ?? "DSPi Console",
    sourceUrl: opts.sourceUrl ?? null,
    readOnly: opts.readOnly ?? false,
  };
  render(<App options={options} />, el);
  return { unmount: () => render(null, el) };
}

export { CoreClient } from "./core/client";
export { CoreCallError } from "./core/protocol";
export type { CoreEvent, DeviceSnapshot, DeviceTarget, LinkPhase } from "./core/protocol";
export { LocalBackend, normaliseHubUrl } from "./fleet/local";
export { VIEW_ONLY_REASON } from "./fleet/types";
export type { Access, FleetBackend, FleetUser, Installation, Source } from "./fleet/types";
