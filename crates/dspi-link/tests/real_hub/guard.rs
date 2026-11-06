//! The only way to the device. No SET can be expressed here, and the one
//! `command` call of the harness is in [`get`] (`self_checks` enforces that).
//!
//! "Direction is GET" is not enough on DSPi: opcodes are multiplexed by wValue,
//! and some SET-named requests answer on the IN direction. So every row is a read
//! the core itself sends at each attach, with the same wValue:
//! `dspi-session/src/probe.rs` and `read_per_item` in `dspi-core/src/connect.rs`
//! at 52ad5f4. Update the table when those change.

use dspi_link::frames::CmdResponse;
use dspi_link::{Dir, LinkClient};

/// wIndex: the vendor interface (`dspi_proto::VENDOR_INTERFACE`).
const VENDOR_INTERFACE: u16 = 2;
/// A bridge truncates a GET to this (bridge spec 9.2); the bulk read is exempt.
pub const BRIDGE_GET_LIMIT: u16 = 132;
/// `PRESET_SLOTS`.
pub const PRESET_SLOTS: u16 = 10;
const REQ_GET_CS_CAPS: u8 = 0x86;
const REQ_SIGGEN_GET_CAPS: u8 = 0xA8;

/// One read.
#[derive(Debug, Clone, Copy)]
pub struct Get {
    pub name: &'static str,
    req: u8,
    pub value: u16,
    pub len: u16,
}

const fn row(name: &'static str, req: u8, value: u16, len: u16) -> Get {
    Get { name, req, value, len }
}

pub const PLATFORM: Get = row("platform", 0x7F, 0, 7);
pub const BULK: Get = row("bulk", 0xA0, 0, 6136);
pub const CS_CAPS_HEADER: Get = row("cs_caps_header", REQ_GET_CS_CAPS, 0xFFFF, 4);
pub const SIGGEN_CAPS_HEADER: Get = row("siggen_caps_header", REQ_SIGGEN_GET_CAPS, 0xFFFF, 8);

/// The fixed reads of an attach, the bulk read apart.
pub const ATTACH: &[Get] = &[
    PLATFORM,
    row("serial", 0x7E, 0, 16),
    row("input_source", 0xE1, 0, 1),
    row("loudness_output_mask", 0xFB, 0, 2),
    row("crossfeed_output_mask", 0xFD, 0, 1),
    row("leveller_masks", 0xDF, 0, 2),
    row("psychoacoustic_bass", 0x31, 0, 1),
    row("upmixer", 0x4E, 0, 16),
    row("adat_output", 0xCB, 0, 1),
    row("adat_input", 0x69, 0, 1),
    row("i2s_slave_clock", 0x89, 0, 1),
    row("i2s_input_channels", 0xF4, 0, 1),
    row("spdif_multi_input", 0xEF, 0, 6),
    row("lg_sound_sync", 0xE7, 0, 1),
    row("dac_hardware_mute", 0xEB, 0, 16),
    row("uart_control", 0xF6, 0, 8),
    row("i2c_control", 0xF8, 0, 8),
    row("test_signals", 0xA7, 0, 16),
    row("subharmonic_synth", 0x11, 0, 1),
    row("tube_preamp", 0x3F, 0, 4),
    row("output_limiter", 0x81, 0x81, 4),
    row("spectrum_analyser", 0x0A, 0, 16),
    row("input_source_names", 0x55, 0, 32),
    row("device_name", 0x57, 0, 32),
    row("output_latency", 0x03, 0, 12),
    CS_CAPS_HEADER,
    row("cs_display_cfg", 0x28, 0, 16),
    SIGGEN_CAPS_HEADER,
    row("build_info", 0x80, 0, 64),
    row("preset_active", 0x9A, 0, 1),
    row("eq_band_0", 0x43, 0, 4),
    row("core1_mode", 0x7A, 0, 1),
    row("subharm_solo", 0x2D, 0, 1),
    row("ctrl_iface_status", 0xF9, 0, 8),
    row("preset_dir", 0x95, 0, 7),
];

/// The whole control-surface caps header, whose length the short one says.
pub fn cs_caps_full(type_count: u8) -> Get {
    let len = 4u16.saturating_add(4u16.saturating_mul(u16::from(type_count))).saturating_add(4);
    row("cs_caps_full", REQ_GET_CS_CAPS, 0xFFFF, len)
}

/// One signal generator descriptor (`SiggenTypeDesc::SIZE`).
pub fn siggen_desc(index: u8) -> Get {
    row("siggen_desc", REQ_SIGGEN_GET_CAPS, u16::from(index), 62)
}

/// The name of preset `slot`.
pub fn preset_name(slot: u16) -> Get {
    row("preset_name", 0x93, slot, 32)
}

/// Run one read on device `handle`.
pub fn get(c: &LinkClient, handle: u8, g: &Get) -> CmdResponse {
    c.command(handle, Dir::Get, g.req, g.value, VENDOR_INTERFACE, g.len, &[])
}
