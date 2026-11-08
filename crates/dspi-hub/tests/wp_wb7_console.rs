//! WP WB7: the web console beside the hub's page, over real sockets on loopback. The
//! console is a small fake dist written here; the real one is the browser test's
//! (apps/web/test/e2e/hosting.spec.ts).

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use dspi_hub::server::{Advertise, NetworkConfig};
use dspi_hub::web::{self, Console, ConsoleError};
use dspi_hub::{Hub, HubConfig};
use dspi_transport::FakeDevice;

/// A fresh directory. `ci/run.sh` points TMPDIR at the home volume, not tmpfs.
fn scratch(what: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join("wp-wb7-console").join(format!(
        "{what}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const INDEX: &str = concat!(
    "<!doctype html><head>",
    r#"<meta name="dspi-hub" content="" />"#,
    r#"<meta name="dspi-hub-page" content="" />"#,
    "</head><body><script type=\"module\" src=\"app.js\"></script></body>"
);

/// A dist as `apps/web/tools/build.mjs` writes one, plus what must be left out.
fn fake_dist() -> PathBuf {
    let dir = scratch("dist");
    let write = |rel: &str, body: &[u8]| {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    };
    write("index.html", INDEX.as_bytes());
    write("app.js", b"console.log('app');");
    write("worker.js", b"// worker");
    write("dspi_wasm.wasm", b"\0asm\x01\0\0\0");
    write("dspi-web.css", b".dspi-web{}");
    write("THIRD-PARTY-NOTICES.txt", b"Preact: MIT");
    write("test/usb.html", b"<!doctype html><title>usb</title>");
    // Left out: hidden, not a served kind, odd names.
    write(".env", b"SECRET=1");
    write("notes.md", b"# not served");
    write("run.sh", b"#!/bin/sh");
    write("a b.js", b"spaces");
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/hostname", dir.join("link.txt")).unwrap();
    dir
}

fn start(web: bool, console: Option<&Path>) -> (Hub, u16) {
    let console = console.map(|d| Arc::new(Console::load(d).unwrap()));
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            web,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            advertise: Advertise::Off,
            console,
            ..NetworkConfig::new(scratch("store"))
        }),
        ..HubConfig::in_process("Console hub")
    })
    .unwrap();
    hub.attach(Box::new(FakeDevice::demo()), 0).unwrap();
    let port = hub.network().unwrap().port();
    (hub, port)
}

/// The status, head (lower case) and body of one request.
fn request(port: u16, path: &str, extra: &str) -> (u16, String, Vec<u8>) {
    let mut tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(tcp, "GET {path} HTTP/1.1\r\n{extra}\r\n").unwrap();
    let mut bytes = Vec::new();
    tcp.read_to_end(&mut bytes).unwrap();
    let split = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&bytes[..split]).to_ascii_lowercase();
    let code = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (code, head, bytes[split + 4..].to_vec())
}

fn get(port: u16, path: &str) -> (u16, String, Vec<u8>) {
    request(port, path, &format!("Host: 127.0.0.1:{port}\r\n"))
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines()
        .find_map(|l| l.strip_prefix(&format!("{name}: ")))
        .map(str::trim)
}

#[test]
fn wp_wb7_load_keeps_what_a_build_writes_and_nothing_else() {
    let console = Console::load(&fake_dist()).unwrap();
    let paths: Vec<&str> = console.paths().collect();
    assert_eq!(
        paths,
        [
            "/console/",
            "/console/THIRD-PARTY-NOTICES.txt",
            "/console/app.js",
            "/console/dspi-web.css",
            "/console/dspi_wasm.wasm",
            "/console/test/usb.html",
            "/console/worker.js",
        ]
    );
    let missing = scratch("empty");
    assert!(matches!(
        Console::load(&missing),
        Err(ConsoleError::NoIndex(_))
    ));
    assert!(matches!(
        Console::load(&missing.join("nowhere")),
        Err(ConsoleError::Unreadable { .. })
    ));
}

#[test]
fn wp_wb7_load_refuses_a_directory_far_too_big() {
    let dir = scratch("big");
    std::fs::write(dir.join("index.html"), INDEX).unwrap();
    for i in 0..=web::MAX_CONSOLE_FILES {
        std::fs::write(dir.join(format!("f{i}.js")), b"x").unwrap();
    }
    assert!(matches!(Console::load(&dir), Err(ConsoleError::TooBig(_))));
}

