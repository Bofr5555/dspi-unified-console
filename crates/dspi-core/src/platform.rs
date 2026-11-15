//! Small stateless facts the clients used to keep their own copies of (WP
//! CG-SWS-a): the platform's shape, the core-voltage code table per platform,
//! the firmware's default channel names, the Console's words for choice
//! settings, and the parameter image's presence bytes that `DspState` has no
//! field for.
//!
//! None of these touches a device or takes a lock, and none panics, whatever
//! the arguments.
//!
//! # Default channel names
//!
//! The rule is the firmware's `get_default_channel_name`
//! (`DSPi/firmware/DSPi/usb_audio.c:294-345`, since 78070f9), which the
//! firmware also uses as its "still default" test when the input source or
//! an output type changes (`channel_name_is_default`, `main.c:768`, `:3781`),
//! so a client must produce exactly these strings:
//!
//! - inputs follow the selected input source: USB (0, and any source this
//!   build does not know) `USB 1`..`USB 8`, no L/R; S/PDIF input 1 (1)
//!   `SPDIF L`/`SPDIF R`; S/PDIF inputs 2-4 (4-6) `SPDIF 2 L`..; I2S (2)
//!   `I2S 1 L`, `I2S 1 R`, `I2S 2 L`..; ADAT (3) `ADAT 1`..; the audio link
//!   (7) `ALINK 1`..;
//! - outputs `<SPDIF|I2S> <slot> <L|R>` by each stereo slot's output type (1
//!   is I2S, anything else S/PDIF), and the last output `PDM`.
//!
//! Qt's `SettingsController.cpp:412-428` already follows it (without the
//! audio link). The Mac's `DSPViewModel.defaultChannelNames` (`USB L`,
//! `USB R`, `USB 3`.. whatever the source), Android's and the TUI's copies
//! are the older rule and switch to this one.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::OnceLock;

use dspi_proto::registry::{self, Kind};
use dspi_session::DeviceState;
use dspi_session::cs_model::{self, StaticContext, noun};

use crate::records::{BulkImageExtras, EnumValueLabel, PlatformShape};
use crate::{DspiError, Result};

// ------------------------------------------------------------ platforms

/// Platform ids of `REQ_GET_PLATFORM`.
const RP2040: u8 = 0;
const RP2350: u8 = 1;
const STM32H723: u8 = 2;

/// The shape of platform `platform_id` (`DspState::platform_id`), for laying
/// out pages before or without a device. A connected device's own counts are
/// in `DspState` (`num_input_channels`, `num_output_channels`); this is the
/// platform's table, the Mac's `platformName == "RP2040" ? ... : ...` rules
/// (`DSPViewModel.swift:1540-1543`, `:1970`, `:2346-2353`, `:2232`;
/// `DSPi_ConsoleApp.swift:427`) in one place.
///
/// An id this build does not know gets the RP2040's shape (the smallest) with
/// `known` false, as the Mac names an unknown id `RP2040`
/// (`Commands.swift:160-162`). The longest delay is the core's rule
/// (`dspi_proto::Platform::max_delay_ms`: 42 ms on RP2350, 21 ms elsewhere),
/// not the Mac's slider ceiling of 85 ms, which no platform delivers.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn platform_shape(platform_id: u8) -> PlatformShape {
    let known = matches!(platform_id, RP2040 | RP2350 | STM32H723);
    let small = !known || platform_id == RP2040;
    let name = match platform_id {
        RP2040 => "RP2040".to_owned(),
        RP2350 => "RP2350".to_owned(),
        STM32H723 => "STM32H723".to_owned(),
        n => format!("unknown platform {n}"),
    };
    // Outputs are the slots' stereo pairs plus the PDM sub, which comes last
    // in the output list and in the pin list.
    let (inputs, slots, i2s_pairs) = if small { (2, 2, 1) } else { (8, 4, 4) };
    let outputs: u8 = if small { 5 } else { 9 };
    let pdm_output: u8 = if small { 4 } else { 8 };
    PlatformShape {
        platform_id,
        name,
        known,
        num_input_channels: inputs,
        num_output_channels: outputs,
        num_output_slots: slots,
        pdm_output,
        pdm_pin_index: slots,
        // `CORE1_EQ_FIRST_OUTPUT` is 2; the last is the output below PDM
        // (config.h, as `dspi_session::write::Session::eq_worker_outputs`).
        eq_worker_first: 2,
        eq_worker_last: if small { 3 } else { 7 },
        i2s_max_pairs: i2s_pairs,
        multichannel_i2s: platform_id == RP2350,
        adat: platform_id == RP2350,
        upmixer: platform_id == RP2350,
        i2s_config: platform_id != STM32H723,
        crossfeed_pairs: if platform_id == RP2040 { 2 } else { 4 },
        max_delay_ms: dspi_proto::Platform::from_id(platform_id).max_delay_ms(),
    }
}

