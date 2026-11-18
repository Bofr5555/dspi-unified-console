//! The generation wrapper: every connection is opened through [`open`]
//! (WP PGWIRE, PLAN-PG sections 1.1 and 1.2).
//!
//! **The layering rule:** raw wire (USB, USB MIDI, DSPi Link, WebUSB, a fake,
//! a host's adapter), then the generation wrapper [`open`] made, then every
//! other wrapper. [`open`] is called once on the raw transport, before
//! anything else is sent, at every place a connection is built; the probes
//! of `dspi_session` then run unchanged on what it returns.
//!
//! **The probe** (section 1.1 steps 1 to 4), only on a link that carries a
//! page ([`Transport::carries_pages`]):
//!
//! 1. `SYS.PROTOCOL_INFO` (GET `0x0110`, 16 bytes; SETUP `C1 10 00 00 02 01 10
//!    00`). A timeout is retried [`PROBE_RETRIES`] times [`PROBE_BACKOFF`]
//!    apart, then the open fails with that timeout: never a guess.
//! 2. A paged answer: the device is paged. Until WP PGADAPT a paged device is
//!    refused with [`Refusal::PagedNotYet`] and nothing else is sent.
//! 3. A later major ([`Refusal::NewerProtocol`]) or an answer that is not a
//!    protocol-info block ([`Refusal::Unrecognised`]): refused, nothing else
//!    is sent.
//! 4. A STALL (USB) or status 0x02 (MIDI): a legacy candidate. `GET_PLATFORM`
//!    must answer; then the device is legacy and [`LegacyGuard`] wraps it,
//!    otherwise it is [`Refusal::Unrecognised`].
//!
//! A link that carries no page (DSPi Link until WP PGLINKC, a host-supplied
//! adapter) is legacy without a probe: nothing paged can reach the device.
//!
//! The generation belongs to the connection ([`Transport::generation`]); it is
//! decided again on every [`open`].

use std::time::Duration;

use dspi_proto::generated::opcodes as op;
use dspi_proto::paged::{PROTOCOL_INFO_LEN, ProbeReply, ProtocolInfo, SYS_PROTOCOL_INFO};

pub use dspi_proto::paged::Generation;

use crate::{
    BUSY_BACKOFF, CmdId, DeviceDescriptor, InRead, NotificationSource, Result, Transport,
    TransportCaps, TransportError, with_busy_retry,
};

/// How many times a probe that timed out is sent again (PLAN-PG section 1.1
/// step 1: a flash write may blind the device for a moment).
pub const PROBE_RETRIES: usize = 3;

/// The pause before each probe retry.
pub const PROBE_BACKOFF: Duration = BUSY_BACKOFF;

/// `GET_PLATFORM`'s length asked by the legacy check: 8 bytes, the longest
/// reply any firmware gives (PLAN-PG section 1.9); older ones answer 4, 6 or 7.
pub const PLATFORM_ASK: u16 = 8;

/// What a [`LegacyGuard`] answers to a paged call.
pub const LEGACY_REFUSES_PAGES: &str =
    "this device speaks the 1.1.6 protocol: no firmware 1.1.7 (paged) request is sent to it";

/// Why [`open`] refused a device. Each is a [`TransportError::Incompatible`]
/// whose words are [`Refusal::text`]; [`Refusal::of`] gives it back from the
/// error, and [`Refusal::code`] is its notice's code (`transfer.<code>` in
/// `assets/notices/notices.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The device answered the probe as paged firmware (1.1.7), which this
    /// build cannot control yet (until WP PGADAPT).
    PagedNotYet,
    /// The device's protocol major is newer than this build's.
    NewerProtocol,
    /// The device answered neither as paged firmware nor as 1.1.6 firmware.
    Unrecognised,
}

impl Refusal {
    /// Every refusal.
    pub const ALL: [Refusal; 3] = [
        Refusal::PagedNotYet,
        Refusal::NewerProtocol,
        Refusal::Unrecognised,
    ];

    /// The notice's code: the key is `transfer.<code>`.
    pub const fn code(self) -> &'static str {
        match self {
            Refusal::PagedNotYet => "paged_not_yet",
            Refusal::NewerProtocol => "newer_protocol",
            Refusal::Unrecognised => "unrecognised_device",
        }
    }

    /// The error's words.
    pub const fn text(self) -> &'static str {
        match self {
            Refusal::PagedNotYet => {
                "this device runs firmware 1.1.7, which this version cannot control yet"
            }
            Refusal::NewerProtocol => {
                "this device's firmware uses a newer protocol; update this app"
            }
            Refusal::Unrecognised => "unrecognised device",
        }
    }

    /// The error [`open`] fails with.
    pub fn error(self) -> TransportError {
        TransportError::Incompatible(self.text().to_owned())
    }

    /// The refusal whose words are `text` exactly (an error's message, after
    /// a conversion that kept it).
    pub fn of_text(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.text() == text)
    }

    /// The refusal `e` is, if it is one.
    pub fn of(e: &TransportError) -> Option<Self> {
        match e {
            TransportError::Incompatible(text) => Self::of_text(text),
            _ => None,
        }
    }
}

