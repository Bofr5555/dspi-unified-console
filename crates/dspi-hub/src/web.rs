//! What a hub serves a browser on its own port, only while sharing. Owned by WP HB5.
//! Its page's `Origin` is the hub itself, which the server's `Origin` check lets through
//! with `NetworkConfig::web` on, and nothing is served with it off.
//!
//! **Today this is one placeholder page of our own**, `assets/web/index.html`: enough to
//! show a browser reaches the hub. It is no control UI, so an embedder should leave `web`
//! off until there is one, since `web=1` in the TXT record tells clients `GET /` serves
//! a UI (spec 3.1). The plan's upstream DSPi-Link bundle is not included: whether to ship
//! it is the owner's question (HUB.md section 9, "HB5 as built", has its pin). Adding it
//! means its built files as entries of [`FILES`], and its licence in `NOTICE.md`.

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
