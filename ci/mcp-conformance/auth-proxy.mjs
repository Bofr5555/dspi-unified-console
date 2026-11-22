// WP MCP6: the conformance suite's runner has no way to send a header, and
// DSPi Console's endpoint answers 401 without a client's token. This passes
// each request on unchanged (Host and Origin included, so the suite's
// DNS-rebinding checks reach the server's own rules) with the token added.
// Node's standard library only.
//
// The server closes its connection after each reply. The suite leaves some
// replies unread (server-sse-multiple-streams reads three tools/list replies'
// headers only), and Node's fetch (undici, Node 24) then fails an internal
// assertion when the connection ends under a reply it has paused, which
// ends the whole run. So the proxy keeps the suite's side of each connection
// open: it drops the server's `Connection: close` on the way back.
//
//   PROXY_PORT=47999 SERVER_PORT=<port> MCP_TOKEN=<token> node auth-proxy.mjs
import http from 'node:http';

const listen = Number(process.env.PROXY_PORT);
const target = Number(process.env.SERVER_PORT);
const token = process.env.MCP_TOKEN ?? '';

http
  .createServer((req, res) => {
    const headers = { ...req.headers, authorization: `Bearer ${token}` };
    const up = http.request(
      { host: '127.0.0.1', port: target, method: req.method, path: req.url, headers, setHost: false },
      (r) => {
        const back = { ...r.headers };
        delete back.connection;
        res.writeHead(r.statusCode ?? 502, back);
        r.pipe(res);
      },
    );
    up.on('error', (e) => {
      res.writeHead(502, { 'content-type': 'text/plain' });
      res.end(`auth-proxy: ${e.message}`);
    });
    req.pipe(up);
  })
  .listen(listen, '127.0.0.1', () => console.error(`auth-proxy: 127.0.0.1:${listen} -> 127.0.0.1:${target}`));