// -------------------------------------------------------- core voltage

/// pico-sdk's RP2350 codes above 1.30 V (0x10-0x1F), in mV. They need the
/// POWMAN limit lifted; the firmware never programs them, but a reply is
/// shown as it is.
const RP2350_HIGH_MV: [u16; 16] = [
    1350, 1400, 1500, 1600, 1650, 1700, 1800, 1900, 2000, 2350, 2500, 2650, 2800, 3000, 3150, 3300,
];

/// A pico-sdk `enum vreg_voltage` code in mV on platform `platform_id`, or
/// `None` for a code the chip does not have.
///
/// - Both chips: 0x06 (0.85 V) to 0x0F (1.30 V) in 50 mV steps.
/// - RP2350 adds 0x00-0x05 (0.55-0.80 V) and 0x10-0x1F (1.35-3.30 V).
/// - On RP2040, 0x00-0x05 all program 0.80 V, and there is nothing above
///   0x0F.
/// - Any other platform (STM32H723 has no `EXT_CORE_VREG`): only the codes
///   both chips agree on, as `dspi_proto::ext::vreg_mv`.
///
/// The Mac's `VregVoltage.volts` (`ExtensionChannel.swift:272-292`) is the
/// same table for RP2040 and RP2350.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn vreg_code_mv(code: u8, platform_id: u8) -> Option<u16> {
    match (platform_id, code) {
        (RP2040, 0x00..=0x05) => Some(800),
        (RP2350, 0x00..=0x05) | (_, 0x06..=0x0F) => {
            Some(550_u16.saturating_add(50_u16.saturating_mul(u16::from(code))))
        }
        (RP2350, 0x10..=0x1F) => RP2350_HIGH_MV
            .get(usize::from(code.saturating_sub(0x10)))
            .copied(),
        _ => None,
    }
}

/// A core-voltage code as the user reads it: `1.20 V`, or `code 0x1F` for a
/// code [`vreg_code_mv`] does not decode on that platform (the Mac's
/// `VregVoltage.label`, `ExtensionChannel.swift:294-298`).
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn vreg_code_label(code: u8, platform_id: u8) -> String {
    match vreg_code_mv(code, platform_id) {
        Some(mv) => format!("{}.{:02} V", mv / 1000, (mv % 1000) / 10),
        None => format!("code 0x{code:02X}"),
    }
}

// -------------------------------------------------- default channel names

/// The firmware's name for each channel when the user has named none: inputs
/// first, then outputs, `num_inputs + num_outputs` names. `input_source` is
/// `DspState::input_source` (it names the inputs); `output_types` is
/// `DspState::output_types` (one byte per stereo slot, 1 for I2S), and a
/// missing entry reads as S/PDIF. The rule is in the module docs.
///
/// For a connected device pass `DspState`'s `num_input_channels`,
/// `num_output_channels`, `input_source` and `output_types`; without one,
/// [`platform_shape`]'s counts.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn default_channel_names(
    num_inputs: u8,
    num_outputs: u8,
    input_source: u8,
    output_types: Vec<u8>,
) -> Vec<String> {
    let total = u16::from(num_inputs).saturating_add(u16::from(num_outputs));
    (0..total)
        .filter_map(|ch| {
            default_channel_name(
                num_inputs,
                num_outputs,
                input_source,
                &output_types,
                ch.into(),
            )
        })
        .collect()
}

