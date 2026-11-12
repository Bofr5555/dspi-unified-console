//! Bonjour (spec 3.1): `_dspi._tcp` through CG2's responder, on only while sharing.
//! Owned by WP HB5.
//!
//! `dspi_link::discovery::LanAdvertiser` does the work, behind this crate's feature
//! `mdns`: CG2's `MdnsAdvertiser`, or on Windows `DnsSdAdvertiser` (WP DISC2), Windows'
//! own DNS-SD service, which needs no firewall rule to advertise. What follows about
//! conflicts and updates is about the former; the latter's are in its own docs. Stopping sends a goodbye and can take a second, so the hub stops it on the
//! thread that called `stop`, never on a listener's or a worker's.
//!
//! Two limits of what is below this are kept here, as a hub is what can break them:
//! - A DNS label is at most 63 bytes, and a conflict adds ` (2)` to the instance name.
//! - One TXT string is at most 255 bytes. The spec's 400 for the record does not save a
//!   long `d` list, and the responder refuses the whole service rather than cut it.
//!   Past that the list is left out, as the spec does past its own limit: `d` is optional.
//!
//! **Name conflicts.** Hearing another machine hold the instance name, the responder takes
//! `Studio (2)` for itself, but mdns-sd 0.21.7 does not carry that through for a name with
//! capitals, which is most of them. It looks its renames up in lower case and stores them
//! as given, so:
//! - It keeps answering for `Studio`, with this hub's `hid` and port, so a browser can
//!   read the other hub's name as this one until that hub next speaks. A client that
//!   dials by `hid` then finds out at `hello` (`expected_hub_id`, CG1).
//! - It does not defend `Studio (2)`, so a third hub takes that too.
//!
//! So [`Advertiser`] takes the event only as news of a conflict, and registers anew under
//! a name it makes: a plain registration, as if that had been the name asked for, which
//! has neither fault. `hello` keeps the configured name. A client knows a hub by its `hid`,
//! so a new instance name is the same hub (`wp_cg2_a_renamed_hub_is_one_hub`). What is
//! left is the moment between the responder's rename and the next [`Advertiser::names`],
//! which is why the hub polls quickly while a conflict can arise ([`Advertiser::poll_wait`]).
//!
//! **Dots and backslashes** are replaced in an instance name. The responder escapes them
//! when it writes and compares a name, but not when it reads one, so it would never see
//! a conflict over such a name: it would announce over the other hub.
//!
//! A host name conflict stays the responder's own (`dspi-0123456789ab-2.local`): those are
//! lower case with no dot, so they work. [`Advertiser::names`] has the current one.
//!
//! **Updates are paced**, at most one per [`MIN_ANNOUNCE_GAP`]. [`Advertiser::update`]
//! holds a change back until then, the latest replacing what was held, and
//! [`Advertiser::names`] sends it when due, on the hub's tick. So the count a client sees
//! before connecting can be a couple of seconds behind; `device.list` is not. Each record
//! carries `rev=`, which differs every time, since mdns-sd's browser keeps showing the
//! newer data when older data comes back (CG2's `MdnsAdvertiser` has the details).

use std::time::{Duration, Instant};

use dspi_link::discovery::{AdvertiseScope, AdvertisedNames, HubAdvert};
use dspi_link::sanitize_hub_text;

use crate::error::HubError;

/// Longest instance name asked for, in bytes: a DNS label's 63, less room for ` (99)`.
const MAX_INSTANCE_BYTES: usize = 58;
/// Longest value of `d`, in bytes: a TXT string's 255, less the key and `=`.
const MAX_SERIAL_LIST_BYTES: usize = 253;
/// The instance name of a hub whose own has nothing to show.
const UNNAMED: &str = "DSPi Hub";
/// The most a hub gives way: past this it stays as the responder left it, as something
/// answering for every name would otherwise have it re-registering for ever.
const MAX_CONFLICT_SUFFIX: u32 = 99;
/// The least time between two announcements. The responder sends nothing sooner than a
/// second after the same record (RFC 6762 section 6), and each announcement here differs
/// (`rev`), so this only keeps a flapping device from flooding the network.
const MIN_ANNOUNCE_GAP: Duration = Duration::from_secs(2);
/// How long after a registration a conflict can still come, and how often to look.
const CONFLICT_WINDOW: Duration = Duration::from_secs(3);
const CONFLICT_POLL: Duration = Duration::from_millis(20);

