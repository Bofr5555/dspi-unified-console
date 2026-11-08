// WP WB4a: a WebUSB mock (`navigator.usb`, `USBDevice`) for the tests, in Node
// and in headless Chromium. The devices answer as a DSPi would: behind each is
// the core's simulated DSPi (`FakeDevice::demo`), run in its own instance of the
// binding's test module (dspi_wasm_test.wasm, crates/dspi-wasm/src/usb.rs
// `mock`), not the instance that holds the sessions. The session under test is
// a USB session on the page's own module: the core never opens its demo.
//
// What is checked as a browser would: the device must be opened and interface 2
// claimed before a transfer; requests are vendor/interface with wIndex 2; an
// unplugged device rejects with NotFoundError and fires `disconnect`; the
// notification endpoint (3) answers a packet as it arrives, or the idle
// keep-alive after 100 ms of quiet.

import { USB_IDS, isDspi, type UsbConnectionEventLike, type UsbDeviceLike, type UsbInResult, type UsbLike, type UsbSetup } from "../../src/core/usb-ids";

interface MockExports {
  memory: WebAssembly.Memory;
  dspi_alloc(len: number): number;
  dspi_free(ptr: number, len: number): void;
  dspi_mock_new(): number;
  dspi_mock_in(id: number, op: number, value: number, len: number): number;
  dspi_mock_out(id: number, op: number, value: number, ptr: number, len: number): number;
  dspi_mock_notify(id: number): number;
  dspi_mock_rename(id: number, ptr: number, len: number): number;
  dspi_mock_unplug(id: number): void;
  dspi_mock_buf_ptr(): number;
}

const GONE = -1;
const STALL = -2;

/** The simulated DSPis' engine: one instance of the test module. */
export class MockBackend {
  private constructor(private readonly x: MockExports) {}

  static async load(bytes: BufferSource): Promise<MockBackend> {
    const never = () => {
      throw new Error("the mock device does not wait");
    };
    const imports = {
      dspi: {
        dspi_now_ms: () => performance.now(),
        dspi_unix_ms: () => Date.now(),
        dspi_sleep_ms: never,
        dspi_random_fill: () => 0,
        dspi_usb_in: never,
        dspi_usb_out: never,
        dspi_usb_error: () => 0,
      },
    };
    const { instance } = await WebAssembly.instantiate(bytes, imports as unknown as WebAssembly.Imports);
    return new MockBackend(instance.exports as unknown as MockExports);
  }

  newDevice(): number {
    return this.x.dspi_mock_new();
  }

  in(id: number, op: number, value: number, len: number): Uint8Array | number {
    const rc = this.x.dspi_mock_in(id, op, value, len);
    return rc < 0 ? rc : this.buf(rc);
  }

  out(id: number, op: number, value: number, data: Uint8Array): number {
    return this.withBytes(data, (ptr) => this.x.dspi_mock_out(id, op, value, ptr, data.length));
  }

  notify(id: number): Uint8Array | number {
    const rc = this.x.dspi_mock_notify(id);
    return rc <= 0 ? rc : this.buf(rc);
  }

  /** Another host renames the device: a notification follows. */
  renameElsewhere(id: number, name: string): number {
    const b = new TextEncoder().encode(name);
    return this.withBytes(b, (ptr) => this.x.dspi_mock_rename(id, ptr, b.length));
  }

  unplug(id: number): void {
    this.x.dspi_mock_unplug(id);
  }

  private buf(n: number): Uint8Array {
    return new Uint8Array(this.x.memory.buffer, this.x.dspi_mock_buf_ptr(), n).slice();
  }

  private withBytes(data: Uint8Array, f: (ptr: number) => number): number {
    const len = Math.max(1, data.length);
    const ptr = this.x.dspi_alloc(len);
    new Uint8Array(this.x.memory.buffer, ptr, data.length).set(data);
    try {
      return f(ptr);
    } finally {
      this.x.dspi_free(ptr, len);
    }
  }
}

const dom = (name: string, message: string) => new DOMException(message, name);
const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));

export class MockDevice implements UsbDeviceLike {
  readonly vendorId = USB_IDS.vendorId;
  readonly productId = USB_IDS.productId;
  readonly productName = "DSPi";
  opened = false;
  configuration: { configurationValue: number } | null = null;
  gone = false;
  private readonly claimed = new Set<number>();
  /** Every control request as it arrived, for the tests. */
  readonly log: UsbSetup[] = [];

  constructor(
    readonly serialNumber: string,
    private readonly backend: MockBackend | null,
    private readonly id: number,
  ) {}

  async open() {
    this.alive();
    this.opened = true;
  }
  async close() {
    this.opened = false;
    this.claimed.clear();
  }
  async selectConfiguration(v: number) {
    this.alive();
    if (!this.opened) throw dom("InvalidStateError", "The device must be opened first.");
    this.configuration = { configurationValue: v };
  }
  async claimInterface(n: number) {
    this.alive();
    if (!this.opened || !this.configuration) throw dom("InvalidStateError", "The device must be opened first.");
    if (n !== USB_IDS.interface) throw dom("SecurityError", "The requested interface implements a protected class.");
    this.claimed.add(n);
  }
  async releaseInterface(n: number) {
    this.claimed.delete(n);
  }
  async clearHalt() {}

