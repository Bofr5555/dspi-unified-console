//! A hub's DNS-SD TXT record, read without the `mdns` feature (WP CG-SWS-a),
//! for the clients that browse with their system's API (iOS `NWBrowser`,
//! Android `NsdManager`) and hand the core what they heard.
//!
//! The rules are `dspi_link::discovery::parse_service`'s, the one reader of
//! spec 3.1's keys that the desktop browser uses too: `v` and `hid` required,
//! `tls=1` refused, unknown keys ignored, and a `kind`, `auth`, `n` or `path`
//! that makes no sense falls back rather than losing the hub. Keys ignore case.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::net::{IpAddr, Ipv4Addr};

use dspi_link::discovery::{ServiceRecord, parse_service};

use crate::records::{LinkTxtEntry, LinkTxtRecord};
use crate::{DspiError, Result};

/// A placeholder the parser needs to accept a record before any address is
/// resolved (TEST-NET-1, RFC 5737); it never leaves this module.
const NO_ADDRESS: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));

/// Read a hub's TXT record. `service_name` is the DNS-SD instance name (with
/// or without `._dspi._tcp.local.`); `txt` its keys and values in record
/// order (of a key repeated, the first counts, RFC 6763 6.4). Where the
/// hub is comes from resolving the service, not from here: dial it with
/// [`crate::link_ws_url`] and `path`.
///
/// Differs from the Mac's `HubBrowser.hub(fromTXT:)`
/// (`Link/HubBrowser.swift:583-597`) by the spec and Android's `HubTxt`: a
/// record without `v` or `hid`, or with `tls=1`, is no hub (the Mac made up a
/// `service:<name>` id and listed it), and a missing `kind` reads `unknown`
/// (the Mac's `console`).
///
/// Errors: [`DspiError::InvalidArgument`] saying why the record is no hub.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn link_parse_txt(service_name: String, txt: Vec<LinkTxtEntry>) -> Result<LinkTxtRecord> {
    let entries = txt.into_iter().map(|e| (e.key, e.value)).collect();
    let record = ServiceRecord {
        instance: service_name,
        port: 1,
        addresses: vec![NO_ADDRESS],
        txt: entries,
        host_name: String::new(),
    };
    let hub = parse_service(&record).map_err(|e| DspiError::InvalidArgument {
        msg: format!("not a hub's TXT record: {e}"),
    })?;
    Ok(LinkTxtRecord {
        hub_id: hub.id,
        name: hub.name,
        kind: hub.kind,
        auth: hub.auth,
        device_count: hub.device_count,
        serials: hub.serials,
        path: hub.path,
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "link_txt/tests_wp_cgswsa.rs"]
mod tests_wp_cgswsa;
