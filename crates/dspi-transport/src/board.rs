//! WP PGZERO (PLAN-PG section 1.9): which board a device is, from the USB
//! descriptor and the platform reply.
//!
//! A board whose platform byte 7 (the board id) is present and not 0, or whose
//! USB descriptor names a board this build does not know as a standard Pico,
//! never enters the bootloader, is never offered an image and is never in a
//! rollout. An unknown board fails closed. A byte 7 that is absent means board 0.
//!
//! A paged DSPi-Zero cannot be told from a Pico by `SYS.PLATFORM` (the registry
//! fixes it at 7 bytes): its product string and `bcdDevice` identify it where
//! the descriptor is at hand ([`DeviceDescriptor::product`],
//! [`DeviceDescriptor::bcd_device`]: USB, WebUSB, Android, our hub's own
//! attach). Behind a hub the descriptor is not at hand: a legacy Zero is still
//! known by byte 7, a paged one behind an upstream hub is unknown (it reads as
//! board 0) unless our hub passes its board on.
//!
//! This sits in the transport crate, not in `dspi-core`, because the hub's
//! headless build (no `dspi-core`) needs it too: the same rule and names as
//! WP PGFWUP's `dspi_core::hardware` (which re-exports these at the batch merge,
//! so there is one table).
//!
//! [`DeviceDescriptor::product`]: crate::DeviceDescriptor::product
//! [`DeviceDescriptor::bcd_device`]: crate::DeviceDescriptor::bcd_device

use dspi_proto::packets::{Board, PlatformInfo};

/// The product string of the DSPi-Zero (upstream DSPi-Zero `192df73`,
/// `uac2/usb_descriptors.c`).
pub const ZERO_PRODUCT: &str = "Weeb Labs DSPi Zero";
/// The DSPi-Zero's `bcdDevice` (`uac2/usb_descriptors.h`).
pub const ZERO_BCD_DEVICE: u16 = 0x0300;
/// The product strings of the Pico builds this build knows: upstream and our
/// DSPi, the alink sender (current and first name). Older firmware on the
/// Raspberry Pi VID names itself `Pico DSP ...` ([`PICO_PRODUCT_PREFIX`]).
pub const PICO_PRODUCTS: [&str; 3] = [
    "Weeb Labs DSPi",
    "DSPi Audio Link Sender",
    "DSPi Link Sender",
];
/// The product prefix of the pre-DSPi firmware (`Pico DSP 2.1 (WinUSB)`).
pub const PICO_PRODUCT_PREFIX: &str = "Pico DSP";
/// The [`Board::Unknown`] id for a board known only by a product string this
/// build does not know (no platform byte names it).
pub const UNKNOWN_PRODUCT_BOARD: u8 = 0xFF;

/// The name the clients show for a board (caps, About, the inventory: S6).
pub fn board_name(board: Board) -> &'static str {
    match board {
        Board::Pico => "DSPi",
        Board::PiZero2W => "DSPi Zero",
        Board::Unknown(_) => "Unknown board",
    }
}

/// What a USB device descriptor says about the board: `None` when it says
/// nothing (no product string, and not the Zero's `bcdDevice`). A product
/// string this build does not know is an unknown board
/// ([`UNKNOWN_PRODUCT_BOARD`]): it fails closed.
pub fn board_from_descriptor(product: Option<&str>, bcd_device: Option<u16>) -> Option<Board> {
    let product = product.map(str::trim).filter(|p| !p.is_empty());
    if product == Some(ZERO_PRODUCT) || bcd_device == Some(ZERO_BCD_DEVICE) {
        return Some(Board::PiZero2W);
    }
    let p = product?;
    if PICO_PRODUCTS.contains(&p) || p.starts_with(PICO_PRODUCT_PREFIX) {
        Some(Board::Pico)
    } else {
        Some(Board::Unknown(UNKNOWN_PRODUCT_BOARD))
    }
}

/// The board, from the platform reply (byte 7) and what the descriptor says:
/// a board the platform names is that board; on board 0 the descriptor
/// decides, if it says anything, else it is a Pico.
pub fn identify_board(platform: &PlatformInfo, descriptor: Option<Board>) -> Board {
    match platform.board() {
        Board::Pico => descriptor.unwrap_or(Board::Pico),
        other => other,
    }
}