/// Whether this build can advertise at all (feature `mdns`).
pub const AVAILABLE: bool = cfg!(feature = "mdns");

// -- WP DISC2 -- Windows' own DNS-SD on Windows (`DnsSdAdvertiser`), which has no
// call to change a registration: an update is a goodbye and a new registration on its
// thread, and clients' removal grace hides the gap. Its conflict handling is below.
#[cfg(feature = "mdns")]
use dspi_link::discovery::LanAdvertiser as Responder;

/// A build without feature `mdns` has no responder, and so no [`Advertiser`] to call.
#[cfg(not(feature = "mdns"))]
struct Responder(std::convert::Infallible);

#[cfg(not(feature = "mdns"))]
impl Responder {
    fn start_on(
        _: &HubAdvert,
        _: AdvertiseScope,
    ) -> Result<Self, dspi_link::discovery::DiscoveryError> {
        Err(dspi_link::discovery::DiscoveryError::Network(
            "this build has no mDNS responder".to_owned(),
        ))
    }

    fn update(&mut self, _: &HubAdvert) -> Result<(), dspi_link::discovery::DiscoveryError> {
        match self.0 {}
    }

    fn names(&mut self) -> AdvertisedNames {
        match self.0 {}
    }
}

/// The instance name asked for, within a DNS label.
fn fit_name(name: &str) -> String {
    let name = sanitize_hub_text(name)
        .replace(char::REPLACEMENT_CHARACTER, " ")
        .replace('.', "-")
        .replace('\\', "/")
        .trim()
        .to_owned();
    let mut end = name.len().min(MAX_INSTANCE_BYTES);
    while !name.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let name = name.get(..end).unwrap_or_default().trim_end();
    if name.is_empty() { UNNAMED } else { name }.to_owned()
}

/// `d` within what one TXT string can carry. TXT values are ASCII (spec 3.1) and `d`
/// splits on commas, and a serial is the device's own say-so: one that breaks either is
/// left out of a list that is optional.
fn fit_serials(serials: &[String]) -> Vec<String> {
    let usable = |s: &&String| s.bytes().all(|b| b.is_ascii_graphic() && b != b',');
    let serials: Vec<String> = serials.iter().filter(usable).cloned().collect();
    if serials.join(",").len() <= MAX_SERIAL_LIST_BYTES {
        serials
    } else {
        Vec::new()
    }
}

/// A running advertisement. Dropping it sends the goodbye.
pub struct Advertiser {
    responder: Responder,
    /// As given, so the same data twice announces nothing.
    current: HubAdvert,
    /// The instance name to register: what was asked for, or after a conflict one of ours.
    held: String,
    /// The instance name last registered, which is what a conflict is judged against:
    /// `held` is ahead of it while a new name waits its turn.
    registered: String,
    /// The suffix the next conflict takes.
    next_suffix: u32,
    /// `current` has not gone out yet.
    pending: bool,
    last_sent: Instant,
    /// What was last registered, under `registered`.
    record: HubAdvert,
}

impl std::fmt::Debug for Advertiser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Advertiser")
            .field("current", &self.current)
            .field("held", &self.held)
            .finish_non_exhaustive()
    }
}

impl Advertiser {
    /// Start advertising in `scope`. Blocks briefly, for the responder's sockets.
    pub fn start(advert: &HubAdvert, scope: AdvertiseScope) -> Result<Self, HubError> {
        let held = fit_name(&advert.name);
        let record = HubAdvert {
            name: held.clone(),
            serials: fit_serials(&advert.serials),
            ..advert.clone()
        };
        let responder = Responder::start_on(&record, scope)
            .map_err(|e| HubError::Io(format!("cannot advertise: {e}")))?;
        Ok(Self {
            responder,
            record,
            current: advert.clone(),
            registered: held.clone(),
            held,
            next_suffix: 2,
            pending: false,
            last_sent: Instant::now(),
        })
    }

    /// Announce a change: devices came or went, a new name. Nothing goes out if nothing
    /// in `advert` changed, since a hub publishes far more often than its record changes
    /// (a lock, a device's name), and it may go out later (the module's header). A new
    /// name sends a goodbye for the old one, which can take a second.
    pub fn update(&mut self, advert: &HubAdvert) -> Result<(), HubError> {
        if *advert == self.current {
            return Ok(());
        }
        if advert.name != self.current.name {
            self.held = fit_name(&advert.name);
            self.next_suffix = 2;
        }
        advert.clone_into(&mut self.current);
        self.pending = true;
        self.send_if_due()
    }

