//! What a hub serves a browser on its own port, only while sharing. Owned by WP HB5.
//! Its page's `Origin` is the hub itself, which the server's `Origin` check lets through
//! with `NetworkConfig::web` on, and nothing is served with it off.
//!
//! **Today this is one placeholder page of our own**, `assets/web/index.html`: enough to
//! show a browser reaches the hub. It is no control UI by itself, so an embedder should
//! leave `web` off unless it also serves the console below, since `web=1` in the TXT
//! record tells clients `GET /` serves a UI (spec 3.1). The plan's upstream DSPi-Link bundle is not included: whether to ship
//! it is the owner's question (HUB.md section 9, "HB5 as built", has its pin). Adding it
//! means its built files as entries of [`FILES`], and its licence in `NOTICE.md`.
//!
//! -- WP WB7 -- **The web console beside it, at [`CONSOLE_PATH`]** ([`Console`]): the
//! built `apps/web` dist, served as it is but for the `<meta name="dspi-hub">` the hub
//! fills in with its own Link address, so the page knows its hub. `/` stays the page
//! above, byte for byte (the owner's decision 1: the hub's page is not replaced). The
//! console's files are read **at run time** from a directory the embedder names, whole
//! and once, when the hub starts ([`Console::load`]), never on a request: a request's
//! path is only ever a key into that table, as for [`FILES`]. So the Rust build needs no
//! Node, and the console and the hub are built and versioned apart.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::Path;

use sha2::{Digest, Sha256};

/// One file served to a browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    /// Its `Content-Type`.
    pub content_type: &'static str,
    /// Its bytes.
    pub body: &'static [u8],
}

/// Every file by its request path, compiled in. Nothing is read from a disk, so a path
/// from a client is only ever a key here.
const FILES: &[(&str, Asset)] = &[(
    "/index.html",
    Asset {
        content_type: "text/html; charset=utf-8",
        body: include_bytes!("../assets/web/index.html"),
    },
)];

/// The asset at this request path, without its query. `None` is a 404. Never a path
/// under `/dspi/`, which is the protocol's (spec 4.2).
pub fn asset(path: &str) -> Option<Asset> {
    if path == "/dspi" || path.starts_with("/dspi/") {
        return None;
    }
    let path = if path == "/" { "/index.html" } else { path };
    FILES
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, asset)| *asset)
}

// -- WP WB7 -- the web console

/// Where the console is served, the trailing slash included: the page's own URLs are
/// relative to it.
pub const CONSOLE_PATH: &str = "/console/";

/// The `<meta>` of the console's `index.html` that names the hub serving it, exactly as
/// `apps/web/index.html` writes it. The hub fills in `content`.
pub const HUB_META: &str = r#"<meta name="dspi-hub" content="" />"#;
/// The `<meta>` that links the console to the hub's own page at `/`, filled in likewise.
pub const HUB_PAGE_META: &str = r#"<meta name="dspi-hub-page" content="" />"#;

/// Most files a console may have, and most bytes in all: a dist is a few files and
/// about 1.5 MB, so anything near these is the wrong directory.
pub const MAX_CONSOLE_FILES: usize = 512;
/// See [`MAX_CONSOLE_FILES`].
pub const MAX_CONSOLE_BYTES: usize = 64 << 20;
/// Deepest directory read below the console's root.
const MAX_DEPTH: usize = 4;

/// What is served, by extension; a file of any other kind is left out.
const TYPES: &[(&str, &str)] = &[
    ("html", "text/html; charset=utf-8"),
    ("js", "text/javascript; charset=utf-8"),
    ("mjs", "text/javascript; charset=utf-8"),
    ("css", "text/css; charset=utf-8"),
    ("wasm", "application/wasm"),
    ("json", "application/json"),
    ("map", "application/json"),
    ("txt", "text/plain; charset=utf-8"),
    ("svg", "image/svg+xml"),
    ("png", "image/png"),
    ("ico", "image/x-icon"),
    ("woff2", "font/woff2"),
];

