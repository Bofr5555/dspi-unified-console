// WP SECWEB unit tests (PLAN-SEC 4.6): the scheme from the page (never `ws://` from an https
// page), the `uconsole web` proxy's route, the hub key's display form (the hub's own), and each
// link's security: the browser's TLS, the proxy's pinned TLS, a loopback hub, an unencrypted one.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  fingerprint,
  forPage,
  httpsConsoleOf,
  routeLink,
  securityOf,
  type Fetcher,
  type LinkContext,
  type PageLocation,
} from "../../src/fleet/security";
import { LocalBackend, type KeyValueStore } from "../../src/fleet/local";
import { isUnsecured } from "../../src/ui/LinkSecurity";

const https: PageLocation = { protocol: "https:", host: "hub.local:11915", hostname: "hub.local" };
const local: PageLocation = { protocol: "http:", host: "localhost:11916", hostname: "localhost" };
const lanHttp: PageLocation = { protocol: "http:", host: "192.168.1.5:11915", hostname: "192.168.1.5" };

/** A fetcher answering from a table; records what was asked. */
function fetcherOf(table: Record<string, unknown>): Fetcher & { asked: string[] } {
  const asked: string[] = [];
  const f = (async (url: string) => {
    asked.push(url);
    return url in table ? { ok: true, json: async () => table[url] } : { ok: false, json: async () => null };
  }) as Fetcher & { asked: string[] };
  f.asked = asked;
  return f;
}

function memory(): KeyValueStore {
  const m = new Map<string, string>();
  return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => void m.set(k, v) };
}

