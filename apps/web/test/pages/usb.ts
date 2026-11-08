// WP WB4a: the USB e2e page. The standalone console over a WebUSB mock
// (webusb-mock.ts): `?usb=A,B` are the DSPis plugged in, `?permitted=B` those
// this page may already open. The page's mock answers the picker and the list;
// each session's Worker (usb-worker.ts) has its own mock of the same devices,
// whose transfers reach the core's simulated DSPi. `usbMock` lets a test pick
// in the picker, unplug a device, or change it at the device; the Workers hear
// of it over a BroadcastChannel.

import { CoreClient, LocalBackend, mount } from "../../src/index";
import { MOCK_CHANNEL, MockUsb, installMockUsb, type MockMessage } from "./webusb-mock";

const q = new URLSearchParams(location.search);
const list = (k: string) => (q.get(k) ?? "").split(",").filter(Boolean);
const serials = list("usb");
const usb = new MockUsb(serials, { permitted: list("permitted") });
installMockUsb(usb);
const channel = new BroadcastChannel(MOCK_CHANNEL);
const tell = (m: MockMessage) => channel.postMessage(m);

(globalThis as unknown as { usbMock: unknown }).usbMock = {
  pick(serial: string | null) {
    usb.choose = () => serial;
  },
  unplug(serial: string) {
    usb.unplug(serial);
    tell({ unplug: serial });
  },
  renameElsewhere(serial: string, name: string) {
    tell({ rename: serial, name });
  },
};

const worker = new URL(`./usb-worker.js?usb=${encodeURIComponent(serials.join(","))}`, location.href);
mount(document.getElementById("console")!, {
  backend: new LocalBackend({ storage: null }),
  spawnCore: () => CoreClient.spawn(worker),
  title: "USB test",
});