/// Why a console directory could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConsoleError {
    /// The directory, or a file in it, cannot be read.
    #[error("cannot read the web console at {path}: {why}")]
    Unreadable {
        /// What could not be read.
        path: String,
        /// The OS's words.
        why: String,
    },
    /// No `index.html` at its root: not a built console.
    #[error("{0} has no index.html: not a built web console (apps/web: node tools/build.mjs)")]
    NoIndex(String),
    /// Past [`MAX_CONSOLE_FILES`] or [`MAX_CONSOLE_BYTES`].
    #[error("{0} is too big to be a web console")]
    TooBig(String),
}

/// One file of the console.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ConsoleFile {
    content_type: &'static str,
    body: Vec<u8>,
    /// A strong validator of the body; `None` for a page the hub fills in, which differs
    /// by the address it was asked at.
    etag: Option<String>,
    /// Whether the body holds [`HUB_META`] or [`HUB_PAGE_META`].
    filled: bool,
}

/// The built web console, in memory: its files by request path under [`CONSOLE_PATH`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Console {
    files: BTreeMap<String, ConsoleFile>,
    bytes: usize,
}

/// A console's answer to one request under [`CONSOLE_PATH`] (or to `/console`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleAnswer<'a> {
    /// `/console`, to [`CONSOLE_PATH`], where the page's relative URLs resolve.
    Redirect,
    /// The file, unchanged since the browser's copy (`If-None-Match`).
    NotModified {
        /// Its validator.
        etag: &'a str,
    },
    /// A file.
    File {
        /// Its `Content-Type`.
        content_type: &'static str,
        /// Its bytes, the hub's address filled in where the page asks for it.
        body: Cow<'a, [u8]>,
        /// Its validator, when it has a fixed one.
        etag: Option<&'a str>,
    },
    /// Nothing at that path. Directories are no keys, so there is no listing.
    NotFound,
}

/// Whether a request path is the console's.
pub fn is_console_path(path: &str) -> bool {
    path == "/console" || path.starts_with(CONSOLE_PATH)
}

/// A name a key may hold: what a build writes, nothing a browser would escape or a
/// file system would read as more than a name.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn content_type(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    TYPES.iter().find(|(e, _)| *e == ext).map(|(_, t)| *t)
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

impl Console {
    /// Read a built console from `dir`, whole. Hidden files, symbolic links, names with
    /// characters a build does not write, kinds not in the table and anything deeper
    /// than four directories are left out; an `index.html` at the root is required.
    pub fn load(dir: &Path) -> Result<Self, ConsoleError> {
        let mut console = Self {
            files: BTreeMap::new(),
            bytes: 0,
        };
        console.read_dir(dir, dir, "", 0)?;
        if !console.files.contains_key(CONSOLE_PATH) {
            return Err(ConsoleError::NoIndex(dir.display().to_string()));
        }
        Ok(console)
    }

    fn read_dir(
        &mut self,
        root: &Path,
        dir: &Path,
        prefix: &str,
        depth: usize,
    ) -> Result<(), ConsoleError> {
        let unreadable = |path: &Path, e: std::io::Error| ConsoleError::Unreadable {
            path: path.display().to_string(),
            why: e.to_string(),
        };
        let mut entries = std::fs::read_dir(dir)
            .map_err(|e| unreadable(dir, e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| unreadable(dir, e))?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !plain_name(&name) {
                continue;
            }
            let path = entry.path();
            // Not followed: a link could lead anywhere on the machine.
            let kind = std::fs::symlink_metadata(&path).map_err(|e| unreadable(&path, e))?;
            if kind.is_dir() {
                if depth < MAX_DEPTH {
                    self.read_dir(
                        root,
                        &path,
                        &format!("{prefix}{name}/"),
                        depth.saturating_add(1),
                    )?;
                }
                continue;
            }
            let Some(content_type) = kind.is_file().then(|| content_type(&name)).flatten() else {
                continue;
            };
            let too_big = || ConsoleError::TooBig(root.display().to_string());
            if self.files.len() >= MAX_CONSOLE_FILES {
                return Err(too_big());
            }
            let body = std::fs::read(&path).map_err(|e| unreadable(&path, e))?;
            self.bytes = self
                .bytes
                .checked_add(body.len())
                .filter(|&b| b <= MAX_CONSOLE_BYTES)
                .ok_or_else(too_big)?;
            let filled = content_type.starts_with("text/html")
                && (contains(&body, HUB_META) || contains(&body, HUB_PAGE_META));
            let etag = (!filled).then(|| {
                let hash = Sha256::digest(&body);
                format!(
                    "\"{}\"",
                    data_encoding::HEXLOWER.encode(hash.get(..12).unwrap_or_default())
                )
            });
            let key = match format!("{prefix}{name}").as_str() {
                "index.html" => CONSOLE_PATH.to_owned(),
                rel => format!("{CONSOLE_PATH}{rel}"),
            };
            self.files.insert(
                key,
                ConsoleFile {
                    content_type,
                    body,
                    etag,
                    filled,
                },
            );
        }
        Ok(())
    }

