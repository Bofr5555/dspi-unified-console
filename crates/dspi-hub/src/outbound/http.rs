//! A minimal HTTP/1.1 POST over a `TcpStream` (WP P2WH, PLAN-P2 M4): `http://` only
//! (OD-P2f), bounded in time (connect, then the whole exchange) and in what it reads
//! (the status line, within [`HttpLimits::max_response`] bytes). Nothing after the
//! status line is read or parsed (review N6).
//!
//! Network I/O runs in real time ([`std::time::Instant`]), not on the hub's clock: a
//! socket timeout is the OS's.

use std::io::{self, Read, Write};
use std::net::{Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

/// The longest URL a target may have, in bytes.
pub const MAX_URL_BYTES: usize = 2048;
/// Addresses tried for one host name, at most.
const MAX_ADDRS: usize = 8;

/// How long and how much.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpLimits {
    /// To connect (all addresses of the host together).
    pub connect: Duration,
    /// For the whole delivery: resolve, connect, send, read the status line.
    pub total: Duration,
    /// Bytes of the response read, at most.
    pub max_response: usize,
}

impl Default for HttpLimits {
    /// 5 s to connect, 10 s in all, 8 KiB of the response.
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(5),
            total: Duration::from_secs(10),
            max_response: 8 * 1024,
        }
    }
}

/// Why a URL is not a target's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
    /// `https://`: no TLS in this version.
    #[error(
        "https:// is not supported: the hub sends plain http:// only (no TLS in this version, OD-P2f); use a receiver on a network you trust"
    )]
    Https,
    /// Another scheme, or none.
    #[error("the URL must start with http://")]
    Scheme,
    /// Longer than [`MAX_URL_BYTES`].
    #[error("the URL is longer than {MAX_URL_BYTES} bytes")]
    TooLong,
    /// Something else in it is wrong.
    #[error("the URL is not valid: {0}")]
    Invalid(&'static str),
}

/// An `http://` URL, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpUrl {
    /// The host to resolve: a name, an IPv4 address, or an IPv6 address without brackets.
    pub host: String,
    /// The port (80 unless given).
    pub port: u16,
    /// The request target: the path and query, starting with `/`.
    pub path: String,
    /// The `Host` header's value.
    pub host_header: String,
}

/// Parse a target's URL. Only `http://host[:port][/path][?query]`; no credentials in
/// it, no spaces or control characters; a `#fragment` is dropped.
pub fn parse_url(text: &str) -> Result<HttpUrl, UrlError> {
    if text.len() > MAX_URL_BYTES {
        return Err(UrlError::TooLong);
    }
    let lower_prefix = |n: usize| text.get(..n).map(str::to_ascii_lowercase);
    if lower_prefix(8).as_deref() == Some("https://") {
        return Err(UrlError::Https);
    }
    if lower_prefix(7).as_deref() != Some("http://") {
        return Err(UrlError::Scheme);
    }
    if !text.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(UrlError::Invalid("spaces, control or non-ASCII characters"));
    }
    let rest = text.get(7..).unwrap_or_default();
    let rest = rest.split('#').next().unwrap_or_default();
    let split = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(split);
    if authority.contains('@') {
        return Err(UrlError::Invalid("credentials in the URL"));
    }
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (addr, after) = v6
            .split_once(']')
            .ok_or(UrlError::Invalid("an unclosed [ in the host"))?;
        addr.parse::<Ipv6Addr>()
            .map_err(|_| UrlError::Invalid("not an IPv6 address in [ ]"))?;
        let port = match after {
            "" => None,
            p => Some(
                p.strip_prefix(':')
                    .ok_or(UrlError::Invalid("text after ]"))?,
            ),
        };
        (addr, port)
    } else {
        match authority.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return Err(UrlError::Invalid("no host"));
    }
    if !authority.starts_with('[')
        && !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    {
        return Err(UrlError::Invalid("a character a host name cannot have"));
    }
    let port = match port {
        None => 80,
        Some(p) => match p.parse::<u16>() {
            Ok(n) if n > 0 && p.bytes().all(|b| b.is_ascii_digit()) => n,
            _ => return Err(UrlError::Invalid("the port is not 1 to 65535")),
        },
    };
    let path = match path {
        "" => "/".to_owned(),
        p if p.starts_with('?') => format!("/{p}"),
        p => p.to_owned(),
    };
    let shown = if authority.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_ascii_lowercase()
    };
    let host_header = if port == 80 {
        shown
    } else {
        format!("{shown}:{port}")
    };
    Ok(HttpUrl {
        host: host.to_ascii_lowercase(),
        port,
        path,
        host_header,
    })
}