#[test]
fn wp_wb7_the_console_is_served_beside_the_page_with_its_types() {
    let dist = fake_dist();
    let (hub, port) = start(true, Some(&dist));
    for (path, kind, file) in [
        (
            "/console/app.js",
            "text/javascript; charset=utf-8",
            "app.js",
        ),
        (
            "/console/worker.js",
            "text/javascript; charset=utf-8",
            "worker.js",
        ),
        (
            "/console/dspi_wasm.wasm",
            "application/wasm",
            "dspi_wasm.wasm",
        ),
        (
            "/console/dspi-web.css",
            "text/css; charset=utf-8",
            "dspi-web.css",
        ),
        (
            "/console/THIRD-PARTY-NOTICES.txt",
            "text/plain; charset=utf-8",
            "THIRD-PARTY-NOTICES.txt",
        ),
        (
            "/console/test/usb.html",
            "text/html; charset=utf-8",
            "test/usb.html",
        ),
        (
            "/console/app.js?v=2",
            "text/javascript; charset=utf-8",
            "app.js",
        ),
    ] {
        let (code, head, body) = get(port, path);
        assert_eq!(code, 200, "{path}");
        assert_eq!(header(&head, "content-type"), Some(kind), "{path}");
        assert_eq!(header(&head, "x-content-type-options"), Some("nosniff"));
        assert_eq!(header(&head, "cache-control"), Some("no-cache"));
        assert!(header(&head, "etag").is_some(), "{path}");
        assert!(header(&head, "content-security-policy").is_some(), "{path}");
        assert_eq!(body, std::fs::read(dist.join(file)).unwrap(), "{path}");
    }
    // The hub's own page at `/` is the page it always was, byte for byte.
    let (code, _, body) = get(port, "/");
    assert_eq!(code, 200);
    assert_eq!(body, web::asset("/").unwrap().body);
    hub.stop();
}

#[test]
fn wp_wb7_the_page_names_its_hub_and_the_policy_this_hubs_socket() {
    let (hub, port) = start(true, Some(&fake_dist()));
    for path in ["/console/", "/console/index.html", "/console/?readonly"] {
        let (code, head, body) = get(port, path);
        assert_eq!(code, 200, "{path}");
        let page = String::from_utf8(body).unwrap();
        assert!(
            page.contains(&format!(
                r#"<meta name="dspi-hub" content="ws://127.0.0.1:{port}" />"#
            )),
            "{page}"
        );
        assert!(
            page.contains(r#"<meta name="dspi-hub-page" content="/" />"#),
            "{page}"
        );
        assert_eq!(
            header(&head, "etag"),
            None,
            "a filled page has no fixed validator"
        );
        let csp = header(&head, "content-security-policy").unwrap();
        assert!(
            csp.contains(&format!("connect-src 'self' ws://127.0.0.1:{port};")),
            "{csp}"
        );
        assert!(csp.contains("'wasm-unsafe-eval'"), "{csp}");
    }
    // Asked by another of its names, the page names that one.
    let (_, _, body) = request(port, "/console/", &format!("Host: localhost:{port}\r\n"));
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains(&format!("ws://localhost:{port}"))
    );
    // A name that is not the hub's (a rebinding site, a typo) goes nowhere into the page.
    for host in ["evil.example", "127.0.0.1:1", "x\" onload=\"y"] {
        let (code, head, body) = request(port, "/console/", &format!("Host: {host}\r\n"));
        assert_eq!(code, 200, "{host}");
        let page = String::from_utf8(body).unwrap();
        assert!(
            page.contains(r#"<meta name="dspi-hub" content="" />"#),
            "{host}: {page}"
        );
        let csp = header(&head, "content-security-policy").unwrap();
        assert!(csp.contains("connect-src 'self';"), "{host}: {csp}");
    }
    let (code, _, _) = request(port, "/console/", "");
    assert_eq!(code, 200, "no Host at all");
    hub.stop();
}

#[test]
fn wp_wb7_no_listing_no_escape_and_a_redirect_to_the_slash() {
    let (hub, port) = start(true, Some(&fake_dist()));
    let (code, head, _) = get(port, "/console");
    assert_eq!(code, 301);
    assert_eq!(header(&head, "location"), Some("/console/"));
    for path in [
        "/console/test/",
        "/console/test",
        "/console/.env",
        "/console/notes.md",
        "/console/run.sh",
        "/console/link.txt",
        "/console/a%20b.js",
        "/console/../Cargo.toml",
        "/console/..%2fCargo.toml",
        "/console//app.js",
        "/console/APP.JS",
        "/console/nothing.js",
    ] {
        assert_eq!(get(port, path).0, 404, "{path}");
    }
    hub.stop();
}

#[test]
fn wp_wb7_unchanged_files_answer_304() {
    let (hub, port) = start(true, Some(&fake_dist()));
    let (_, head, _) = get(port, "/console/dspi_wasm.wasm");
    let etag = header(&head, "etag").unwrap().to_owned();
    let ask = |tag: &str| {
        request(
            port,
            "/console/dspi_wasm.wasm",
            &format!("Host: 127.0.0.1:{port}\r\nIf-None-Match: {tag}\r\n"),
        )
    };
    let (code, head, body) = ask(&etag);
    assert_eq!(code, 304);
    assert!(body.is_empty());
    assert_eq!(header(&head, "etag"), Some(etag.as_str()));
    assert_eq!(ask(&format!("\"other\", {etag}")).0, 304);
    assert_eq!(ask("\"other\"").0, 200);
    hub.stop();
}

#[test]
fn wp_wb7_nothing_with_web_off_and_404_without_a_console() {
    let (hub, port) = start(false, Some(&fake_dist()));
    for path in ["/console/", "/console/app.js", "/"] {
        assert_eq!(get(port, path).0, 404, "{path}");
    }
    hub.stop();
    let (hub, port) = start(true, None);
    assert_eq!(get(port, "/console/").0, 404);
    assert_eq!(
        get(port, "/").0,
        200,
        "the hub's page is there all the same"
    );
    hub.stop();
}
