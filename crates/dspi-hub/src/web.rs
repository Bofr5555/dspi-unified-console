//! The MIT DSPi-Link web bundle, at a pinned tag and hash, served on the hub's port
//! only while sharing. Owned by WP HB5. Its page's `Origin` is the hub itself, which
//! the server's `Origin` check lets through.

/// One file of the bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    /// Its `Content-Type`.
    pub content_type: &'static str,
    /// Its bytes.
    pub body: &'static [u8],
}

/// The asset at this request path. `None` is a 404. Never a path under `/dspi/`,
/// which is the protocol's (spec 4.2).
pub fn asset(path: &str) -> Option<Asset> {
    let _ = path;
    todo!("WP HB5")
}
