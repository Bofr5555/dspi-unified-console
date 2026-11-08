// What this page can reach (WP WB5). Each answer carries the reason, in words a
// user can act on, for the picker to show beside a source it cannot offer.
// No text names the device the user is holding (owner's rule).

import { DSPI_FILTERS, isDspi, webUsb, type UsbLike } from "../core/usb-ids";

export interface Availability {
  ok: boolean;
  reason: string;
}

export interface PageEnv {
  usb: Availability;
  /** Link to a hub: needs WB4b's transport. */
  link: Availability;
  // -- WP WB4a --
  /** The page's side of WebUSB, when `usb.ok`: the browser's picker and the DSPis it already allows. */
  usbDevices?: UsbAccess;
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
    // -- WP WB4a --
    const api = webUsb(g);
    return { usb: { ok: true, reason: "" }, link: { ok: false, reason: LATER }, usbDevices: api ? usbAccess(api) : undefined };
  }
  return { usb, link: { ok: false, reason: LATER } };
}

// -- WP WB4a --
/** A DSPi the page may open: what the session's Worker finds it by. */
export interface UsbChoice {
  serial: string;
  vendorId: number;
}

export interface UsbAccess {
  /** The browser's picker (needs a click). Null when the user chose nothing. */
  choose(): Promise<UsbChoice | null>;
  /** The DSPis this page is already allowed to open and that are plugged in, by serial. */
  permitted(): Promise<UsbChoice[]>;
  /** Calls `fn` when a DSPi is plugged in or out; returns the way to stop. */
  watch(fn: () => void): () => void;
}

export function usbAccess(usb: UsbLike): UsbAccess {
  return {
    async choose() {
      try {
        const d = await usb.requestDevice({ filters: DSPI_FILTERS });
        // A DSPi without a serial cannot be found again by the Worker (nor told
        // apart from another): the other consoles skip such a device too.
        return d.serialNumber ? { serial: d.serialNumber, vendorId: d.vendorId } : null;
      } catch (e) {
        // The user closed the picker without choosing.
        if ((e as { name?: string } | null)?.name === "NotFoundError") return null;
        throw e;
      }
    },
    async permitted() {
      const all = await usb.getDevices();
      return all
        .filter((d) => isDspi(d) && !!d.serialNumber)
        .map((d) => ({ serial: d.serialNumber!, vendorId: d.vendorId }))
        .sort((a, b) => (a.serial < b.serial ? -1 : a.serial > b.serial ? 1 : 0));
    },
    watch(fn) {
      const l = (e: { device: { vendorId: number; productId: number } }) => {
        if (isDspi(e.device)) fn();
      };
      usb.addEventListener("connect", l);
      usb.addEventListener("disconnect", l);
      return () => {
        usb.removeEventListener("connect", l);
        usb.removeEventListener("disconnect", l);
      };
    },
  };
}