/// [`default_channel_names`]' entry for wire channel `ch` (inputs first), or
/// `None` past the last output. The one implementation; `presets.rs` reads it.
pub(crate) fn default_channel_name(
    num_inputs: u8,
    num_outputs: u8,
    input_source: u8,
    output_types: &[u8],
    ch: usize,
) -> Option<String> {
    let side = |n: usize| if n.is_multiple_of(2) { 'L' } else { 'R' };
    let inputs = usize::from(num_inputs);
    if ch < inputs {
        let n = ch.saturating_add(1);
        let pair = (ch / 2).saturating_add(1);
        return Some(match input_source {
            1 => format!("SPDIF {}", side(ch)),
            2 => format!("I2S {pair} {}", side(ch)),
            3 => format!("ADAT {n}"),
            // SPDIF2..SPDIF4: input index + 1 (`spdif_index_for_source`).
            4..=6 => format!("SPDIF {} {}", input_source.saturating_sub(2), side(ch)),
            7 => format!("ALINK {n}"),
            _ => format!("USB {n}"),
        });
    }
    let out = ch.checked_sub(inputs)?;
    let outputs = usize::from(num_outputs);
    if out >= outputs {
        return None;
    }
    if out.saturating_add(1) == outputs {
        return Some("PDM".to_owned());
    }
    let slot = out / 2;
    let kind = if output_types.get(slot) == Some(&1) {
        "I2S"
    } else {
        "SPDIF"
    };
    Some(format!("{kind} {} {}", slot.saturating_add(1), side(out)))
}

// ------------------------------------------------------ choice labels

/// The registry paths [`enum_value_labels`] knows, with the control-surface
/// noun whose labels they share.
const LABELLED: [(&str, u8); 7] = [
    ("in.source", noun::INPUT_SOURCE),
    ("lev.speed", noun::LEVELLER_SPEED),
    ("cf.preset", noun::CROSSFEED_PRESET),
    ("up.center_mode", noun::UPMIX_CENTER_MODE),
    ("up.surround_mode", noun::UPMIX_SURROUND_MODE),
    ("sub.select", noun::SUBHARM_SELECT),
    ("tube.type", noun::TUBE_TYPE),
];

/// A device state for [`cs_model::enum_value_label`], which reads none for
/// these nouns: only preset and macro labels look at the device.
fn label_state() -> &'static DeviceState {
    static STATE: OnceLock<DeviceState> = OnceLock::new();
    STATE.get_or_init(cs_model::demo::base_state)
}

/// Every value of the choice setting `path` with the words the Console shows
/// for it, in value order: `in.source`, `lev.speed`, `cf.preset`,
/// `up.center_mode`, `up.surround_mode`, `sub.select` and `tube.type`. The
/// words are `dspi_session::cs_model::enum_value_label`'s, the one table the
/// control surfaces, the TUI and now every client read (upmixer `Sinner`,
/// `Logician`, `Off`; selectivity `All material`, `Percussive`, `Sustained`;
/// the tube styles, 0 `Custom`).
///
/// Errors: [`DspiError::InvalidArgument`] for any other path.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn enum_value_labels(path: String) -> Result<Vec<EnumValueLabel>> {
    let invalid = || DspiError::InvalidArgument {
        msg: format!("no labelled choices for {path}"),
    };
    let (_, n) = LABELLED
        .iter()
        .find(|(p, _)| *p == path)
        .ok_or_else(invalid)?;
    let desc = registry::by_path(&path).ok_or_else(invalid)?;
    let values: Vec<(u8, String)> = match desc.kind {
        Kind::Choice(variants) => variants
            .iter()
            .map(|(v, key)| (*v, (*key).to_owned()))
            .collect(),
        Kind::Int { min, max, .. } => {
            let lo = u8::try_from(min.max(0)).unwrap_or(0);
            let hi = u8::try_from(max.clamp(0, 255)).unwrap_or(u8::MAX);
            (lo..=hi).map(|v| (v, v.to_string())).collect()
        }
        _ => return Err(invalid()),
    };
    let cx = StaticContext {
        state: label_state(),
        claims: &[],
        control_i2c_sda: None,
        preset_names: &[],
    };
    let cs = cs_model::CsData::default();
    Ok(values
        .into_iter()
        .map(|(value, key)| EnumValueLabel {
            value,
            key,
            label: cs_model::enum_value_label(&cx, &cs, *n, i32::from(value)),
        })
        .collect())
}

