//! Finding DSPi Link hubs on the network: the desktop clients' browser.
//!
//! Owned by WP CG2 (docs/core-merge/PLAN-CLIENTS.md), over
//! [`dspi_link::discovery`]. Behind feature `mdns`, which the mobile libraries
//! leave off: Android and iOS browse with their system APIs, which own the
//! permissions those systems ask for, and tell the core about a new address
//! with [`DspiCore::link_set_url`]. So nothing here is exported over UniFFI.
//!
//! The browser's thread opens the network and calls the listener; no method
//! here touches the network or the `Inner` lock, except
//! [`DspiCore::link_browse_reresolve`], which takes that lock as
//! [`DspiCore::link_set_url`] does.
//!
//! # A hub that moves (F5)
//!
//! The core does not re-resolve by itself: only the client knows when it wants
//! the network looked at. A client keeps a browse running while a session is
//! [`crate::LinkPhase::Reconnecting`], and when its listener hears a change it
//! calls [`DspiCore::link_browse_reresolve`] with the session's hub id. The next
//! dial, at once if one is waiting, goes to the hub's address. The supervisor
//! still checks the hub id in the hello, so an address that has gone to another
//! hub gets no token.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

pub use dspi_link::discovery::{
    BrowseListener, BrowseOptions, BrowseSession, BrowseSnapshot, BrowseSource, BrowsedHub,
    DiscoveryError, HubBrowser, LanSource, ServiceRecord, SourceEvent, SourceSink,
};
// -- WP DISC2 -- kept for embedders that name it; not in a Windows build
#[cfg(not(windows))]
pub use dspi_link::discovery::MdnsSource;

use crate::{DspiCore, DspiError, Result};

/// The running browse, if any. A field of [`DspiCore`], taken without the `Inner`
/// lock. Its mutex is never held while stopping a browser or calling a listener.
#[derive(Default)]
pub(crate) struct BrowseSlot(Mutex<Option<HubBrowser>>);

impl BrowseSlot {
    fn lock(&self) -> MutexGuard<'_, Option<HubBrowser>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn with<T>(&self, f: impl FnOnce(&HubBrowser) -> T) -> Option<T> {
        self.lock().as_ref().map(f)
    }
}

impl DspiCore {
    /// Start looking for hubs on the local network, replacing any browse already
    /// running. `own_hub_id` is this app's own hub, which is not listed; "" for
    /// none.
    ///
    /// Returns at once. The network is opened on the browser's thread, and a
    /// failure there is a state, in [`BrowseSnapshot::error`], not an error
    /// here: browsing is tried again by itself, up to every 30 s.
    ///
    /// `listener` hears each change on that thread, with no core lock held. It
    /// schedules work and returns, or calls `link_browse_*` methods, including
    /// [`DspiCore::link_browse_stop`].
    ///
    /// Errors: [`DspiError::Io`] when a thread cannot be started.
    pub fn link_browse_start(
        &self,
        listener: Option<Arc<dyn BrowseListener>>,
        own_hub_id: String,
    ) -> Result<()> {
        // -- WP DISC2 -- Windows' own DNS-SD on Windows, the embedded daemon elsewhere.
        self.link_browse_start_with(Arc::new(LanSource), listener, own_hub_id)
    }

    /// [`DspiCore::link_browse_start`] from another source of services: a test's
    /// [`dspi_link::discovery::FakeAdvertiser`] (feature `fake-hub`).
    pub fn link_browse_start_with(
        &self,
        source: Arc<dyn BrowseSource>,
        listener: Option<Arc<dyn BrowseListener>>,
        own_hub_id: String,
    ) -> Result<()> {
        self.link_browse_stop();
        let options = BrowseOptions {
            own_hub_id,
            ..BrowseOptions::default()
        };
        let browser = HubBrowser::start(source, listener, options)
            .map_err(|e| DspiError::Io { msg: e.to_string() })?;
        // A racing start's browser, if any, stops once the lock is released.
        let previous = self.browse.lock().replace(browser);
        drop(previous);
        Ok(())
    }

    /// Stop looking and forget the hubs. Waits for a listener call in progress,
    /// so none is in progress or to come afterwards, unless called from the
    /// listener itself. Nothing happens if no browse runs.
    pub fn link_browse_stop(&self) {
        let browser = self.browse.lock().take();
        drop(browser);
    }

    /// Where the browse stands; the default (not browsing, no hubs) with none
    /// running.
    pub fn link_browse_snapshot(&self) -> BrowseSnapshot {
        self.browse.with(HubBrowser::snapshot).unwrap_or_default()
    }

    /// The hub with this `hid` (any case) as last heard, with the address to dial
    /// now. `None` when it is not on the network or no browse runs.
    pub fn link_browse_resolve(&self, hub_id: String) -> Option<BrowsedHub> {
        self.browse.with(|b| b.resolve(&hub_id)).flatten()
    }

    /// Point the current session at the address its hub is announced at now, as
    /// [`DspiCore::link_set_url`] does; a session waiting to reconnect dials at
    /// once. Pass the id the session's hub had in its hello. False, with nothing
    /// changed, when the hub is not currently found. See the module docs.
    pub fn link_browse_reresolve(&self, hub_id: String) -> bool {
        let Some(hub) = self.link_browse_resolve(hub_id) else {
            return false;
        };
        self.link_set_url(hub.url);
        true
    }

    /// Ask the network whether this hub is still there. One that lost power sends
    /// no goodbye and otherwise stays listed for up to 75 minutes; one that does
    /// not answer now leaves the list within seconds, and the listener hears.
    /// For when dialling it fails. Returns at once.
    pub fn link_browse_verify(&self, hub_id: String) {
        self.browse.with(|b| b.verify(&hub_id));
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "link_browse/tests_wp_cg2.rs"]
mod tests_wp_cg2;