/// Why a POST did not get a status line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PostError {
    /// The host name did not resolve.
    #[error("cannot resolve the host: {0}")]
    Resolve(String),
    /// No address of the host took the connection.
    #[error("cannot connect: {0}")]
    Connect(String),
    /// Out of time (connect or the whole exchange).
    #[error("timed out")]
    Timeout,
    /// The connection failed while sending or reading.
    #[error("connection failed: {0}")]
    Io(String),
    /// No line end within the read limit: the rest was never read.
    #[error("no status line in the first {read} bytes of the reply")]
    TooLong {
        /// Bytes read before giving up.
        read: usize,
    },
    /// The receiver closed before a whole status line.
    #[error("the receiver closed the connection without a reply")]
    Closed,
    /// The first line is not `HTTP/1.x NNN ...`.
    #[error("the reply is not HTTP")]
    NotHttp,
}

/// One POST.
#[derive(Debug, Clone, Copy)]
pub struct Post<'a> {
    /// Where.
    pub url: &'a HttpUrl,
    /// `Content-Type`.
    pub content_type: &'a str,
    /// Further headers (names and values the hub made; no line breaks).
    pub headers: &'a [(&'a str, String)],
    /// The body.
    pub body: &'a [u8],
}

fn remaining(deadline: Instant) -> Result<Duration, PostError> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        Err(PostError::Timeout)
    } else {
        Ok(left)
    }
}

fn io_err(e: &io::Error) -> PostError {
    match e.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => PostError::Timeout,
        _ => PostError::Io(e.to_string()),
    }
}

/// The request head.
fn head(post: &Post<'_>) -> String {
    let mut head = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: dspi-hub/{}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        post.url.path,
        post.url.host_header,
        env!("CARGO_PKG_VERSION"),
        post.content_type,
        post.body.len()
    );
    for (name, value) in post.headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    head
}

fn connect(url: &HttpUrl, limits: &HttpLimits, deadline: Instant) -> Result<TcpStream, PostError> {
    let addrs: Vec<SocketAddr> = (url.host.as_str(), url.port)
        .to_socket_addrs()
        .map_err(|e| PostError::Resolve(e.to_string()))?
        .take(MAX_ADDRS)
        .collect();
    let connect_by = Instant::now()
        .checked_add(limits.connect)
        .map_or(deadline, |c| c.min(deadline));
    let mut last = PostError::Resolve("no address".into());
    for addr in addrs {
        let left = remaining(connect_by)?;
        match TcpStream::connect_timeout(&addr, left) {
            Ok(stream) => return Ok(stream),
            Err(e) => last = PostError::Connect(e.to_string()),
        }
    }
    Err(last)
}

/// The status code of `HTTP/1.x NNN reason`.
fn status(line: &[u8]) -> Result<u16, PostError> {
    let line = std::str::from_utf8(line).map_err(|_| PostError::NotHttp)?;
    let mut words = line.trim_end().split(' ');
    let version = words.next().unwrap_or_default();
    if !version.starts_with("HTTP/1.") {
        return Err(PostError::NotHttp);
    }
    let code = words.next().unwrap_or_default();
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(PostError::NotHttp);
    }
    code.parse().map_err(|_| PostError::NotHttp)
}