    /// How many files and bytes it holds.
    pub fn size(&self) -> (usize, usize) {
        (self.files.len(), self.bytes)
    }

    /// The request paths it answers, in order.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// The answer at `path` (without its query). `hub` is the address the page was
    /// asked at, `host[:port]` as the browser wrote it, and only when the server has
    /// found it to be the hub's own (the `Origin` rule's names): the page's
    /// `<meta name="dspi-hub">` then names `ws://` that, and is left empty otherwise.
    /// `if_none_match` is the browser's validator, if any.
    pub fn answer<'a>(
        &'a self,
        path: &str,
        hub: Option<&str>,
        if_none_match: Option<&str>,
    ) -> ConsoleAnswer<'a> {
        if path == "/console" {
            return ConsoleAnswer::Redirect;
        }
        let key = path.strip_suffix("index.html").unwrap_or(path);
        let Some(file) = self.files.get(key).or_else(|| self.files.get(path)) else {
            return ConsoleAnswer::NotFound;
        };
        if let (Some(etag), Some(wanted)) = (&file.etag, if_none_match)
            && wanted.split(',').any(|w| w.trim() == etag)
        {
            return ConsoleAnswer::NotModified { etag };
        }
        let body = if file.filled {
            Cow::Owned(fill(&file.body, hub))
        } else {
            Cow::Borrowed(file.body.as_slice())
        };
        ConsoleAnswer::File {
            content_type: file.content_type,
            body,
            etag: file.etag.as_deref(),
        }
    }
}

/// An address the server let through, checked again: it goes into an HTML attribute.
fn attribute_safe(hub: &str) -> bool {
    !hub.is_empty()
        && hub
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'))
}

fn fill(body: &[u8], hub: Option<&str>) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(body) else {
        return body.to_vec();
    };
    let address = hub.filter(|h| attribute_safe(h));
    let text = match address {
        Some(hub) => text.replacen(
            HUB_META,
            &format!(r#"<meta name="dspi-hub" content="ws://{hub}" />"#),
            1,
        ),
        None => text.to_owned(),
    };
    text.replacen(
        HUB_PAGE_META,
        r#"<meta name="dspi-hub-page" content="/" />"#,
        1,
    )
    .into_bytes()
}

