// WP WB4b: a fleet server's page with hubs. Its backend names the DSPi on one
// hub itself (`hubDevices`) and holds that hub's token (`hubCredentials`); the
// other hub it leaves to the console, which asks the hub and pairs if need be.
// Query: `a`, `b`: the two hubs' addresses; `token`, `hubId`: hub a's credential.
// A fleet server reaches its hubs through its own origin (a hub accepts only its
// own pages); tools/hub-host.mjs plays that part in the tests.

import { CoreClient, mount, type Access, type FleetBackend, type Installation } from "../../src/index";

const q = new URLSearchParams(location.search);
const a = q.get("a") ?? "";
const b = q.get("b") ?? "";

class HubFleet implements FleetBackend {
  readonly kind = "hub-fleet";
  readonly asked: string[] = [];
  async installations(): Promise<Installation[]> {
    return [
      { id: "site-a", name: "Site A", sources: [{ kind: "hub", url: a }], editable: false },
      { id: "site-b", name: "Site B", sources: [{ kind: "hub", url: b }], editable: false },
    ];
  }
  async access(): Promise<Access> {
    return "control";
  }
  async user() {
    return { id: "ops", name: "Operator" };
  }
  async hubDevices(url: string) {
    return url === a ? [{ deviceId: q.get("device") ?? "", name: "Stage rack" }] : null;
  }
  async hubCredentials(url: string) {
    this.asked.push(url);
    return url === a && q.get("token") ? { token: q.get("token")!, hubId: q.get("hubId") ?? "" } : null;
  }
}

const backend = new HubFleet();
(globalThis as unknown as { fleet: HubFleet }).fleet = backend;
// Every session's client, so the tests can call the core directly.
const clients: CoreClient[] = [];
(globalThis as unknown as { dspiClients: CoreClient[] }).dspiClients = clients;
const worker = new URL("../worker.js", location.href);
mount(document.getElementById("fleet")!, {
  backend,
  title: "Fleet hubs",
  spawnCore: () => {
    const c = CoreClient.spawn(worker);
    clients.push(c);
    return c;
  },
});
