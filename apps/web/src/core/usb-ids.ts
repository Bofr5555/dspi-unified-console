// What a DSPi is on WebUSB (WP WB4a), for the page's picker and the Worker's
// claim. Copies of dspi-proto's constants: the binding answers its own
// (`usbIds`, crates/dspi-wasm/src/usb.rs) and a unit test holds these to them.

export const USB_IDS = {
  vendorId: 0x2e8b,
  productId: 0xfeaa,
  /** Older firmware enumerated with the Raspberry Pi vendor ID; the core still reads it. */
  legacyVendorId: 0x2e8a,
  /** The vendor interface every transfer goes through. */
  interface: 2,
  /** The notification endpoint (`VENDOR_EP_IN` 0x83), as WebUSB numbers it. */
  notifyEndpoint: 3,
  notifyPacket: 64,
} as const;

/** `navigator.usb.requestDevice`'s filters: current and legacy firmware. */
export const DSPI_FILTERS = [
  { vendorId: USB_IDS.vendorId, productId: USB_IDS.productId },
  { vendorId: USB_IDS.legacyVendorId, productId: USB_IDS.productId },
];

export function isDspi(d: { vendorId: number; productId: number }): boolean {
  return d.productId === USB_IDS.productId && (d.vendorId === USB_IDS.vendorId || d.vendorId === USB_IDS.legacyVendorId);
}

/** The short form the other consoles show: the serial's last 8 characters. */
export function shortSerial(serial: string): string {
  const chars = [...serial];
  return chars.slice(Math.max(0, chars.length - 8)).join("");
}

/** The part of WebUSB the console uses (`USB`, `USBDevice`), so a mock can stand in. */
export interface UsbSetup {
  requestType: "vendor";
  recipient: "interface";
  request: number;
  value: number;
  index: number;
}

export interface UsbInResult {
  status: "ok" | "stall" | "babble";
  data?: DataView;
}

export interface UsbDeviceLike {
  readonly vendorId: number;
  readonly productId: number;
  readonly serialNumber?: string | null;
  readonly productName?: string | null;
  readonly opened: boolean;
  readonly configuration: unknown;
  open(): Promise<void>;
  close(): Promise<void>;
  selectConfiguration(value: number): Promise<void>;
  claimInterface(n: number): Promise<void>;
  releaseInterface(n: number): Promise<void>;
  controlTransferIn(setup: UsbSetup, length: number): Promise<UsbInResult>;
  controlTransferOut(setup: UsbSetup, data: Uint8Array): Promise<{ status: "ok" | "stall"; bytesWritten: number }>;
  transferIn(endpoint: number, length: number): Promise<UsbInResult>;
  clearHalt(direction: "in" | "out", endpoint: number): Promise<void>;
}

export interface UsbConnectionEventLike {
  readonly device: UsbDeviceLike;
}

export interface UsbLike {
  getDevices(): Promise<UsbDeviceLike[]>;
  requestDevice(options: { filters: { vendorId: number; productId: number }[] }): Promise<UsbDeviceLike>;
  addEventListener(type: "connect" | "disconnect", listener: (e: UsbConnectionEventLike) => void): void;
  removeEventListener(type: "connect" | "disconnect", listener: (e: UsbConnectionEventLike) => void): void;
}

/** This context's WebUSB (a page's or a Worker's), if it has one. */
export function webUsb(g: { navigator?: unknown } = globalThis): UsbLike | undefined {
  const nav = g.navigator as { usb?: UsbLike } | undefined;
  return nav?.usb ?? undefined;
}