  async controlTransferIn(setup: UsbSetup, length: number): Promise<UsbInResult> {
    const b = this.ready(setup);
    const r = b.in(this.id, setup.request, setup.value, length);
    if (r === GONE) throw dom("NotFoundError", "The device was disconnected.");
    if (typeof r === "number") return { status: "stall" };
    return { status: "ok", data: new DataView(r.buffer, r.byteOffset, r.byteLength) };
  }

  async controlTransferOut(setup: UsbSetup, data: Uint8Array): Promise<{ status: "ok" | "stall"; bytesWritten: number }> {
    const b = this.ready(setup);
    const rc = b.out(this.id, setup.request, setup.value, data);
    if (rc === GONE) throw dom("NotFoundError", "The device was disconnected.");
    return rc === STALL || rc < 0 ? { status: "stall", bytesWritten: 0 } : { status: "ok", bytesWritten: data.length };
  }

  /** The notification endpoint: a packet as soon as one is queued, the idle keep-alive after 100 ms. */
  async transferIn(endpoint: number, _length: number): Promise<UsbInResult> {
    if (endpoint !== USB_IDS.notifyEndpoint || !this.claimed.has(USB_IDS.interface)) {
      throw dom("NotFoundError", "The specified endpoint is not part of a claimed and selected alternate interface.");
    }
    const b = this.backend!;
    const until = performance.now() + 100;
    for (;;) {
      this.alive();
      if (!this.opened) throw dom("AbortError", "The transfer was cancelled.");
      const r = b.notify(this.id);
      if (r === GONE) throw dom("NotFoundError", "The device was disconnected.");
      if (typeof r !== "number") return { status: "ok", data: new DataView(r.buffer, r.byteOffset, r.byteLength) };
      if (performance.now() >= until) return { status: "ok", data: new DataView(new Uint8Array([0]).buffer) };
      await pause(5);
    }
  }

  renameElsewhere(name: string): void {
    this.backend?.renameElsewhere(this.id, name);
  }

  unplug(): void {
    this.gone = true;
    this.backend?.unplug(this.id);
  }

  private alive(): void {
    if (this.gone) throw dom("NotFoundError", "The device was disconnected.");
  }

  private ready(setup: UsbSetup): MockBackend {
    this.alive();
    if (!this.opened || !this.claimed.has(USB_IDS.interface)) throw dom("InvalidStateError", "The interface is not claimed.");
    if (setup.requestType !== "vendor" || setup.recipient !== "interface" || setup.index !== USB_IDS.interface) {
      throw dom("SecurityError", `unexpected request ${JSON.stringify(setup)}`);
    }
    this.log.push(setup);
    if (!this.backend) throw dom("NetworkError", "this mock device has no engine");
    return this.backend;
  }
}

/**
 * `navigator.usb`. `serials` are the DSPis plugged in; `permitted` those the page
 * may already open (all of them in a Worker, where the page's grant applies).
 * `choose` answers the picker: the serial the user picks, or null to cancel.
 */
export class MockUsb implements UsbLike {
  readonly devices: MockDevice[] = [];
  readonly permitted = new Set<string>();
  choose: (offered: string[]) => string | null = (offered) => offered[0] ?? null;
  private readonly listeners = { connect: new Set<(e: UsbConnectionEventLike) => void>(), disconnect: new Set<(e: UsbConnectionEventLike) => void>() };

  constructor(
    serials: string[],
    opts: { backend?: MockBackend | null; permitted?: string[] | "all" } = {},
  ) {
    for (const s of serials) {
      const b = opts.backend ?? null;
      this.devices.push(new MockDevice(s, b, b ? b.newDevice() : -1));
    }
    for (const s of opts.permitted === "all" ? serials : (opts.permitted ?? [])) this.permitted.add(s);
  }

  async getDevices(): Promise<UsbDeviceLike[]> {
    return this.devices.filter((d) => !d.gone && this.permitted.has(d.serialNumber));
  }

  async requestDevice(o: { filters: { vendorId: number; productId: number }[] }): Promise<UsbDeviceLike> {
    const offered = this.devices.filter((d) => !d.gone && isDspi(d) && o.filters.some((f) => f.vendorId === d.vendorId && f.productId === d.productId));
    const pick = this.choose(offered.map((d) => d.serialNumber));
    const d = offered.find((x) => x.serialNumber === pick);
    if (!d) throw dom("NotFoundError", "No device selected.");
    this.permitted.add(d.serialNumber);
    return d;
  }

  addEventListener(type: "connect" | "disconnect", l: (e: UsbConnectionEventLike) => void): void {
    this.listeners[type].add(l);
  }
  removeEventListener(type: "connect" | "disconnect", l: (e: UsbConnectionEventLike) => void): void {
    this.listeners[type].delete(l);
  }

  device(serial: string): MockDevice | undefined {
    return this.devices.find((d) => d.serialNumber === serial);
  }

  unplug(serial: string): void {
    const d = this.device(serial);
    if (!d || d.gone) return;
    d.unplug();
    for (const l of this.listeners.disconnect) l({ device: d });
  }
}

/** Puts `usb` in this context's `navigator.usb` (a page's or a Worker's). */
export function installMockUsb(usb: MockUsb): void {
  Object.defineProperty(globalThis.navigator, "usb", { value: usb, configurable: true });
}

/** What the page and its Workers tell each other in the e2e (unplug, a change at the device). */
export const MOCK_CHANNEL = "dspi-webusb-mock";
export type MockMessage = { unplug: string } | { rename: string; name: string };
