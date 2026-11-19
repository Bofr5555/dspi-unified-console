// WP WB4a: the USB e2e page. The standalone console over a WebUSB mock
// (webusb-mock.ts): `?usb=A,B` are the DSPis plugged in, `?permitted=B` those
// this page may already open. The page's mock answers the picker and the list;
// each session's Worker (usb-worker.ts) has its own mock of the same devices,
// whose transfers reach the core's simulated DSPi. `usbMock` lets a test pick
// in the picker, unplug a device, or change it at the device; the Workers hear
// of it over a BroadcastChannel.

import { CoreClient, LocalBackend, mount } from "../../src/index";
import { MOCK_CHANNEL, MockUsb, installMockUsb, type MockMessage } from "./webusb-mock";
import type { UsbSetup } from "../../src/core/usb-ids"; // -- WP PGWEB --

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
  // -- WP PGWEB -- every control request the opened device `serial` saw, from the Worker that has it
  log(serial: string): Promise<UsbSetup[]> {
    const ask = Math.random();
    return new Promise((resolve) => {
      const done = (log: UsbSetup[]) => {
        channel.removeEventListener("message", on);
        resolve(log);
      };
      const on = (ev: MessageEvent<MockMessage>) => {
        if ("logReply" in ev.data && ev.data.logReply === ask) done(ev.data.log);
      };
      channel.addEventListener("message", on);
      tell({ logOf: serial, ask });
      setTimeout(() => done([]), 2000);
    });
  },
};

// -- WP PGWEB -- `?gen=SERIAL:kind,...` reaches the Workers' mocks
const gen = q.get("gen") ?? "";
const worker = new URL(`./usb-worker.js?usb=${encodeURIComponent(serials.join(","))}${gen ? `&gen=${encodeURIComponent(gen)}` : ""}`, location.href);
// -- WP WB7 -- served by `uconsole web`, the hub on this machine is listed too
const servingHub = document.querySelector<HTMLMetaElement>('meta[name="dspi-hub"]')?.content || null;
mount(document.getElementById("console")!, {
  backend: new LocalBackend({ storage: null, servingHub }),
  spawnCore: () => CoreClient.spawn(worker),
  title: "USB test",
});
