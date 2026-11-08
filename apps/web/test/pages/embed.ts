// WP WB5: the embedding test page. Two consoles on one page, each with a
// server-like backend: a signed-in user, installations the server manages (no
// add/remove), and per-installation access. This is the shape a fleet server
// takes; the tests check the console honours it.

import { CoreClient, mount, VIEW_ONLY_REASON, type Access, type FleetBackend, type Installation } from "../../src/index";

class FakeFleet implements FleetBackend {
  readonly kind = "fake-fleet";
  constructor(
    private readonly name: string,
    private readonly roles: Record<string, Access>,
  ) {}
  async installations(): Promise<Installation[]> {
    return [
      { id: "venue-a", name: "Venue A", sources: [{ kind: "demo" }], editable: false },
      { id: "venue-b", name: "Venue B", sources: [{ kind: "demo" }], editable: false },
    ];
  }
  async access(installationId: string): Promise<Access> {
    return this.roles[installationId] ?? "view";
  }
  async user() {
    return { id: "u1", name: this.name };
  }
}

// `?core=stub`: the tests' fake core (stub-worker.js), for what the real core's
// demo cannot do yet (Link phases). Otherwise the page's own Worker, the real core.
const stub = new URLSearchParams(location.search).get("core") === "stub";
const worker = new URL(stub ? "./stub-worker.js" : "../worker.js", location.href);
// Every session's client, so the tests can call the core directly.
const clients: CoreClient[] = [];
(globalThis as unknown as { dspiClients: CoreClient[] }).dspiClients = clients;
const spawnCore = () => {
  const c = CoreClient.spawn(worker);
  clients.push(c);
  return c;
};
mount(document.getElementById("one")!, {
  backend: new FakeFleet("Operator One", { "venue-a": "control", "venue-b": "view" }),
  spawnCore,
  title: "Fleet console one",
});
mount(document.getElementById("two")!, {
  backend: new FakeFleet("Operator Two", { "venue-a": "control" }),
  spawnCore,
  title: "Fleet console two",
});
(globalThis as unknown as { VIEW_ONLY_REASON: string }).VIEW_ONLY_REASON = VIEW_ONLY_REASON;
