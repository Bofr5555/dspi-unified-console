//! The paged vendor protocol of firmware 1.1.7 (upstream `firmware/DSPi/vendor/`,
//! DSPi Link spec 1.3): the seam of PLAN-PG (`docs/core-merge/PLAN-PG.md`).
//!
//! A paged command is addressed by a 16-bit id: the page in the high byte (on USB
//! the high byte of `wIndex`), the command number in the low byte (`bRequest`,
//! 0x10 and up). 1.1.6-generation firmware ignores the page byte and would run
//! the old opcode with the same number, so nothing paged is ever sent before
//! [`SYS_PROTOCOL_INFO`] has answered with [`PROTOCOL_MAGIC`] (PLAN-PG section 1.1).
//!
//! Types only. WP PGPROTO adds the registry generated from the vendored
//! `dspi_commands.json`, the planner generated from `legacy_map.json` and the
//! discovery decoders, and replaces the literals below with generated constants
//! (a test then holds them equal).
//!
//! Pure: no I/O.

// -- WP PLANPG -- (seam)

use crate::VENDOR_INTERFACE;

/// A paged command: `(page << 8) | cmd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CmdId(pub u16);

impl CmdId {
    pub const fn new(page: u8, cmd: u8) -> Self {
        Self(((page as u16) << 8) | cmd as u16)
    }

    /// The page: the high byte of `wIndex`.
    pub const fn page(self) -> u8 {
        (self.0 >> 8) as u8
    }

    /// The command number: `bRequest`.
    pub const fn cmd(self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    /// `wIndex` on USB and in a DSPi Link CMD frame: the page and the vendor
    /// interface (Link spec 1.3 section 8.2).
    pub const fn w_index(self) -> u16 {
        (self.0 & 0xFF00) | VENDOR_INTERFACE
    }

    /// The id a request names, as a hub takes it from a CMD frame: the page from
    /// `wIndex`'s high byte (the low byte is the interface and is not part of it).
    pub const fn from_request(b_request: u8, w_index: u16) -> Self {
        Self((w_index & 0xFF00) | b_request as u16)
    }

    /// Page 0: a 1.1.6-shaped request. It names nothing on paged-only firmware.
    pub const fn is_legacy(self) -> bool {
        self.page() == LEGACY_PAGE
    }
}

/// Page 0: every 1.1.6 request, our `REQ_EXTENSION` 0x00 included.
pub const LEGACY_PAGE: u8 = 0;

/// Pages upstream keeps for forks and ports and never allocates (upstream
/// `vendor_command_overhaul_plan.md` section 4).
pub const PRIVATE_PAGES: core::ops::RangeInclusive<u8> = 0xE0..=0xEF;

/// Our extension page on paged firmware (PLAN-PG OD-PGc).
pub const EXT_PAGE: u8 = 0xE0;

/// The extension tunnel: GET and SET, `wValue` = the extension id
/// (`EXT_*`, unchanged), payloads as on `REQ_EXTENSION` 0x00 (PLAN-PG OD-PGc).
pub const EXT_TUNNEL: CmdId = CmdId::new(EXT_PAGE, 0x10);

/// `SYS.PROTOCOL_INFO`: the probe, sent before anything else (migration guide 4.2).
pub const SYS_PROTOCOL_INFO: CmdId = CmdId::new(0x01, 0x10);

/// `SYS.PROTOCOL_INFO`'s reply length.
pub const PROTOCOL_INFO_LEN: u16 = 16;

/// The first four bytes of a `SYS.PROTOCOL_INFO` reply.
pub const PROTOCOL_MAGIC: [u8; 4] = *b"DSPi";

/// The protocol major this build speaks.
pub const PROTOCOL_MAJOR: u8 = 1;

/// A decoded `SYS.PROTOCOL_INFO` (migration guide 4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProtocolInfo {
    pub major: u8,
    pub minor: u8,
    /// Number of implemented pages (`SYS.PAGE_LIST` has twice as many bytes).
    pub pages: u8,
    /// Largest non-bulk SET payload.
    pub max_set: u16,
    /// Largest non-bulk GET reply.
    pub max_get: u16,
}