test("SECWEB: an https page opens wss:// only, the same hub", () => {
  assert.equal(forPage("ws://hub.local:11915", https), "wss://hub.local:11915");
  assert.equal(forPage("wss://fleet.example/hubs/7/dspi/v1", https), "wss://fleet.example/hubs/7/dspi/v1");
  assert.equal(forPage("ws://10.0.0.2:11915", lanHttp), "ws://10.0.0.2:11915");
  assert.equal(forPage("ws://10.0.0.2:11915", null), "ws://10.0.0.2:11915");
  const ctx: LinkContext = { loc: https, proxy: false };
  for (const url of ["ws://hub.local:11915", "ws://10.0.0.2:11915", "ws://[fd00::2]:11915"]) {
    assert.match(routeLink(url, ctx), /^wss:\/\//, url);
  }
});

test("SECWEB: the hub list of an https page is wss:// only (no mixed content is ever attempted)", async () => {
  const storage = memory();
  storage.setItem("dspi-web.hubs.v1", JSON.stringify([{ url: "ws://10.0.0.2:11915", name: "Rack" }]));
  const backend = new LocalBackend({ storage, servingHub: "ws://hub.local:11915", location: https });
  const [own] = await backend.installations();
  const urls = own!.sources.flatMap((s) => (s.kind === "hub" ? [s.url] : []));
  assert.deepEqual(urls, ["wss://hub.local:11915", "wss://10.0.0.2:11915"]);
  assert.ok(urls.every((u) => !u.startsWith("ws://")));
  // On an http page they stay as they are.
  const plain = new LocalBackend({ storage, servingHub: null, location: lanHttp });
  const [p] = await plain.installations();
  assert.deepEqual(
    p!.sources.flatMap((s) => (s.kind === "hub" ? [s.url] : [])),
    ["ws://10.0.0.2:11915"],
  );
});

test("SECWEB: a uconsole web page routes other hubs through its pinning proxy, never its own", () => {
  const ctx: LinkContext = { loc: local, proxy: true };
  assert.equal(routeLink("ws://10.0.0.2:11915", ctx), "ws://localhost:11916/hub/10.0.0.2%3A11915/dspi/v1");
  assert.equal(routeLink("ws://hub.local", ctx), "ws://localhost:11916/hub/hub.local%3A11915/dspi/v1");
  assert.equal(routeLink("ws://[fd00::2]:11915", ctx), "ws://localhost:11916/hub/%5Bfd00%3A%3A2%5D%3A11915/dspi/v1");
  // The page's own hub, by any loopback name, and an address already routed: as they are.
  assert.equal(routeLink("ws://127.0.0.1:11916", ctx), "ws://127.0.0.1:11916");
  assert.equal(routeLink("ws://localhost:11916", ctx), "ws://localhost:11916");
  const routed = routeLink("ws://10.0.0.2:11915", ctx);
  assert.equal(routeLink(routed, ctx), routed);
  // Without the proxy (an older hub), unchanged.
  assert.equal(routeLink("ws://10.0.0.2:11915", { loc: local, proxy: false }), "ws://10.0.0.2:11915");
});

test("SECWEB: the fingerprint is the hub's display form (the vector dspi-hub's proxy test checks too)", () => {
  assert.equal(fingerprint("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"), "SHA256:000G-40R4-0M30-E209");
  assert.equal(fingerprint("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="), "SHA256:000G-40R4-0M30-E209");
  assert.equal(fingerprint("short"), "");
  assert.equal(fingerprint("!!!"), "");
});

test("SECWEB: the security of each kind of link", async () => {
  // The hub's own https page: encrypted by the browser, the key from its /info.
  const f1 = fetcherOf({ "/dspi/v1/info": { tls: true, tls_spki: "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8", upstream_compat: false } });
  const s1 = await securityOf("wss://hub.local:11915/dspi/v1", { loc: https, proxy: false }, f1);
  assert.deepEqual([s1.state, s1.via, s1.hubKey, s1.upstreamCompat], ["encrypted", "browser", "SHA256:000G-40R4-0M30-E209", false]);
  assert.equal(isUnsecured(s1), false);
  // A fleet server's wss://: encrypted, no key of the hub's to show.
  const s2 = await securityOf("wss://fleet.example/hubs/7/dspi/v1", { loc: { ...https, host: "app.example", hostname: "app.example" }, proxy: false }, fetcherOf({}));
  assert.deepEqual([s2.state, s2.hubKey], ["encrypted", ""]);
  // The proxy, pinned: its own answer.
  const proxied = "ws://localhost:11916/hub/10.0.0.2%3A11915/dspi/v1";
  const f3 = fetcherOf({ "/hub/10.0.0.2%3A11915/dspi/v1/security": { encrypted: true, hubKey: "SHA256:AAAA-BBBB-CCCC-DDDD", tls: true, upstreamCompat: false, refused: "" } });
  const s3 = await securityOf(proxied, { loc: local, proxy: true }, f3);
  assert.deepEqual([s3.state, s3.via, s3.hubKey], ["encrypted", "proxy", "SHA256:AAAA-BBBB-CCCC-DDDD"]);
  // The proxy to an upstream hub (no TLS): unsecured, the red bar.
  const f4 = fetcherOf({ "/hub/10.0.0.2%3A11915/dspi/v1/security": { encrypted: false, hubKey: "", tls: false, upstreamCompat: false, refused: "" } });
  const s4 = await securityOf(proxied, { loc: local, proxy: true }, f4);
  assert.equal(isUnsecured(s4), true);
  // The proxy's refusal, in its words.
  const f5 = fetcherOf({ "/hub/10.0.0.2%3A11915/dspi/v1/security": { encrypted: false, refused: "hub_key_changed", message: "a different key" } });
  const s5 = await securityOf(proxied, { loc: local, proxy: true }, f5);
  assert.deepEqual([s5.refused, s5.message], ["hub_key_changed", "a different key"]);
  // A hub on this machine: local, never red.
  const s6 = await securityOf("ws://127.0.0.1:11916", { loc: local, proxy: true }, fetcherOf({}));
  assert.deepEqual([s6.state, isUnsecured(s6)], ["local", false]);
  // Plain ws:// to a LAN hub that served this page in `on` mode: unsecured, its https page offered.
  const ctx7: LinkContext = { loc: lanHttp, proxy: false };
  const s7 = await securityOf("ws://192.168.1.5:11915/dspi/v1", ctx7, fetcherOf({ "/dspi/v1/info": { tls: true, upstream_compat: true } }));
  assert.deepEqual([s7.state, s7.tlsServed, s7.upstreamCompat], ["unsecured", true, true]);
  assert.equal(httpsConsoleOf(ctx7, s7), "https://192.168.1.5:11915/console/");
  // No /info reachable (an upstream hub): unsecured, nothing else known.
  const s8 = await securityOf("ws://10.0.0.9:11915/dspi/v1", ctx7, fetcherOf({}));
  assert.deepEqual([s8.state, s8.tlsServed, s8.upstreamCompat], ["unsecured", null, null]);
  assert.equal(httpsConsoleOf(ctx7, s8), "");
});
