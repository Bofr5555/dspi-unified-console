//! Bonjour (spec 3.1): `_dspi._tcp` through CG2's responder, on only while sharing.
//! Owned by WP HB5.
//!
//! `dspi_link::discovery::MdnsAdvertiser` already does the work, behind this crate's
//! feature `mdns`. Stopping sends a goodbye and can take a second, so the hub stops
//! it on the thread that called `stop`, never on a listener's or a worker's.

use dspi_link::discovery::HubAdvert;

use crate::error::HubError;

/// A running advertisement.
#[derive(Debug)]
pub struct Advertiser {}

impl Advertiser {
    /// Start advertising.
    pub fn start(advert: &HubAdvert) -> Result<Self, HubError> {
        let _ = advert;
        todo!("WP HB5")
    }

    /// Announce a change: devices came or went, a new name.
    pub fn update(&mut self, advert: &HubAdvert) -> Result<(), HubError> {
        let _ = advert;
        todo!("WP HB5")
    }
}
