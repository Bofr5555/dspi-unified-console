// What this page can reach (WP WB5). Each answer carries the reason, in words a
// user can act on, for the picker to show beside a source it cannot offer.
// No text names the device the user is holding (owner's rule).

export interface Availability {
  ok: boolean;
  reason: string;
}

export interface PageEnv {
  usb: Availability;
  /** Link to a hub: needs WB4b's transport. */
  link: Availability;
}

const LATER = "Not available in this version yet";

export function pageEnv(g: { isSecureContext?: boolean; navigator?: unknown } = globalThis): PageEnv {
  const nav = g.navigator as { usb?: unknown } | undefined;
  let usb: Availability;
  if (!g.isSecureContext) {
    usb = { ok: false, reason: "USB needs a secure page (https:// or http://localhost)" };
  } else if (!nav || !("usb" in nav) || !nav.usb) {
    usb = { ok: false, reason: "USB needs a Chromium-based browser, such as Chrome or Edge" };
  } else {
    usb = { ok: false, reason: LATER };
  }
  return { usb, link: { ok: false, reason: LATER } };
}
