//! Manual hub entry (spec 3.3) and the info endpoint (spec 3.2).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::clock::Instant;

use crate::messages::InfoDoc;
use crate::{LinkError, DEFAULT_PATH, DEFAULT_PORT};

/// `host`, `host:port`, `[v6]`, `[v6]:port` or a bare IPv6 address, as the
/// user typed it (port. of macOS `HubBrowser.parseAddress`). None when it
/// cannot be an address.
pub fn parse_address(text: &str) -> Option<(String, u16)> {
    let t = text.trim();
    if t.is_empty() || t.contains(char::is_whitespace) || t.contains('/') {
        return None;
    }
    if let Some(rest) = t.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        if host.is_empty() {
            return None;
        }
        if tail.is_empty() {
            return Some((host.to_owned(), DEFAULT_PORT));
        }
        let port = tail.strip_prefix(':')?.parse::<u16>().ok()?;
        return Some((host.to_owned(), port));
    }
    if t.matches(':').count() > 1 {
        return Some((t.to_owned(), DEFAULT_PORT));
    }
    if let Some((host, port)) = t.rsplit_once(':') {
        let port = port.parse::<u16>().ok()?;
        return if host.is_empty() { None } else { Some((host.to_owned(), port)) };
    }
    Some((t.to_owned(), DEFAULT_PORT))
}

fn bracket(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

/// `ws://host:port/path` (`path` defaults to `/dspi/v1`).
pub fn ws_url(host: &str, port: u16, path: Option<&str>) -> String {
    let path = path.filter(|p| p.starts_with('/')).unwrap_or(DEFAULT_PATH);
    format!("ws://{}:{port}{path}", bracket(host))
}

/// `GET http://host:port/dspi/v1/info` (plain HTTP/1.1, no redirects).
pub fn fetch_info(host: &str, port: u16, timeout: Duration) -> Result<InfoDoc, LinkError> {
    let io = |e: std::io::Error| LinkError::Io(e.to_string());
    let addr = (host, port).to_socket_addrs().map_err(io)?.next().ok_or_else(|| LinkError::Io("no address".into()))?;
    let mut s = TcpStream::connect_timeout(&addr, timeout).map_err(io)?;
    s.set_read_timeout(Some(timeout)).map_err(io)?;
    s.set_write_timeout(Some(timeout)).map_err(io)?;
    write!(
        s,
        "GET {DEFAULT_PATH}/info HTTP/1.1\r\nHost: {}:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        bracket(host)
    )
    .map_err(io)?;
    // -- WP LK4 -- (F1) the response ends where its framing says, not at EOF:
    // a hub may keep the socket open after its answer whatever
    // `Connection: close` asked (the ESP32 bridge's HTTP server does, WP LK3).
    let deadline = Instant::now() + timeout;
    let (head, body) = read_response(&mut Deadlined { s: &mut s, deadline }).map_err(|e| match e {
        Framing::Io(e) => io(e),
        Framing::Bad(msg) => LinkError::Protocol(msg.into()),
    })?;
    let status = head.split_whitespace().nth(1).unwrap_or("");
    if status != "200" {
        return Err(LinkError::Protocol(format!("info endpoint answered {status}")));
    }
    serde_json::from_slice(&body).map_err(|e| LinkError::Protocol(e.to_string()))
}

/// Most a response head may take.
const HEAD_MAX: usize = 16 * 1024;
/// Most a response body may take, however it is framed.
const BODY_MAX: usize = 256 * 1024;

/// Why a response could not be read.
#[derive(Debug)]
enum Framing {
    Io(std::io::Error),
    Bad(&'static str),
}

impl From<std::io::Error> for Framing {
    fn from(e: std::io::Error) -> Self {
        Framing::Io(e)
    }
}

/// A socket whose reads end at one deadline for the whole response, so a hub
/// that trickles bytes cannot hold the caller past `timeout`.
struct Deadlined<'a> {
    s: &'a mut TcpStream,
    deadline: Instant,
}

impl Read for Deadlined<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.s.set_read_timeout(Some(left))?;
        self.s.read(buf)
    }
}

