// WP WB4a: the Worker's side of the USB e2e page (usb.ts): `navigator.usb` is a
// WebUSB mock of the page's DSPis (`?usb=`), each the core's simulated DSPi in
// this Worker's own instance of the test module (dspi_wasm_test.wasm). A
// Worker sees every plugged DSPi, as the page's grant covers it. Imported
// before the Worker itself (usb-worker.ts).

import type { UsbLike } from "../../src/core/usb-ids";
import { MOCK_CHANNEL, MockBackend, MockUsb, type MockMessage } from "./webusb-mock";

const q = new URLSearchParams(location.search);
const serials = (q.get("usb") ?? "").split(",").filter(Boolean);
// -- WP PGWEB -- `?gen=SERIAL:kind,...`: each DSPi's firmware generation or board (webusb-mock.ts MOCK_KINDS)
const kinds = Object.fromEntries((q.get("gen") ?? "").split(",").filter(Boolean).map((x) => x.split(":") as [string, string]));
const mock: Promise<MockUsb> = fetch(new URL("./dspi_wasm_test.wasm", location.href))
  .then((r) => r.arrayBuffer())
  .then((b) => MockBackend.load(b))
  .then((backend) => new MockUsb(serials, { backend, permitted: "all", kinds }));

const lazy: UsbLike = {
  getDevices: async () => (await mock).getDevices(),
  requestDevice: async (o) => (await mock).requestDevice(o),
  addEventListener: (t, l) => void mock.then((m) => m.addEventListener(t, l)),
  removeEventListener: (t, l) => void mock.then((m) => m.removeEventListener(t, l)),
};
Object.defineProperty(globalThis.navigator, "usb", { value: lazy, configurable: true });

// Held for the Worker's life: an unreferenced channel may be collected.
const channel = new BroadcastChannel(MOCK_CHANNEL);
(globalThis as unknown as { usbMockChannel: BroadcastChannel }).usbMockChannel = channel;
channel.onmessage = async (ev: MessageEvent<MockMessage>) => {
  const m = await mock;
  const d = ev.data;
  if ("unplug" in d) m.unplug(d.unplug);
  // -- WP PGWEB -- only the Worker whose device has been opened answers with its log
  else if ("logOf" in d) {
    const dev = m.device(d.logOf);
    if (dev?.log.length) channel.postMessage({ logReply: d.ask, log: dev.log } satisfies MockMessage);
  } else if ("rename" in d) m.device(d.rename)?.renameElsewhere(d.name);
};