// -------------------------------------------------------- image bytes

/// Offsets into the V32 parameter image, from the generated layout.
// Evaluated at compile time over constant tables: an index out of range or
// an overflow is a compile error, not a panic.
#[allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
mod off {
    use dspi_proto::generated as g;

    /// `field`'s offset in a `(name, offset, length)` table, 0 if absent
    /// (checked by the tests).
    const fn field(table: &[(&str, usize, usize)], name: &str) -> usize {
        let mut i = 0;
        while i < table.len() {
            let (n, o, _) = table[i];
            if const_eq(n, name) {
                return o;
            }
            i += 1;
        }
        0
    }

    const fn const_eq(a: &str, b: &str) -> bool {
        let (a, b) = (a.as_bytes(), b.as_bytes());
        if a.len() != b.len() {
            return false;
        }
        let mut i = 0;
        while i < a.len() {
            if a[i] != b[i] {
                return false;
            }
            i += 1;
        }
        true
    }

    use dspi_proto::wire::INPUT_CONFIG_FIELDS as IC;
    pub const I2S_INPUT_CHANNELS: usize = g::OFF_INPUT_CONFIG + field(&IC, "i2s_input_channels");
    pub const I2S_RX_PIN_EXT: usize = g::OFF_INPUT_CONFIG + field(&IC, "i2s_rx_pin_ext");
    pub const SPDIF_RX_PIN_EXT: usize = g::OFF_INPUT_CONFIG + field(&IC, "spdif_rx_pin_ext");
    /// `WireI2sConfig.bck_pin_slave` (byte 9, as `state.rs`'s `i2s::BCK_PIN_SLAVE`).
    pub const I2S_BCK_PIN_SLAVE: usize = g::OFF_I2S_CONFIG + 9;
    /// `WireAdatConfig.pin` (byte 1, as `state.rs`'s `adat::PIN`).
    pub const ADAT_PIN: usize = g::OFF_ADAT_CONFIG + 1;
}

/// The parameter image's presence bytes that `DspState` has no field for:
/// the firmware writes 0 for "absent, keep what you have" (`bulk_params.h`),
/// and a client that keeps its own value for an absent one asks here instead
/// of reading `bulk_raw` by offset (the Mac's `CoreMirror.raw`,
/// `CoreMirror.swift:661-720`, `:798-807`). `bulk_raw` is
/// `DspState::bulk_raw`; anything shorter than an image reads as absent.
///
/// The custom crossfeed pair, which the Mac also reads by offset
/// (`PresetDocumentTransfer.swift:20-28`), is
/// [`crate::DspiCore::crossfeed_custom_pair`].
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn bulk_image_extras(bulk_raw: Vec<u8>) -> BulkImageExtras {
    use dspi_proto::generated::{BULK_SIZE, wire::WIRE_FORMAT_VERSION};
    let at = |i: usize| bulk_raw.get(i).copied().unwrap_or(0) != 0;
    let three = |first: usize| (0..3).map(|i| at(first.saturating_add(i))).collect();
    BulkImageExtras {
        decoded: bulk_raw.len() >= BULK_SIZE
            && bulk_raw.first().copied() == Some(WIRE_FORMAT_VERSION as u8),
        i2s_bck_pin_slave_present: at(off::I2S_BCK_PIN_SLAVE),
        i2s_input_channels_present: at(off::I2S_INPUT_CHANNELS),
        i2s_rx_pin_ext_present: three(off::I2S_RX_PIN_EXT),
        spdif_rx_pin_ext_present: three(off::SPDIF_RX_PIN_EXT),
        adat_out_pin_present: at(off::ADAT_PIN),
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
#[path = "platform/tests_wp_cgswsa.rs"]
mod tests_wp_cgswsa;
