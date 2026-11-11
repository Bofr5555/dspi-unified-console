// A hub that serves the web console, for the Link e2e tests (WP WB4b). Loopback
// only; a test tool, not a product (WB7 makes `uconsole hub` serve the console).
//
//   node tools/hub-host.mjs <dist> <port> <dspi-link-fakehub>
//
// It plays what a real hub does for a browser: it serves the page from its own
// address, and it enforces Link spec 4's Origin rule on the WebSocket upgrade (an
// `Origin` naming any site but this one is refused with HTTP 403, as `uconsole hub`
// does). Behind that, each test gets fake hubs of its own (`dspi-link-fakehub`,
// the protocol's fake from crates/dspi-link), so faults in one test never reach
// another:
//
//   POST /__hub?args=--auth%20fixed:123456   start one; answers {"id": n}
//   POST /__fault?hub=n   body: one fault line ("drop", "close 4000", ...: see
//                         the fake hub's own help); answers its "ok"
//   GET  /?hub=n          the standalone page, its <meta name="dspi-hub"> naming
//                         hub n's address on this origin: ws://<this>/h/n/dspi/v1
//   ws://<this>/h/n/dspi/v1   hub n (the Origin rule first)
//
// The page's address names the fake hub through a path on this origin: what a
// fleet server does when it proxies its hubs (apps/web/README.md).

import { spawn } from "node:child_process";
import { connect } from "node:net";
import { createServer as httpServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { createInterface } from "node:readline";
import { extname, join, normalize, resolve, sep } from "node:path";

const dir = resolve(process.argv[2] ?? "dist");
const port = Number(process.argv[3] ?? 8738);
const fakehub = process.argv[4];
if (!fakehub) {
  console.error("usage: hub-host.mjs <dist> <port> <dspi-link-fakehub>");
  process.exit(2);
}
const own = new Set([`http://127.0.0.1:${port}`, `http://localhost:${port}`]);
const types = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json",
  ".map": "application/json",
  ".wasm": "application/wasm",
};

/** n -> { port, child, lines } */
const hubs = new Map();
let nextHub = 1;

function startHub(args) {
  return new Promise((ok, fail) => {
    const child = spawn(fakehub, ["--bind", "127.0.0.1:0", ...args], { stdio: ["pipe", "pipe", "inherit"] });
    const lines = createInterface({ input: child.stdout });
    const waiting = [];
    let started = false;
    lines.on("line", (line) => {
      if (!started) {
        const m = / on 127\.0\.0\.1:(\d+) /.exec(line);
        if (!m) return;
        started = true;
        const id = nextHub++;
        hubs.set(id, { port: Number(m[1]), child, waiting });
        ok(id);
        return;
      }
      waiting.shift()?.(line.trim());
    });
    child.on("error", fail);
    child.on("exit", (code, signal) => {
      if (!started) fail(new Error(`fake hub exited ${code ?? signal} before it listened`));
    });
  });
}

function fault(id, line) {
  const hub = hubs.get(id);
  if (!hub) return Promise.resolve("no such hub");
  return new Promise((ok) => {
    hub.waiting.push(ok);
    hub.child.stdin.write(`${line.trim()}\n`);
  });
}

async function body(req) {
  let s = "";
  for await (const chunk of req) s += chunk;
  return s;
}

const server = httpServer(async (req, res) => {
  const url = new URL(req.url ?? "/", `http://127.0.0.1:${port}`);
  try {
    if (req.method === "POST" && url.pathname === "/__hub") {
      const args = (url.searchParams.get("args") ?? "").split(" ").filter(Boolean);
      const id = await startHub(args);
      res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ id }));
      return;
    }
    if (req.method === "POST" && url.pathname === "/__fault") {
      const answer = await fault(Number(url.searchParams.get("hub")), await body(req));
      res.writeHead(answer === "ok" ? 200 : 400).end(answer);
      return;
    }
  } catch (e) {
    console.error(`hub host: ${e}`);
    if (!res.headersSent) res.writeHead(500);
    res.end(String(e));
    return;
  }
  if (req.method !== "GET" && req.method !== "HEAD") {
    res.writeHead(405).end();
    return;
  }
  let path;
  try {
    path = decodeURIComponent(url.pathname);
  } catch {
    res.writeHead(400).end();
    return;
  }
  let file = normalize(join(dir, path));
  if (file !== dir && !file.startsWith(dir + sep)) {
    res.writeHead(403).end();
    return;
  }
  try {
    if ((await stat(file)).isDirectory()) file = join(file, "index.html");
    let content = await readFile(file);
    if (file === join(dir, "index.html") && url.searchParams.has("hub")) {
      // What a hub serving the page writes into it: its own Link address.
      const address = `ws://127.0.0.1:${port}/h/${Number(url.searchParams.get("hub"))}/dspi/v1`;
      content = Buffer.from(content.toString("utf8").replace('<meta name="dspi-hub" content="" />', `<meta name="dspi-hub" content="${address}" />`));
    }
    res.writeHead(200, { "content-type": types[extname(file)] ?? "application/octet-stream", "cache-control": "no-store" });
    res.end(req.method === "HEAD" ? undefined : content);
  } catch {
    res.writeHead(404).end();
  }
});

server.on("upgrade", (req, socket, head) => {
  const m = /^\/h\/(\d+)\/dspi\/v1$/.exec(new URL(req.url ?? "/", "http://x").pathname);
  const hub = m ? hubs.get(Number(m[1])) : undefined;
  const origin = req.headers.origin;
  // Link spec 4: no Origin (a native client), or this site's own.
  if (!hub || (origin !== undefined && !own.has(origin))) {
    socket.end(`HTTP/1.1 ${hub ? "403 Forbidden" : "404 Not Found"}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n`);
    return;
  }
  const upstream = connect(hub.port, "127.0.0.1", () => {
    // The request as the browser sent it, to the fake hub's own path.
    let head0 = `${req.method} /dspi/v1 HTTP/1.1\r\n`;
    for (let i = 0; i < req.rawHeaders.length; i += 2) head0 += `${req.rawHeaders[i]}: ${req.rawHeaders[i + 1]}\r\n`;
    upstream.write(`${head0}\r\n`);
    if (head?.length) upstream.write(head);
    upstream.pipe(socket);
    socket.pipe(upstream);
  });
  const end = () => {
    socket.destroy();
    upstream.destroy();
  };
  upstream.on("error", end);
  socket.on("error", end);
  upstream.on("close", end);
  socket.on("close", end);
});

server.listen(port, "127.0.0.1", () => console.log(`hub host on http://127.0.0.1:${port}/ serving ${dir}`));
const stop = () => {
  for (const h of hubs.values()) h.child.kill();
  process.exit(0);
};
process.on("SIGTERM", stop);
process.on("SIGINT", stop);