/// Read one HTTP/1.1 response: the head (status line and headers, as text),
/// then the body as its framing says: `Transfer-Encoding: chunked` up to the
/// last chunk, `Content-Length` bytes, or, with neither, up to EOF. The
/// socket may stay open after a framed body. Heads over [`HEAD_MAX`] and
/// bodies over [`BODY_MAX`] are refused.
fn read_response(r: &mut impl Read) -> Result<(String, Vec<u8>), Framing> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    let end = loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break end;
        }
        if buf.len() > HEAD_MAX {
            return Err(Framing::Bad("HTTP header too long"));
        }
        match r.read(&mut chunk)? {
            0 => return Err(Framing::Bad("no HTTP header")),
            n => buf.extend_from_slice(chunk.get(..n).unwrap_or_default()),
        }
    };
    let head = String::from_utf8_lossy(buf.get(..end).unwrap_or_default()).into_owned();
    let mut rest = buf.split_off(end + 4);
    let header = |name: &str| {
        head.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_ascii_lowercase())
        })
    };
    if header("transfer-encoding").is_some_and(|v| v.split(',').any(|t| t.trim() == "chunked")) {
        return Ok((head, read_chunked(r, rest)?));
    }
    if let Some(v) = header("content-length") {
        let len: usize = v.parse().map_err(|_| Framing::Bad("bad Content-Length"))?;
        if len > BODY_MAX {
            return Err(Framing::Bad("HTTP body too long"));
        }
        while rest.len() < len {
            match r.read(&mut chunk)? {
                0 => return Err(Framing::Bad("HTTP body cut short")),
                n => rest.extend_from_slice(chunk.get(..n).unwrap_or_default()),
            }
        }
        rest.truncate(len);
        return Ok((head, rest));
    }
    // Close-delimited (RFC 9112 6.3, last rule)
    loop {
        if rest.len() > BODY_MAX {
            return Err(Framing::Bad("HTTP body too long"));
        }
        match r.read(&mut chunk)? {
            0 => return Ok((head, rest)),
            n => rest.extend_from_slice(chunk.get(..n).unwrap_or_default()),
        }
    }
}