    fn send_if_due(&mut self) -> Result<(), HubError> {
        if self.pending && self.last_sent.elapsed() >= MIN_ANNOUNCE_GAP {
            self.send()?;
        }
        Ok(())
    }

    fn send(&mut self) -> Result<(), HubError> {
        let record = HubAdvert {
            name: self.held.clone(),
            serials: fit_serials(&self.current.serials),
            ..self.current.clone()
        };
        self.pending = false;
        self.register(record)
    }

    fn register(&mut self, record: HubAdvert) -> Result<(), HubError> {
        self.responder
            .update(&record)
            .map_err(|e| HubError::Io(format!("cannot advertise: {e}")))?;
        // A new name goes out after the old one's goodbye, which took the host's address
        // records with it, and the responder sends nothing twice within a second: so the
        // announcement at the end of its probe can lack them, and a browser cannot resolve
        // it until it asks. Say it again, once that second has passed.
        if record.name != self.record.name {
            self.pending = true;
        }
        self.held.clone_into(&mut self.registered);
        self.record = record;
        self.last_sent = Instant::now();
        Ok(())
    }

    /// How soon [`Self::names`] should be called again, if sooner than a regular tick:
    /// while a conflict could be arising from the latest registration.
    pub fn poll_wait(&self) -> Option<Duration> {
        (self.last_sent.elapsed() < CONFLICT_WINDOW).then_some(CONFLICT_POLL)
    }

    /// What was last asked for.
    pub fn current(&self) -> &HubAdvert {
        &self.current
    }

    /// The names held now, having dealt with any conflict the responder has reported
    /// and sent any change that has come due. Call it about once a second or more. Can
    /// take a second when a conflict needs a goodbye.
    pub fn names(&mut self) -> AdvertisedNames {
        let mut names = self.responder.names();
        if names.instance != self.registered && self.next_suffix <= MAX_CONFLICT_SUFFIX {
            let base = fit_name(&self.current.name);
            self.held = format!("{base} ({})", self.next_suffix);
            self.next_suffix = self.next_suffix.saturating_add(1);
            log::info!(
                "hub: name taken on the network, now advertised as {}",
                self.held
            );
            // The data the responder has just been sending under its own new name: other
            // data under it would be a conflict with itself.
            let record = HubAdvert {
                name: self.held.clone(),
                ..self.record.clone()
            };
            if let Err(e) = self.register(record) {
                log::warn!("hub: {e}");
                self.pending = true;
            }
            names = self.responder.names();
        } else if let Err(e) = self.send_if_due() {
            log::warn!("hub: {e}");
        }
        names
    }
}

#[cfg(test)]
mod tests_wp_hb5 {
    use super::*;

    #[test]
    fn wp_hb5_an_instance_name_fits_a_dns_label_with_room_for_a_conflict() {
        let long = fit_name(&"å".repeat(100));
        assert!(long.len() <= MAX_INSTANCE_BYTES && long.len().is_multiple_of(2));
        assert!(long.chars().all(|c| c == 'å'), "cut on a character: {long}");
        assert_eq!(fit_name("Studio Mac"), "Studio Mac");
    }

    #[test]
    fn wp_hb5_a_name_with_nothing_to_show_gets_one() {
        assert_eq!(fit_name(""), UNNAMED);
        assert_eq!(fit_name(" \n\u{1b}\t "), UNNAMED);
        assert_eq!(fit_name("a\nb"), "a b");
    }

    #[test]
    fn wp_hb5_what_the_responder_cannot_compare_is_replaced() {
        assert_eq!(fit_name("Studio 1.2"), "Studio 1-2");
        assert_eq!(fit_name("back\\slash"), "back/slash");
    }

    #[test]
    fn wp_hb5_a_serial_list_past_one_txt_string_is_left_out() {
        let serial = |i: usize| format!("{i:016X}");
        let of = |n: usize| (0..n).map(serial).collect::<Vec<_>>();
        // 15 serials are 254 bytes with their commas; 14 are 237.
        assert_eq!(fit_serials(&of(14)).len(), 14);
        assert!(fit_serials(&of(15)).is_empty());
    }

    #[test]
    fn wp_hb5_a_serial_that_would_break_the_list_is_left_out_of_it() {
        let serials = ["GOOD", "a,b", "sp ace", "åäö", "\u{1b}[2J"].map(str::to_owned);
        assert_eq!(fit_serials(&serials), ["GOOD"]);
    }
}
