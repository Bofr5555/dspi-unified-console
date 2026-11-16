// WP SECWEB (PLAN-SEC 4.6): how this page reaches a hub, and whether that link is
// encrypted. The browser does the TLS itself (the core in the Worker has none), so the
// answer depends on the page:
//
// - An https page (the hub's own `https://<hub>:11915/console/`, or a fleet server)
//   opens `wss://` only: a `ws://` from it would be mixed content, never attempted
//   (`forPage`). A `wss://` link is encrypted; the serving hub's key fingerprint comes from
//   its `/dspi/v1/info` (`tls_spki`), read over the same TLS the user accepted, to compare
//   with the hub's own display.
// - A page of `uconsole web` (`http://localhost:11916`, its `/dspi/v1/info` says
//   `proxy`) reaches every other hub through the hub process's pinning proxy:
//   `ws://localhost:11916/hub/<host:port>/dspi/v1` (`routeLink`). Whether that relay is
//   encrypted, and the key it pinned, is the proxy's `/security` answer.
// - `ws://` to a loopback hub never leaves this machine: "local", not unsecured.
// - Any other `ws://` is unsecured: the connection bar turns red, " (unsecured)".

/** What a page's location says. */
export interface PageLocation {
  protocol: string;
  host: string;
  hostname: string;
}

export type LinkSecurityState = "encrypted" | "local" | "unsecured";

/** A hub link's security, for the connection bar and the hub's settings. */
export interface LinkSecurity {
  state: LinkSecurityState;
  /** The hub key's fingerprint (`SHA256:K7Q2-M9XD-4TNB-0ZRA`), "" when not known. */
  hubKey: string;
  /** How: the browser's TLS, the `uconsole web` proxy's pinned TLS, or neither. */
  via: "browser" | "proxy" | "none";
  /** The hub serves TLS (its `/dspi/v1/info`); null when not read. */
  tlsServed: boolean | null;
  /** The hub's upstream compatibility mode is on; null when not read. */
  upstreamCompat: boolean | null;
  /** The proxy's refusal code ("" when none) and its words. */
  refused: string;
  message: string;
}

export const UNKNOWN_SECURITY: LinkSecurity = {
  state: "unsecured",
  hubKey: "",
  via: "none",
  tlsServed: null,
  upstreamCompat: null,
  refused: "",
  message: "",
};

/** The page's own security context, and whether it can reach the proxy. */
export interface LinkContext {
  loc: PageLocation | null;
  /** The page is served by a `uconsole web` hub with the pinning proxy. */
  proxy: boolean;
  /** The page is a hub's own (its `<meta name="dspi-hub">` is filled): its `/dspi/v1/info` may be read. Default true. */
  hubPage?: boolean;
}

export type Fetcher = (url: string, init?: { method?: string }) => Promise<{ ok: boolean; json(): Promise<unknown> }>;

const LOOPBACK = new Set(["localhost", "127.0.0.1", "[::1]", "::1"]);

export function isLoopbackHost(hostname: string): boolean {
  const h = hostname.toLowerCase();
  return LOOPBACK.has(h) || h.endsWith(".localhost");
}

/** `url` as this page may open it: on an https page `ws://` becomes `wss://` (never mixed content). */
export function forPage(url: string, loc: PageLocation | null): string {
  if (loc?.protocol === "https:" && url.startsWith("ws://")) return `wss://${url.slice("ws://".length)}`;
  return url;
}

/** `host:port` of a `ws(s)://` URL, the hub's port when none is written. */
export function hubAuthority(url: string): string {
  const u = new URL(url);
  return u.port ? u.host : `${u.host}:11915`;
}

/** The proxy's address for the hub at `url` on the page at `loc`. */
export function proxiedUrl(url: string, loc: PageLocation): string {
  return `ws://${loc.host}/hub/${encodeURIComponent(hubAuthority(url))}/dspi/v1`;
}

/** Whether `url` names the page's own host (the hub that served it). */
function isServingHub(url: string, loc: PageLocation | null): boolean {
  if (!loc) return false;
  try {
    return new URL(url).host.toLowerCase() === loc.host.toLowerCase();
  } catch {
    return false;
  }
}

/**
 * The URL the page's core opens for the hub at `url`: through the proxy on a `uconsole web`
 * page (for every hub not on this machine), else `forPage`'s.
 */
export function routeLink(url: string, ctx: LinkContext): string {
  const page = forPage(url, ctx.loc);
  if (!ctx.proxy || !ctx.loc || !page.startsWith("ws://") || isServingHub(page, ctx.loc)) return page;
  const u = new URL(page);
  // A hub on this machine (the page's own, by any loopback name) needs no proxy: nothing crosses a network.
  if (u.pathname.startsWith("/hub/") || isLoopbackHost(u.hostname)) return page;
  return proxiedUrl(page, ctx.loc);
}

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/** A pin (base64url SPKI hash) for people: `SHA256:` and the first 80 bits in Crockford base32, the hub's display form. */
export function fingerprint(b64url: string): string {
  let bytes: Uint8Array;
  try {
    const b64 = b64url.trim().replace(/-/g, "+").replace(/_/g, "/");
    const bin = atob(b64 + "=".repeat((4 - (b64.length % 4)) % 4));
    bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
  } catch {
    return "";
  }
  if (bytes.length !== 32) return "";
  let chars = "";
  for (let chunk = 0; chunk < 2; chunk++) {
    let bits = 0n;
    for (let i = 0; i < 5; i++) bits = (bits << 8n) | BigInt(bytes[chunk * 5 + i]!);
    for (let i = 7; i >= 0; i--) chars += CROCKFORD[Number((bits >> BigInt(i * 5)) & 31n)];
  }
  return `SHA256:${chars.match(/.{4}/g)!.join("-")}`;
}