/// The `Content-Security-Policy` of every console response: its own files only, wasm
/// compiled (`wasm-unsafe-eval`, which JSPI's module needs and which allows no `eval`),
/// and sockets to this hub alone. `hub` as for [`Console::answer`].
pub fn console_csp(hub: Option<&str>) -> String {
    let socket = hub
        .filter(|h| attribute_safe(h))
        .map(|h| format!(" ws://{h}"))
        .unwrap_or_default();
    format!(
        "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; \
         connect-src 'self'{socket}; style-src 'self'; img-src 'self' data:; font-src 'self'; \
         base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    )
}

#[cfg(test)]
mod tests_wp_hb5 {
    use super::*;

    #[test]
    fn wp_hb5_the_root_is_the_page() {
        let page = asset("/").unwrap();
        assert_eq!(Some(page), asset("/index.html"));
        assert!(page.content_type.starts_with("text/html"));
        assert!(!page.body.is_empty());
    }

    #[test]
    fn wp_hb5_only_what_is_listed_is_served() {
        for path in [
            "",
            "index.html",
            "//",
            "/../index.html",
            "/./index.html",
            "//index.html",
            "/index.html/",
            "/INDEX.HTML",
            "/%69ndex.html",
            "/etc/passwd",
            "/../../Cargo.toml",
            "/dspi",
            "/dspi/",
            "/dspi/v1",
            "/dspi/v1/info",
            "/dspi/index.html",
        ] {
            assert_eq!(asset(path), None, "{path}");
        }
    }

    #[test]
    fn wp_hb5_no_file_is_under_the_protocols_path() {
        assert!(FILES.iter().all(|(name, _)| !name.starts_with("/dspi")));
        assert!(FILES.iter().all(|(name, _)| name.starts_with('/')));
    }

    #[test]
    fn wp_hb5_the_placeholder_is_self_contained_and_treats_names_as_text() {
        let page = std::str::from_utf8(asset("/").unwrap().body).unwrap();
        for forbidden in [
            "innerHTML",
            "outerHTML",
            "document.write",
            "https://",
            "src=",
        ] {
            assert!(!page.contains(forbidden), "{forbidden}");
        }
    }
}

#[cfg(test)]
mod tests_wp_wb7 {
    use super::*;

    #[test]
    fn wp_wb7_console_paths() {
        for path in [
            "/console",
            "/console/",
            "/console/app.js",
            "/console/test/usb.html",
        ] {
            assert!(is_console_path(path), "{path}");
        }
        for path in ["/", "/consoles", "/console.html", "/index.html", "/dspi/v1"] {
            assert!(!is_console_path(path), "{path}");
        }
    }

    #[test]
    fn wp_wb7_only_plain_names_and_known_kinds() {
        for name in [
            "app.js",
            "dspi_wasm.wasm",
            "dspi-web.css",
            "THIRD-PARTY-NOTICES.txt",
        ] {
            assert!(plain_name(name) && content_type(name).is_some(), "{name}");
        }
        for name in ["", ".hidden", "..", "a b.js", "a%2f.js", "ä.js", "x\\y.js"] {
            assert!(!plain_name(name), "{name}");
        }
        assert_eq!(content_type("dspi_wasm.wasm"), Some("application/wasm"));
        assert_eq!(
            content_type("app.js"),
            Some("text/javascript; charset=utf-8")
        );
        assert_eq!(content_type("index.HTML"), Some("text/html; charset=utf-8"));
        assert_eq!(content_type("run.sh"), None);
        assert_eq!(content_type("Makefile"), None);
    }

    #[test]
    fn wp_wb7_the_page_is_told_its_hub_and_the_hubs_page() {
        let page = format!("<head>{HUB_META}{HUB_PAGE_META}</head>");
        let filled = String::from_utf8(fill(page.as_bytes(), Some("127.0.0.1:11916"))).unwrap();
        assert_eq!(
            filled,
            r#"<head><meta name="dspi-hub" content="ws://127.0.0.1:11916" /><meta name="dspi-hub-page" content="/" /></head>"#
        );
        let v6 = String::from_utf8(fill(page.as_bytes(), Some("[::1]:80"))).unwrap();
        assert!(v6.contains(r#"content="ws://[::1]:80""#), "{v6}");
        // An address that is not one stays out of the page: the meta stays empty.
        for bad in [
            None,
            Some(""),
            Some(r#"x" onload="alert(1)"#),
            Some("a/b"),
            Some("a b"),
        ] {
            let kept = String::from_utf8(fill(page.as_bytes(), bad)).unwrap();
            assert!(kept.contains(HUB_META), "{bad:?}");
            assert!(!kept.contains("alert"), "{bad:?}");
        }
    }

    #[test]
    fn wp_wb7_the_policy_allows_wasm_and_this_hubs_socket_only() {
        let csp = console_csp(Some("hub.local:11915"));
        assert!(
            csp.contains("script-src 'self' 'wasm-unsafe-eval';"),
            "{csp}"
        );
        assert!(
            csp.contains("connect-src 'self' ws://hub.local:11915;"),
            "{csp}"
        );
        assert!(csp.starts_with("default-src 'none';"), "{csp}");
        assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
        for word in ["unsafe-inline", "'unsafe-eval'", "*", "wss:", "http:"] {
            assert!(!csp.contains(word), "{word} in {csp}");
        }
        assert!(console_csp(None).contains("connect-src 'self';"));
        assert!(console_csp(Some("x; script-src *")).contains("connect-src 'self';"));
    }
}
