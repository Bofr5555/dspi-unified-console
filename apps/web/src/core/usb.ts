// WebUSB in the core's Worker (WP WB4a). Chromium only, secure pages only
// (fleet/env.ts says why when not).
//
// The page chooses a DSPi with the browser's own picker and hands its serial to
// the session's Worker; permission is the origin's, so the Worker finds the same
// device in its own `navigator.usb.getDevices()`. Here it is opened, vendor
// interface 2 claimed, and given a number the core's transport names it by
// (crates/dspi-wasm/src/usb.rs). Each Worker is one session; several Workers,
// several devices, each claimed by its own.
//
// - Control transfers are the module's imports `dspi_usb_in` / `dspi_usb_out`,
//   `WebAssembly.Suspending`: the core blocks, the browser transfers. They do the
//   transfer and nothing else, never a call back into the module.
// - Notifications: this file reads the notification endpoint and hands the
//   packets to WasmCore, which pushes them in (`usbPush`) before its next `poll`.
// - Unplug: the browser's `disconnect` event, or a transfer that finds the device
//   gone, calls the session's `onGone` once.

import { CoreCallError } from "./protocol";
import { USB_IDS, isDspi, webUsb, type UsbDeviceLike, type UsbLike, type UsbSetup } from "./usb-ids";

/** The imports' answers when a transfer did not complete (usb.rs `code`). */
const GONE = -1;
const STALL = -2;
const TIMEOUT = -3;
const OTHER = -4;

/** A control transfer's limit, as the core's own USB transport (`DEFAULT_TIMEOUT`). */
export const TRANSFER_TIMEOUT_MS = 1000;

/** Shorter than a header: the firmware's idle keep-alive, which the core drops anyway. */
const NOTIFY_HEADER = 4;

export const USB_NEEDS_CHROMIUM = "USB needs a Chromium-based browser, such as Chrome or Edge";

interface Claimed {
  device: UsbDeviceLike;
  serial: string;
  gone: boolean;
  reading: boolean;
  lastError: string;
  onGone: () => void;
}

/** What `claim` hands the core's `open`. */
export interface ClaimedDevice {
  device: number;
  serial: string;
  vendorId: number;
}

type Suspending = new (f: (...a: number[]) => Promise<number>) => unknown;

export class UsbHost {
  private readonly claimed = new Map<number, Claimed>();
  private next = 1;
  private memory: () => WebAssembly.Memory = () => {
    throw new Error("the core's memory is not bound yet");
  };
  private listening: UsbLike | null = null;

  /** `usb`: this context's WebUSB, looked up when a device is claimed. */
  constructor(private readonly usb: () => UsbLike | undefined = () => webUsb()) {}

  bindMemory(memory: () => WebAssembly.Memory): void {
    this.memory = memory;
  }

  /** The module's three USB imports (module `dspi`). */
  imports(S: Suspending): Record<string, unknown> {
    return {
      dspi_usb_in: new S((id, request, value, len, out) => this.controlIn(id, request, value, len, out)),
      dspi_usb_out: new S((id, request, value, ptr, len) => this.controlOut(id, request, value, ptr, len)),
      dspi_usb_error: (id: number, out: number, cap: number) => {
        const bytes = new TextEncoder().encode(this.claimed.get(id)?.lastError ?? "the DSPi is not open");
        const n = Math.min(bytes.length, cap);
        new Uint8Array(this.memory().buffer, out, n).set(bytes.subarray(0, n));
        return n;
      },
    };
  }

  /** Find the DSPi with this serial, open it and claim interface 2. `onGone` runs once when it leaves. */
  async claim(serial: string, onGone: () => void): Promise<ClaimedDevice> {
    const usb = this.usb();
    if (!usb) throw new CoreCallError("unsupported", USB_NEEDS_CHROMIUM);
    this.listen(usb);
    const device = (await usb.getDevices()).find((d) => isDspi(d) && d.serialNumber === serial);
    // The core's own words for it (TransportError::SerialNotFound).
    if (!device) throw new CoreCallError("not_open", `No DSPi with serial ${serial}`);
    try {
      if (!device.opened) await device.open();
    } catch (e) {
      throw new CoreCallError(
        "transfer",
        `The DSPi could not be opened (${words(e)}). On Linux the DSPi udev rule must be installed; elsewhere, close any other program using the DSPi`,
      );
    }
    try {
      if (device.configuration == null) await device.selectConfiguration(1);
      await device.claimInterface(USB_IDS.interface);
    } catch (e) {
      await device.close().catch(() => undefined);
      throw new CoreCallError(
        "transfer",
        `The DSPi is in use by another program or another tab: close it there and try again (${words(e)})`,
      );
    }
    const id = this.next++;
    this.claimed.set(id, { device, serial, gone: false, reading: false, lastError: "", onGone });
    return { device: id, serial, vendorId: device.vendorId };
  }