/// POST, and answer the reply's status code. Blocks for at most `limits.total`.
pub fn post(post: &Post<'_>, limits: &HttpLimits) -> Result<u16, PostError> {
    let deadline = Instant::now()
        .checked_add(limits.total)
        .ok_or(PostError::Timeout)?;
    let mut stream = connect(post.url, limits, deadline)?;
    let _ = stream.set_nodelay(true);
    stream
        .set_write_timeout(Some(remaining(deadline)?))
        .map_err(|e| io_err(&e))?;
    let head = head(post);
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(post.body))
        .and_then(|()| stream.flush())
        .map_err(|e| io_err(&e))?;
    // The status line only, within the limit: never a byte past it.
    let mut got: Vec<u8> = Vec::with_capacity(limits.max_response.min(1024));
    let mut chunk = [0u8; 512];
    loop {
        if let Some(end) = got.iter().position(|&b| b == b'\n') {
            return status(got.get(..end).unwrap_or_default());
        }
        let room = limits.max_response.saturating_sub(got.len());
        if room == 0 {
            return Err(PostError::TooLong { read: got.len() });
        }
        stream
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(|e| io_err(&e))?;
        let want = room.min(chunk.len());
        let buf = chunk.get_mut(..want).unwrap_or_default();
        match stream.read(buf) {
            Ok(0) => return Err(PostError::Closed),
            Ok(n) => got.extend_from_slice(buf.get(..n).unwrap_or_default()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io_err(&e)),
        }
    }
}

#[cfg(test)]
mod tests_wp_p2wh {
    #![allow(
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[test]
    fn wp_p2wh_urls_http_only_https_refused_with_a_reason() {
        let u = parse_url("http://Example.lan:8123/api/hook?x=1#frag").unwrap();
        assert_eq!(u.host, "example.lan");
        assert_eq!(u.port, 8123);
        assert_eq!(u.path, "/api/hook?x=1");
        assert_eq!(u.host_header, "example.lan:8123");
        let u = parse_url("HTTP://10.0.0.5").unwrap();
        assert_eq!(
            (u.port, u.path.as_str(), u.host_header.as_str()),
            (80, "/", "10.0.0.5")
        );
        let u = parse_url("http://[::1]:9/x").unwrap();
        assert_eq!(
            (u.host.as_str(), u.host_header.as_str()),
            ("::1", "[::1]:9")
        );
        assert_eq!(parse_url("http://h?q").unwrap().path, "/?q");

        let https = parse_url("https://ntfy.sh/topic").unwrap_err();
        assert_eq!(https, UrlError::Https);
        assert!(https.to_string().contains("no TLS"), "{https}");
        assert_eq!(parse_url("HTTPS://x").unwrap_err(), UrlError::Https);
        assert_eq!(parse_url("ftp://x").unwrap_err(), UrlError::Scheme);
        assert_eq!(parse_url("x").unwrap_err(), UrlError::Scheme);
        for bad in [
            "http://",
            "http://user:pw@host/",
            "http://host:0/",
            "http://host:99999/",
            "http://host:+80/",
            "http://ho st/",
            "http://host/a b",
            "http://host/\r\nX: y",
            "http://[::1/",
            "http://[nope]/",
            "http://h_st/",
            "http://hést/",
        ] {
            assert!(
                matches!(parse_url(bad), Err(UrlError::Invalid(_))),
                "{bad}: {:?}",
                parse_url(bad)
            );
        }
        let long = format!("http://h/{}", "a".repeat(MAX_URL_BYTES));
        assert_eq!(parse_url(&long).unwrap_err(), UrlError::TooLong);
    }