/// What a probe reply means. Only [`ProbeReply::Paged`] lets anything paged be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeReply {
    /// Paged firmware whose major this build speaks.
    Paged(ProtocolInfo),
    /// Paged firmware of a major this build does not know: send nothing else.
    UnknownMajor(u8),
    /// Something answered, but not with a protocol-info block: send nothing else.
    Garbled,
}

impl ProtocolInfo {
    /// Decode a probe reply. A reply is paged only with the magic, at least 12
    /// bytes and major [`PROTOCOL_MAJOR`].
    pub fn decode(b: &[u8]) -> ProbeReply {
        let (Some(magic), Some(rest)) = (b.get(..4), b.get(4..12)) else {
            return ProbeReply::Garbled;
        };
        if magic != PROTOCOL_MAGIC {
            return ProbeReply::Garbled;
        }
        let [major, minor, pages, _flags, s0, s1, g0, g1] = rest else {
            return ProbeReply::Garbled;
        };
        if *major != PROTOCOL_MAJOR {
            return ProbeReply::UnknownMajor(*major);
        }
        ProbeReply::Paged(ProtocolInfo {
            major: *major,
            minor: *minor,
            pages: *pages,
            max_set: u16::from_le_bytes([*s0, *s1]),
            max_get: u16::from_le_bytes([*g0, *g1]),
        })
    }
}

/// Which protocol a connected device speaks, decided once per connection by the
/// probe (PLAN-PG section 1.1) and never changed without a reconnect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum Generation {
    /// 1.1.6 and older (stock, or `bofr-main` with or without its page guard):
    /// page-0 requests only.
    Legacy,
    /// Paged firmware (stock 1.1.7, or our dual-stack port): paged requests only,
    /// from this client, whatever else the firmware also answers.
    Paged(ProtocolInfo),
}

#[cfg(test)]
mod tests_wp_planpg {
    use super::*;

    /// The migration guide's own probe exchange (section 4.2).
    const GUIDE_REPLY: [u8; 16] = [
        0x44, 0x53, 0x50, 0x69, 0x01, 0x00, 0x0C, 0x00, 0x40, 0x00, 0x00, 0x01, 0, 0, 0, 0,
    ];

    #[test]
    fn wp_planpg_probe_setup_matches_the_guide() {
        // SETUP C1 10 00 00 02 01 10 00
        assert_eq!(SYS_PROTOCOL_INFO.cmd(), 0x10);
        assert_eq!(SYS_PROTOCOL_INFO.w_index().to_le_bytes(), [0x02, 0x01]);
        assert_eq!(PROTOCOL_INFO_LEN, 0x10);
    }

    #[test]
    fn wp_planpg_guide_reply_decodes() {
        assert_eq!(
            ProtocolInfo::decode(&GUIDE_REPLY),
            ProbeReply::Paged(ProtocolInfo {
                major: 1,
                minor: 0,
                pages: 12,
                max_set: 64,
                max_get: 256,
            })
        );
    }

    #[test]
    fn wp_planpg_anything_else_is_not_paged() {
        assert_eq!(ProtocolInfo::decode(&[]), ProbeReply::Garbled);
        assert_eq!(
            ProtocolInfo::decode(&GUIDE_REPLY[..11]),
            ProbeReply::Garbled
        );
        let mut wrong = GUIDE_REPLY;
        wrong[0] = b'X';
        assert_eq!(ProtocolInfo::decode(&wrong), ProbeReply::Garbled);
        let mut newer = GUIDE_REPLY;
        newer[4] = 2;
        assert_eq!(ProtocolInfo::decode(&newer), ProbeReply::UnknownMajor(2));
    }

    #[test]
    fn wp_planpg_ids_split_and_join() {
        let id = CmdId::new(0x05, 0x53);
        assert_eq!((id.page(), id.cmd(), id.0), (0x05, 0x53, 0x0553));
        assert_eq!(CmdId::from_request(0x53, 0x0502), id);
        // A Mac-style wIndex 0 and our 2 are both page 0.
        assert!(CmdId::from_request(0x7F, 0).is_legacy());
        assert!(CmdId::from_request(0x7F, 2).is_legacy());
        assert_eq!(EXT_TUNNEL.0, 0xE010);
        assert_eq!(EXT_TUNNEL.w_index(), 0xE002);
        assert!(PRIVATE_PAGES.contains(&EXT_TUNNEL.page()));
    }
}