  /** Stop reading, give the interface back and close the device. */
  async release(id: number): Promise<void> {
    const c = this.claimed.get(id);
    if (!c) return;
    this.claimed.delete(id);
    c.reading = false;
    if (c.gone) return;
    await c.device.releaseInterface(USB_IDS.interface).catch(() => undefined);
    await c.device.close().catch(() => undefined);
  }

  /** Read the notification endpoint until released or gone; each packet to `onPacket`. */
  readNotifications(id: number, onPacket: (p: number[]) => void): void {
    const c = this.claimed.get(id);
    if (!c || c.reading) return;
    c.reading = true;
    void (async () => {
      while (c.reading && !c.gone) {
        try {
          const r = await c.device.transferIn(USB_IDS.notifyEndpoint, USB_IDS.notifyPacket);
          if (!c.reading) break;
          if (r.status === "stall") {
            await c.device.clearHalt("in", USB_IDS.notifyEndpoint).catch(() => undefined);
            await pause(50);
          } else if (r.status === "ok" && r.data && r.data.byteLength >= NOTIFY_HEADER) {
            onPacket([...new Uint8Array(r.data.buffer, r.data.byteOffset, r.data.byteLength)]);
          }
        } catch (e) {
          if (isGone(e)) this.lose(c);
          else if (c.reading) await pause(50);
        }
      }
    })();
  }

  private listen(usb: UsbLike): void {
    if (this.listening === usb) return;
    this.listening = usb;
    usb.addEventListener("disconnect", (ev) => {
      for (const c of this.claimed.values()) {
        if (ev.device === c.device || (ev.device.serialNumber === c.serial && isDspi(ev.device))) this.lose(c);
      }
    });
  }

  private lose(c: Claimed): void {
    if (c.gone) return;
    c.gone = true;
    c.reading = false;
    c.onGone();
  }

  private async controlIn(id: number, request: number, value: number, len: number, out: number): Promise<number> {
    const c = this.claimed.get(id);
    if (!c || c.gone) return GONE;
    try {
      const r = await withTimeout(c.device.controlTransferIn(setup(request, value), len));
      if (r === null) return TIMEOUT;
      if (r.status === "stall") return STALL;
      if (r.status !== "ok" || !r.data) {
        c.lastError = `the transfer ended in ${r.status}`;
        return OTHER;
      }
      const n = Math.min(r.data.byteLength, len);
      new Uint8Array(this.memory().buffer, out, n).set(new Uint8Array(r.data.buffer, r.data.byteOffset, n));
      return n;
    } catch (e) {
      return this.failed(c, e);
    }
  }

  private async controlOut(id: number, request: number, value: number, ptr: number, len: number): Promise<number> {
    const c = this.claimed.get(id);
    if (!c || c.gone) return GONE;
    // A copy before the first await: the module's memory may move meanwhile.
    const data = new Uint8Array(this.memory().buffer, ptr, len).slice();
    try {
      const r = await withTimeout(c.device.controlTransferOut(setup(request, value), data));
      if (r === null) return TIMEOUT;
      return r.status === "stall" ? STALL : 0;
    } catch (e) {
      return this.failed(c, e);
    }
  }

  private failed(c: Claimed, e: unknown): number {
    if (isGone(e)) {
      this.lose(c);
      return GONE;
    }
    c.lastError = words(e);
    return OTHER;
  }
}

/** Vendor request to interface 2; `wIndex` is never anything else (the protocol's rule). */
function setup(request: number, value: number): UsbSetup {
  return { requestType: "vendor", recipient: "interface", request, value, index: USB_IDS.interface };
}

/**
 * WebUSB has no transfer timeout, so the core's is kept here: null after
 * TRANSFER_TIMEOUT_MS. The browser's transfer may still finish later; the core
 * treats the timeout as its own transport would (a busy window, then an error).
 */
async function withTimeout<T>(p: Promise<T>): Promise<T | null> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([p, new Promise<null>((r) => (timer = setTimeout(() => r(null), TRANSFER_TIMEOUT_MS)))]);
  } finally {
    clearTimeout(timer);
  }
}

/** Chromium rejects a transfer to an unplugged device with NotFoundError. */
function isGone(e: unknown): boolean {
  return (e as { name?: string } | null)?.name === "NotFoundError";
}

function words(e: unknown): string {
  return e instanceof Error ? e.message || e.name : String(e);
}

const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));