    /// A listener that answers each connection with `reply` after reading the request
    /// head and body, and passes the request on.
    fn listener(reply: &'static [u8]) -> (u16, mpsc::Receiver<Vec<u8>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { return };
                let mut req = Vec::new();
                let mut b = [0u8; 4096];
                // Head, then as much body as Content-Length says.
                loop {
                    let n = s.read(&mut b).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&b[..n]);
                    let text = String::from_utf8_lossy(&req).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len: usize = text
                            .lines()
                            .find_map(|l| l.strip_prefix("Content-Length: "))
                            .map_or(0, |v| v.trim().parse().unwrap());
                        if req.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                let _ = s.write_all(reply);
                let _ = tx.send(req);
            }
        });
        (port, rx)
    }

    fn url(port: u16) -> HttpUrl {
        parse_url(&format!("http://127.0.0.1:{port}/hook")).unwrap()
    }

    #[test]
    fn wp_p2wh_a_post_sends_head_and_body_and_reads_the_status() {
        let (port, rx) = listener(b"HTTP/1.1 204 No Content\r\nX-Big: yes\r\n\r\n");
        let u = url(port);
        let headers = [("X-DSPi-Timestamp", "12".to_owned())];
        let p = Post {
            url: &u,
            content_type: "application/json",
            headers: &headers,
            body: b"{\"a\":1}",
        };
        assert_eq!(post(&p, &HttpLimits::default()), Ok(204));
        let req = String::from_utf8(rx.recv().unwrap()).unwrap();
        assert!(req.starts_with("POST /hook HTTP/1.1\r\n"), "{req}");
        assert!(
            req.contains(&format!("\r\nHost: 127.0.0.1:{port}\r\n")),
            "{req}"
        );
        assert!(req.contains("\r\nContent-Length: 7\r\n"), "{req}");
        assert!(req.contains("\r\nX-DSPi-Timestamp: 12\r\n"), "{req}");
        assert!(req.ends_with("\r\n\r\n{\"a\":1}"), "{req}");

        let (port, _rx) = listener(b"HTTP/1.0 500 Oops\n");
        let u = url(port);
        assert_eq!(
            post(&Post { url: &u, ..p }, &HttpLimits::default()),
            Ok(500)
        );
        let (port, _rx) = listener(b"SSH-2.0-x\r\n");
        let u = url(port);
        assert_eq!(
            post(&Post { url: &u, ..p }, &HttpLimits::default()),
            Err(PostError::NotHttp)
        );
        let (port, _rx) = listener(b"HTTP/1.1 2");
        let u = url(port);
        assert_eq!(
            post(&Post { url: &u, ..p }, &HttpLimits::default()),
            Err(PostError::Closed)
        );
    }

    #[test]
    fn wp_p2wh_an_endless_reply_is_cut_at_8_kib() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut sent = 0usize;
            let chunk = [b'A'; 1024];
            // Endless, no line end, until the hub closes.
            while s.write_all(&chunk).is_ok() {
                sent += chunk.len();
                if sent > 64 << 20 {
                    break;
                }
            }
            let _ = tx.send(sent);
        });
        let u = url(port);
        let p = Post {
            url: &u,
            content_type: "text/plain",
            headers: &[],
            body: b"x",
        };
        let started = Instant::now();
        let got = post(&p, &HttpLimits::default());
        assert_eq!(got, Err(PostError::TooLong { read: 8 * 1024 }));
        assert!(started.elapsed() < Duration::from_secs(5));
        // The hub hung up: the sender stops long before its 64 MiB.
        let sent = rx.recv_timeout(Duration::from_secs(20)).unwrap();
        assert!(sent < 64 << 20, "sent {sent}");
    }

    #[test]
    fn wp_p2wh_dead_and_silent_receivers_fail_in_time() {
        // Nobody listens: refused at once.
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let u = url(port);
        let p = Post {
            url: &u,
            content_type: "text/plain",
            headers: &[],
            body: b"x",
        };
        assert!(matches!(
            post(&p, &HttpLimits::default()),
            Err(PostError::Connect(_))
        ));
        // Accepts and says nothing: the total limit ends it.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let u = url(l.local_addr().unwrap().port());
        let limits = HttpLimits {
            total: Duration::from_millis(300),
            ..HttpLimits::default()
        };
        let started = Instant::now();
        assert_eq!(
            post(&Post { url: &u, ..p }, &limits),
            Err(PostError::Timeout)
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        drop(l);
    }
}