/// Open a connection on `raw`, the raw wire, before anything else is sent:
/// probe the device's generation (module docs) and wrap it in the generation
/// wrapper. Every place that builds a connection calls this once, and every
/// other wrapper goes on top of what it returns.
///
/// Errors: a refusal ([`Refusal`], as [`TransportError::Incompatible`]); the
/// probe's timeout after [`PROBE_RETRIES`] retries; any other transport error
/// of the probe or of the legacy check (a disconnect, a USB error).
pub fn open(mut raw: Box<dyn Transport>) -> Result<Box<dyn Transport>> {
    if !raw.carries_pages() {
        return Ok(Box::new(LegacyGuard::new(raw)));
    }
    match probe(&mut *raw)? {
        Some(reply) => match ProtocolInfo::decode(&reply) {
            ProbeReply::Paged(info) => Ok(Box::new(PagedTransport::new(raw, info)?)),
            ProbeReply::UnknownMajor(major) => {
                log::warn!("the device's protocol major is {major}; refused");
                Err(Refusal::NewerProtocol.error())
            }
            ProbeReply::Garbled => Err(Refusal::Unrecognised.error()),
        },
        None => {
            let kind = raw.caps().kind;
            let platform = with_busy_retry(
                kind,
                || raw.control_in_upto(op::REQ_GET_PLATFORM, 0, PLATFORM_ASK),
                op::REQ_GET_PLATFORM,
            );
            match platform {
                Ok(reply) if !reply.is_empty() => Ok(Box::new(LegacyGuard::new(raw))),
                Ok(_) | Err(TransportError::Stalled { .. }) => Err(Refusal::Unrecognised.error()),
                Err(e) => Err(e),
            }
        }
    }
}

/// Send the probe: its reply, or `None` when the device refused it (a STALL
/// on USB, status 0x02 on MIDI). A timeout is retried; after
/// [`PROBE_RETRIES`] retries it is the error.
fn probe(raw: &mut dyn Transport) -> Result<Option<Vec<u8>>> {
    let mut retries = 0usize;
    loop {
        match raw.control_in_paged(SYS_PROTOCOL_INFO, 0, PROTOCOL_INFO_LEN) {
            Ok(reply) => return Ok(Some(reply)),
            Err(TransportError::Stalled { .. }) => return Ok(None),
            Err(TransportError::TimedOut { opcode, .. }) => {
                if retries >= PROBE_RETRIES {
                    return Err(TransportError::TimedOut { opcode, retries });
                }
                retries = retries.saturating_add(1);
                crate::clock::sleep(PROBE_BACKOFF);
            }
            Err(e) => return Err(e),
        }
    }
}

/// A 1.1.6-generation device (G1s, G1b, the Zero, the sender): page-0
/// requests pass, every paged call is refused here, with nothing on the wire.
/// Made only by [`open`].
pub struct LegacyGuard {
    inner: Box<dyn Transport>,
}

impl LegacyGuard {
    fn new(inner: Box<dyn Transport>) -> Self {
        Self { inner }
    }
}

fn refuse_paged() -> TransportError {
    TransportError::Incompatible(LEGACY_REFUSES_PAGES.into())
}

impl Transport for LegacyGuard {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        self.inner.control_in(opcode, value, len)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        self.inner.control_in_upto(opcode, value, max_len)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        self.inner.control_out(opcode, value, data)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }

    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.inner.notifications()
    }

    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }

    fn session_id(&self) -> Option<u16> {
        self.inner.session_id()
    }

    fn lock(&mut self, reason: &str, timeout: Duration) -> Result<()> {
        self.inner.lock(reason, timeout)
    }

    fn unlock(&mut self) {
        self.inner.unlock();
    }

    fn control_in_batch(&mut self, reads: &[InRead]) -> Vec<Result<Vec<u8>>> {
        self.inner.control_in_batch(reads)
    }

    /// No page is ever sent to a legacy device.
    fn carries_pages(&self) -> bool {
        false
    }

    fn control_in_paged(&mut self, _id: CmdId, _value: u16, _max_len: u16) -> Result<Vec<u8>> {
        Err(refuse_paged())
    }

    fn control_out_paged(&mut self, _id: CmdId, _value: u16, _data: &[u8]) -> Result<()> {
        Err(refuse_paged())
    }

    fn generation(&self) -> Option<Generation> {
        Some(Generation::Legacy)
    }
}

/// A paged device (G2s, G2b). **Placeholder until WP PGADAPT**, which makes
/// it the adapter (1.1.6-shaped calls planned into paged requests): today
/// [`PagedTransport::new`] refuses the device ([`Refusal::PagedNotYet`]), so
/// [`open`] sends nothing after the probe, and every call of the type refuses
/// too.
pub struct PagedTransport {
    inner: Box<dyn Transport>,
    info: ProtocolInfo,
}

impl PagedTransport {
    /// The paged connection on `inner`, whose probe answered `info`. Until
    /// WP PGADAPT: always [`Refusal::PagedNotYet`] (`inner` is dropped,
    /// closing the link, with nothing more sent).
    pub fn new(inner: Box<dyn Transport>, info: ProtocolInfo) -> Result<Self> {
        drop(Self { inner, info });
        Err(Refusal::PagedNotYet.error())
    }
}

impl Transport for PagedTransport {
    fn control_in(&mut self, _opcode: u8, _value: u16, _len: u16) -> Result<Vec<u8>> {
        Err(Refusal::PagedNotYet.error())
    }

    fn control_out(&mut self, _opcode: u8, _value: u16, _data: &[u8]) -> Result<()> {
        Err(Refusal::PagedNotYet.error())
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }

    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }

    fn control_in_paged(&mut self, _id: CmdId, _value: u16, _max_len: u16) -> Result<Vec<u8>> {
        Err(Refusal::PagedNotYet.error())
    }

    fn control_out_paged(&mut self, _id: CmdId, _value: u16, _data: &[u8]) -> Result<()> {
        Err(Refusal::PagedNotYet.error())
    }

    fn generation(&self) -> Option<Generation> {
        Some(Generation::Paged(self.info))
    }
}

#[cfg(test)]
#[path = "paged/tests_wp_pgwire.rs"]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_pgwire;