#[cfg(test)]
mod tests_wp_pgzero {
    use super::*;
    use crate::fake::{FakeBoard, FakeGeneration};
    use crate::{FakeDevice, FakeOptions, Transport};
    use dspi_proto::generated::opcodes as op;

    fn platform_of(t: &mut dyn Transport) -> PlatformInfo {
        let len = u16::try_from(PlatformInfo::FULL_LEN).unwrap();
        PlatformInfo::decode(&t.control_in_upto(op::REQ_GET_PLATFORM, 0, len).unwrap()).unwrap()
    }

    #[test]
    fn wp_pgzero_descriptor_rule() {
        assert_eq!(board_from_descriptor(None, None), None);
        assert_eq!(board_from_descriptor(Some("  "), Some(0x0204)), None);
        assert_eq!(
            board_from_descriptor(Some(ZERO_PRODUCT), None),
            Some(Board::PiZero2W)
        );
        assert_eq!(
            board_from_descriptor(None, Some(ZERO_BCD_DEVICE)),
            Some(Board::PiZero2W)
        );
        for p in PICO_PRODUCTS {
            assert_eq!(
                board_from_descriptor(Some(p), Some(0x0204)),
                Some(Board::Pico)
            );
        }
        assert_eq!(
            board_from_descriptor(Some("Pico DSP 2.1 (WinUSB)"), None),
            Some(Board::Pico)
        );
        // A product this build does not know fails closed
        let b = board_from_descriptor(Some("Somebody's DSP"), Some(0x0204)).unwrap();
        assert_eq!(b, Board::Unknown(UNKNOWN_PRODUCT_BOARD));
        assert!(!b.has_bootsel());
        assert_eq!(board_name(Board::PiZero2W), "DSPi Zero");
        assert_eq!(board_name(Board::Pico), "DSPi");
        assert_eq!(board_name(b), "Unknown board");
    }

    /// The fakes carry their board's product string and `bcdDevice`, as the
    /// USB backends do, so the Zero is named from its descriptor.
    #[test]
    fn wp_pgzero_fake_descriptors_name_the_board() {
        let zero = FakeDevice::zero();
        let d = zero.descriptor();
        assert_eq!(d.product.as_deref(), Some(ZERO_PRODUCT));
        assert_eq!(d.bcd_device, Some(ZERO_BCD_DEVICE));
        assert_eq!(d.board(), Some(Board::PiZero2W));
        assert_eq!(d.board().map(board_name), Some("DSPi Zero"));
        let pico = FakeDevice::demo();
        assert_eq!(pico.descriptor().board(), Some(Board::Pico));
        let sender = FakeDevice::sender(false);
        assert_eq!(sender.descriptor().board(), Some(Board::Pico));
        assert_eq!(sender.descriptor().bcd_device, Some(0x0280));
    }

    /// A legacy Zero: byte 7 names it with or without the descriptor, so a
    /// Zero behind a link (no descriptor) is still recognised.
    #[test]
    fn wp_pgzero_legacy_zero_by_byte_7_without_a_descriptor() {
        let mut zero = crate::paged::open(Box::new(FakeDevice::zero())).unwrap();
        let info = platform_of(&mut *zero);
        assert_eq!(info.board_id, Board::PI_ZERO_2W_ID);
        assert_eq!(identify_board(&info, None), Board::PiZero2W);
        assert_eq!(
            identify_board(&info, zero.descriptor().board()),
            Board::PiZero2W
        );
    }

    /// A paged Zero answers the registry's 7-byte `SYS.PLATFORM`: board 0 by
    /// the platform, the Zero by its descriptor; without a descriptor (behind
    /// an upstream hub) it reads as a Pico, which the manual says.
    #[test]
    fn wp_pgzero_paged_zero_only_by_its_descriptor() {
        let fake = FakeDevice::new(
            Capture::demo(),
            FakeOptions {
                board: FakeBoard::Zero,
                generation: Some(FakeGeneration::STOCK117),
                ..FakeOptions::default()
            },
        );
        let mut t = crate::paged::open(Box::new(fake)).unwrap();
        let info = platform_of(&mut *t);
        assert_eq!(info.board_id, 0, "SYS.PLATFORM is 7 bytes");
        assert_eq!(
            identify_board(&info, t.descriptor().board()),
            Board::PiZero2W
        );
        assert_eq!(identify_board(&info, None), Board::Pico);
    }

    use crate::fake::Capture;
}