/// A chunked body (RFC 9112 7.1), `have` being what came with the head. Ends
/// at the last chunk and its trailers, without waiting for EOF.
fn read_chunked(r: &mut impl Read, mut have: Vec<u8>) -> Result<Vec<u8>, Framing> {
    let mut chunk = [0u8; 2048];
    let mut more = |have: &mut Vec<u8>| -> Result<(), Framing> {
        match r.read(&mut chunk)? {
            0 => Err(Framing::Bad("chunked HTTP body cut short")),
            n => {
                have.extend_from_slice(chunk.get(..n).unwrap_or_default());
                Ok(())
            }
        }
    };
    let mut body = Vec::new();
    let mut at = 0;
    loop {
        // The size line (extensions after ';' ignored)
        let line_end = loop {
            if let Some(i) = have.get(at..).and_then(|h| h.windows(2).position(|w| w == b"\r\n")) {
                break at + i;
            }
            if have.len() - at > 1024 {
                return Err(Framing::Bad("bad chunk size line"));
            }
            more(&mut have)?;
        };
        let line = String::from_utf8_lossy(have.get(at..line_end).unwrap_or_default()).into_owned();
        let size = line.split(';').next().unwrap_or("").trim();
        let n = usize::from_str_radix(size, 16).map_err(|_| Framing::Bad("bad chunk size"))?;
        at = line_end + 2;
        if n == 0 {
            // Trailers, up to an empty line
            loop {
                if let Some(i) = have.get(at..).and_then(|h| h.windows(2).position(|w| w == b"\r\n")) {
                    if i == 0 {
                        return Ok(body);
                    }
                    at += i + 2;
                    continue;
                }
                if have.len() - at > HEAD_MAX {
                    return Err(Framing::Bad("HTTP trailers too long"));
                }
                more(&mut have)?;
            }
        }
        if body.len().saturating_add(n) > BODY_MAX {
            return Err(Framing::Bad("HTTP body too long"));
        }
        while have.len() < at + n + 2 {
            more(&mut have)?;
        }
        body.extend_from_slice(have.get(at..at + n).unwrap_or_default());
        if have.get(at + n..at + n + 2) != Some(b"\r\n") {
            return Err(Framing::Bad("bad chunk end"));
        }
        at += n + 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_parse_like_the_macos_console() {
        assert_eq!(parse_address("192.168.1.20"), Some(("192.168.1.20".into(), 11915)));
        assert_eq!(parse_address(" studio.local:8080 "), Some(("studio.local".into(), 8080)));
        assert_eq!(parse_address("[fe80::1]:11916"), Some(("fe80::1".into(), 11916)));
        assert_eq!(parse_address("[fe80::1]"), Some(("fe80::1".into(), 11915)));
        assert_eq!(parse_address("fe80::1"), Some(("fe80::1".into(), 11915)));
        assert_eq!(parse_address(""), None);
        assert_eq!(parse_address(":80"), None);
        assert_eq!(parse_address("host:port"), None);
        assert_eq!(parse_address("host:99999"), None);
        assert_eq!(parse_address("a b"), None);
    }

    #[test]
    fn urls() {
        assert_eq!(ws_url("10.0.2.2", 11915, None), "ws://10.0.2.2:11915/dspi/v1");
        assert_eq!(ws_url("fe80::1", 1, Some("/x")), "ws://[fe80::1]:1/x");
    }

    // -- WP LK4 -- (F1)
    fn parse(raw: &[u8]) -> Result<(String, Vec<u8>), Framing> {
        read_response(&mut std::io::Cursor::new(raw.to_vec()))
    }

    #[test]
    fn wp_lk4_responses_end_where_their_framing_says() {
        let (head, body) = parse(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nabcdEXTRA").unwrap();
        assert!(head.starts_with("HTTP/1.1 200"));
        assert_eq!(body, b"abcd");
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4;x=y\r\nabcd\r\n2\r\nef\r\n0\r\nX-T: 1\r\n\r\nEXTRA";
        assert_eq!(parse(chunked).unwrap().1, b"abcdef");
        assert_eq!(parse(b"HTTP/1.1 200 OK\r\nCONTENT-length:  3 \r\n\r\nxyz").unwrap().1, b"xyz");
        // Neither: up to EOF
        assert_eq!(parse(b"HTTP/1.1 200 OK\r\nServer: x\r\n\r\nto the end").unwrap().1, b"to the end");
    }

    #[test]
    fn wp_lk4_bad_or_oversized_responses_are_refused() {
        let big = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", BODY_MAX + 1);
        assert!(matches!(parse(big.as_bytes()), Err(Framing::Bad("HTTP body too long"))));
        let mut eof = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        eof.resize(eof.len() + BODY_MAX + 10, b'a');
        assert!(matches!(parse(&eof), Err(Framing::Bad("HTTP body too long"))));
        let chunk = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n", BODY_MAX + 1);
        assert!(matches!(parse(chunk.as_bytes()), Err(Framing::Bad("HTTP body too long"))));
        assert!(matches!(parse(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nabc"), Err(Framing::Bad(_))));
        assert!(matches!(parse(b"HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\n"), Err(Framing::Bad(_))));
        assert!(matches!(parse(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n"), Err(Framing::Bad(_))));
        assert!(matches!(
            parse(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nabcd"),
            Err(Framing::Bad(_))
        ));
        let mut head = b"HTTP/1.1 200 OK\r\nX: ".to_vec();
        head.resize(head.len() + HEAD_MAX + 10, b'a');
        assert!(matches!(parse(&head), Err(Framing::Bad("HTTP header too long"))));
        assert!(matches!(parse(b"HTTP/1.1 200"), Err(Framing::Bad("no HTTP header"))));
    }

    const INFO: &str = r#"{"protocol":1,"hub":{"id":"abc","name":"Bridge","kind":"esp32"}}"#;

    /// A hub that answers `/info` with `answer` and keeps the socket open
    /// (the ESP32 bridge's httpd) until the test ends.
    fn keep_open_hub(answer: Vec<Vec<u8>>, pause: Duration) -> (u16, std::sync::mpsc::Sender<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (done, wait) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = [0u8; 1024];
            let _ = s.read(&mut req);
            for part in answer {
                let _ = s.write_all(&part);
                let _ = s.flush();
                std::thread::sleep(pause);
            }
            let _ = wait.recv_timeout(Duration::from_secs(20));
        });
        (port, done)
    }

    #[test]
    fn wp_lk4_fetch_info_reads_a_hub_that_keeps_the_socket_open() {
        let started = Instant::now();
        let head =
            format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", INFO.len());
        let (port, _done) = keep_open_hub(vec![head.into_bytes(), INFO.as_bytes().to_vec()], Duration::from_millis(50));
        let doc = fetch_info("127.0.0.1", port, Duration::from_secs(5)).expect("info");
        assert_eq!(doc.hub.id, "abc");
        assert!(started.elapsed() < Duration::from_secs(2), "waited for EOF: {:?}", started.elapsed());

        let chunked = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{INFO}\r\n", INFO.len());
        let (port, _done) = keep_open_hub(vec![chunked.into_bytes(), b"0\r\n\r\n".to_vec()], Duration::from_millis(50));
        let doc = fetch_info("127.0.0.1", port, Duration::from_secs(5)).expect("chunked info");
        assert_eq!(doc.hub.name, "Bridge");
    }

    #[test]
    fn wp_lk4_fetch_info_is_bounded_by_its_timeout() {
        // A body that never ends: the whole read, not each read, is bounded
        let head = b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n".to_vec();
        let trickle: Vec<Vec<u8>> = std::iter::once(head).chain((0..40).map(|_| b" ".to_vec())).collect();
        let (port, _done) = keep_open_hub(trickle, Duration::from_millis(100));
        let started = Instant::now();
        assert!(fetch_info("127.0.0.1", port, Duration::from_millis(700)).is_err());
        assert!(started.elapsed() < Duration::from_millis(2500), "{:?}", started.elapsed());
    }
}