interface HubInfo {
  tls?: boolean;
  tls_spki?: string;
  upstream_compat?: boolean;
  proxy?: boolean;
}

async function readJson<T>(fetcher: Fetcher, url: string): Promise<T | null> {
  try {
    const r = await fetcher(url);
    if (!r.ok) return null;
    const v = await r.json();
    return v && typeof v === "object" ? (v as T) : null;
  } catch {
    return null;
  }
}

function defaultFetcher(): Fetcher | null {
  return typeof fetch === "function" ? (u, init) => fetch(u, { cache: "no-store", ...init }) : null;
}

function pageLocation(): PageLocation | null {
  const l = (globalThis as { location?: PageLocation }).location;
  return l && (l.protocol === "http:" || l.protocol === "https:") ? { protocol: l.protocol, host: l.host, hostname: l.hostname } : null;
}

/** Whether a hub served this page (it fills `<meta name="dspi-hub">`); a static host's page asks no hub route. */
function servedByHub(): boolean {
  const d = (globalThis as { document?: { querySelector(s: string): { getAttribute(n: string): string | null } | null } }).document;
  return !!d?.querySelector('meta[name="dspi-hub"]')?.getAttribute("content");
}

let context: Promise<LinkContext> | null = null;

/** This page's link context, read once: its location, and its hub's `/dspi/v1/info` (`proxy`). */
export function linkContext(fetcher: Fetcher | null = defaultFetcher(), loc: PageLocation | null = pageLocation()): Promise<LinkContext> {
  context ??= (async () => {
    const hubPage = servedByHub();
    if (!loc || !fetcher || !hubPage) return { loc, proxy: false, hubPage };
    const info = loc.protocol === "http:" && isLoopbackHost(loc.hostname) ? await readJson<HubInfo>(fetcher, "/dspi/v1/info") : null;
    return { loc, proxy: info?.proxy === true, hubPage };
  })();
  return context;
}

/** For tests: read the context again. */
export function resetLinkContext(): void {
  context = null;
}

/** The security of the link the core opened to `routed` (what `routeLink` gave for the hub). */
export async function securityOf(routed: string, ctx: LinkContext, fetcher: Fetcher | null = defaultFetcher()): Promise<LinkSecurity> {
  let u: URL;
  try {
    u = new URL(routed);
  } catch {
    return UNKNOWN_SECURITY;
  }
  const serving = isServingHub(routed, ctx.loc) && ctx.hubPage !== false;
  // The proxy: its own answer for the target.
  const proxied = ctx.proxy && serving && u.protocol === "ws:" && u.pathname.startsWith("/hub/") && u.pathname.endsWith("/dspi/v1");
  if (proxied) {
    const s = fetcher ? await readJson<Record<string, unknown>>(fetcher, `${u.pathname}/security`) : null;
    const encrypted = s?.encrypted === true;
    return {
      state: encrypted ? "encrypted" : "unsecured",
      hubKey: typeof s?.hubKey === "string" ? s.hubKey : "",
      via: encrypted ? "proxy" : "none",
      tlsServed: typeof s?.tls === "boolean" ? s.tls : null,
      upstreamCompat: typeof s?.upstreamCompat === "boolean" ? s.upstreamCompat : null,
      refused: typeof s?.refused === "string" ? s.refused : "",
      message: typeof s?.message === "string" ? s.message : "",
    };
  }
  // The serving hub's own facts, same origin.
  const info = serving && fetcher ? await readJson<HubInfo>(fetcher, "/dspi/v1/info") : null;
  const base = {
    tlsServed: typeof info?.tls === "boolean" ? info.tls : null,
    upstreamCompat: typeof info?.upstream_compat === "boolean" ? info.upstream_compat : null,
    refused: "",
    message: "",
  };
  if (u.protocol === "wss:") {
    return { ...base, state: "encrypted", via: "browser", hubKey: serving && info?.tls_spki ? fingerprint(info.tls_spki) : "" };
  }
  if (isLoopbackHost(u.hostname)) return { ...base, state: "local", via: "none", hubKey: "" };
  return { ...base, state: "unsecured", via: "none", hubKey: "" };
}

/** Forget the proxy's pin for the hub at `url` (the user removed it): it is pinned anew next time. */
export async function forgetProxyPin(url: string, ctx: LinkContext, fetcher: Fetcher | null = defaultFetcher()): Promise<void> {
  if (!ctx.proxy || !ctx.loc || !fetcher) return;
  try {
    await fetcher(`/hub/${encodeURIComponent(hubAuthority(forPage(url, ctx.loc)))}/dspi/v1/forget`, { method: "POST" });
  } catch {
    // Nothing pinned, or the proxy is gone: nothing to forget.
  }
}

/** The https console of the hub that served this http page, when it serves TLS ("" otherwise). */
export function httpsConsoleOf(ctx: LinkContext, sec: LinkSecurity): string {
  if (!ctx.loc || ctx.loc.protocol !== "http:" || sec.tlsServed !== true || isLoopbackHost(ctx.loc.hostname)) return "";
  return `https://${ctx.loc.host}/console/`;
}
