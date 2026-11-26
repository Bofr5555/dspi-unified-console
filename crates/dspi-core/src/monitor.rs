//! The notification monitor (WP C7): every notification the core drains is
//! turned into one readable line and kept, with a rising id and the time the
//! reader received it, for a client's Interrupt Monitor view.
//!
//! The log is bounded ([`MonitorLog::LIMIT`] lines, oldest dropped first).
//! Ids never restart while the core lives, not even across
//! [`DspiCore::monitor_clear`], so a client polling with the last id it saw
//! cannot read a line twice.
//!
//! A `PARAM_CHANGED` carries only an offset into `WireBulkParams`, so the
//! line names the field there: the section comes from the generated section
//! table, the field from the per-section layouts below (bulk_params.h). Every
//! step has a fallback (`section+0x..`, `offset=0x....`, a hex dump), so no
//! offset or payload, however odd, can make a line fail.

use std::collections::VecDeque;
use std::fmt::Write as _;

use dspi_proto::generated::{SECTIONS, notify as nfy, rta, wire};
use dspi_proto::packets::RtaConfig;
use dspi_session::notify::{Event, Notification, Source};

use crate::DspiCore;
use crate::records::{MonitorBatch, MonitorEntry};

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The monitor lines with an id above `after_id`, oldest first, and the
    /// id to pass next time (`after_id` itself when there is nothing new).
    /// Pass 0 for everything still kept. Never fails.
    pub fn monitor_read(&self, after_id: u64) -> MonitorBatch {
        self.lock().monitor.after(after_id)
    }

    /// Forget every monitor line. Ids carry on from where they were, so a
    /// reader's last id stays valid. Never fails.
    pub fn monitor_clear(&self) {
        self.lock().monitor.entries.clear();
    }
}

/// The kept lines and the last id handed out.
#[derive(Default)]
pub(crate) struct MonitorLog {
    entries: VecDeque<MonitorEntry>,
    last_id: u64,
}

impl MonitorLog {
    /// The most lines kept; a client keeps the same number on its side.
    pub(crate) const LIMIT: usize = 2000;

    /// Add the line for `note`, stamped with the time its packet arrived.
    pub(crate) fn record(&mut self, note: &Notification) {
        self.last_id = self.last_id.saturating_add(1);
        self.entries.push_back(MonitorEntry {
            id: self.last_id,
            time_ms: crate::raw::epoch_ms(note.received),
            text: describe(note),
        });
        while self.entries.len() > Self::LIMIT {
            self.entries.pop_front();
        }
    }

    fn after(&self, after_id: u64) -> MonitorBatch {
        // Ids rise along the queue, so the new ones are a tail.
        let first_new = self.entries.partition_point(|e| e.id <= after_id);
        let entries: Vec<MonitorEntry> = self.entries.iter().skip(first_new).cloned().collect();
        MonitorBatch {
            last_id: entries.last().map_or(after_id, |e| e.id),
            entries,
        }
    }
}

/// One readable line for `note`.
///
/// Parameter changes read `[seq] [SOURCE  ] section.field  value`; other
/// events `[seq] v2.EventName  details`; the legacy volume packet, which has
/// no sequence number, `v1.MasterVolume  value`. A packet that came after a
/// gap in the sequence says so at the end, as does one a Link peer wrote.
pub(crate) fn describe(note: &Notification) -> String {
    let seq = note.seq;
    let mut line = match &note.event {
        Event::Idle => event_line(seq, "Idle", String::new()),
        Event::MasterVolume(db) => format!("{:<16}{}", "v1.MasterVolume", volume(*db)),
        Event::ParamChanged {
            offset,
            source,
            bytes,
        } => {
            let (name, value) = param(usize::from(*offset), bytes);
            format!(
                "[{seq:3}] [{:<8}] {name:<32}  {value}",
                source_label(*source)
            )
        }
        Event::BulkInvalidated { source } => event_line(
            seq,
            "BulkInvalidated",
            format!("source={}", source_label(*source)),
        ),
        Event::PresetLoaded { slot } => event_line(seq, "PresetLoaded", format!("slot={slot}")),
        Event::InputFormat { channels } => {
            event_line(seq, "InputFormat", format!("channels={channels}"))
        }
        Event::SiggenState {
            state,
            reason,
            signal_type,
            channel,
        } => {
            const STATES: &[&str] = &["IDLE", "FADE_IN", "RUN", "GAP", "FADE_OUT"];
            const REASONS: &[&str] = &["-", "HOST", "COMPLETED", "PRESET", "RECONFIG"];
            let channel = if *channel == 0xFF {
                "-".to_string()
            } else {
                channel.to_string()
            };
            event_line(
                seq,
                "SiggenState",
                format!(
                    "{} reason={} type={signal_type} ch={channel}",
                    named(STATES, *state),
                    named(REASONS, *reason)
                ),
            )
        }
        Event::AdatState {
            enabled,
            active,
            pin,
        } => event_line(
            seq,
            "AdatState",
            format!(
                "enabled={} active={} pin={pin}",
                u8::from(*enabled),
                u8::from(*active)
            ),
        ),
        Event::I2sSlaveState { state, rate_hz } => {
            const STATES: &[&str] = &["INACTIVE", "ACQUIRING", "RELOCKING", "LOCKED"];
            event_line(
                seq,
                "I2sSlaveState",
                format!("{} rate={rate_hz}", named(STATES, *state)),
            )
        }
        Event::IrLearn {
            state,
            protocol,
            code,
        } => {
            // control_surfaces.h: CS_IR_LEARN_* and CS_IR_PROTO_*.
            const STATES: &[&str] = &["IDLE", "ARMED", "DONE", "TIMEOUT"];
            const PROTOCOLS: &[&str] = &["NONE", "NEC", "RC5", "RC6", "HASH"];
            event_line(
                seq,
                "IrLearn",
                format!(
                    "{} protocol={} code=0x{code:08X}",
                    named(STATES, *state),
                    named(PROTOCOLS, *protocol)
                ),
            )
        }
        Event::AdatInputState {
            state,
            rate_hz,
            clock_mode,
        } => {
            const STATES: &[&str] = &["INACTIVE", "ACQUIRING", "SYNCING", "LOCKED", "RELOCKING"];
            let mode = if *clock_mode == 1 { "slave" } else { "master" };
            event_line(
                seq,
                "AdatInputState",
                format!("{} rate={rate_hz} mode={mode}", named(STATES, *state)),
            )
        }
        Event::CsAux {
            slot,
            state,
            level_q8,
            source,
        } => event_line(
            seq,
            "CsAux",
            format!(
                "slot={slot} state={state} level={:.1}% src={}",
                f64::from(*level_q8) / 256.0,
                source_label(*source)
            ),
        ),
        // The session's stand-in for a packet it could not decode holds the
        // whole packet (dspi_session::notify, the reader).
        Event::Unknown { id: 0xFF, bytes } if note.lost && note.seq == 0 => {
            format!("undecodable packet ({} bytes): {}", bytes.len(), hex(bytes))
        }
        Event::Unknown { id, bytes } => undecoded_event(seq, *id, bytes),
    };
    if note.lost && !matches!(note.event, Event::Unknown { id: 0xFF, .. }) {
        line.push_str("  (after a gap)");
    }
    if let Some(peer) = note.origin.filter(|&o| o != 0) {
        let _ = write!(line, "  link session {peer}");
    }
    line
}

/// `[seq] v2.<name>` padded to a column, then `details`.
fn event_line(seq: u8, name: &str, details: String) -> String {
    let event = format!("v2.{name}");
    format!("[{seq:3}] {event:<28} {details}")
        .trim_end()
        .to_string()
}

/// The events the session passes on undecoded: the analyser configuration,
/// the two name events (notify.h 0x0D, 0x0F, 0x10), and anything newer.
fn undecoded_event(seq: u8, id: u8, bytes: &[u8]) -> String {
    let id16 = u16::from(id);
    if id16 == nfy::NOTIFY_EVT_RTA_CONFIG {
        // [RtaConfig as applied (12), src]
        let src = Source::from_raw(bytes.get(RtaConfig::SIZE).copied().unwrap_or(0));
        let details = match RtaConfig::decode(bytes) {
            Ok(c) => format!(
                "tap={} mask=0x{:04X} order={} avg={}ms decay={}dB/s flags=0x{:02X} src={}",
                rta_tap(c.tap),
                c.channel_mask,
                c.fft_order,
                c.avg_ms,
                c.peak_decay_db_s,
                c.flags,
                source_label(src)
            ),
            Err(_) => format!("(unreadable) {}", hex(bytes)),
        };
        return event_line(seq, "RtaConfig", details);
    }
    if id16 == nfy::NOTIFY_EVT_INPUT_SOURCE_NAME || id16 == nfy::NOTIFY_EVT_DEVICE_NAME {
        // [source_id, src, 0, 0, name(32)]
        let src = source_label(Source::from_raw(bytes.get(1).copied().unwrap_or(0)));
        let name = text(bytes.get(4..).unwrap_or_default());
        return if id16 == nfy::NOTIFY_EVT_DEVICE_NAME {
            event_line(seq, "DeviceName", format!("name={name} src={src}"))
        } else {
            let input = bytes.first().copied().unwrap_or(0);
            event_line(
                seq,
                "InputSourceName",
                format!("source={input} name={name} src={src}"),
            )
        };
    }
    format!("[{seq:3}] v2?evt=0x{id:02X}  {}", hex(bytes))
}

fn rta_tap(tap: u8) -> String {
    match u16::from(tap) {
        rta::RTA_TAP_INPUT => "in".into(),
        rta::RTA_TAP_OUTPUT => "out".into(),
        rta::RTA_TAP_NONE => "none".into(),
        _ => tap.to_string(),
    }
}

/// The `ParamSource` tag as notify.h spells it, short.
pub(crate) fn source_label(source: Source) -> String {
    match source {
        Source::Unknown => "?".into(),
        Source::HostSet => "HOST".into(),
        Source::BulkSet => "BULK".into(),
        Source::Preset => "PRESET".into(),
        Source::Factory => "FACTORY".into(),
        Source::Gpio => "GPIO".into(),
        Source::Internal => "INTERNAL".into(),
        Source::Uac1 => "UAC1".into(),
        Source::Uart => "UART".into(),
        Source::I2c => "I2C".into(),
        Source::Midi => "MIDI".into(),
        Source::Other(raw) => format!("src=0x{raw:02X}"),
    }
}

/// `names[n]`, or the number itself when it is past the end.
fn named(names: &[&str], n: u8) -> String {
    names
        .get(usize::from(n))
        .map_or_else(|| n.to_string(), |s| (*s).to_string())
}

// ── Bulk-image fields ────────────────────────────────────────────────

/// How a field's bytes read.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// `f32` with a unit suffix (may be empty).
    Float(&'static str),
    /// `f32` dB where -128 and below means muted.
    Volume,
    /// `u8`, 0 off and 1 on.
    Flag,
    Byte,
    Signed,
    /// `u16`.
    Word,
    Mask8,
    Mask16,
    /// A NUL-padded name of this many bytes.
    Text(usize),
}

impl Kind {
    const fn size(self) -> usize {
        match self {
            Kind::Float(_) | Kind::Volume => 4,
            Kind::Flag | Kind::Byte | Kind::Signed | Kind::Mask8 => 1,
            Kind::Word | Kind::Mask16 => 2,
            Kind::Text(n) => n,
        }
    }

    // -- WP SHOWFIX -- (finding 2)
    /// `bytes` as a number, for a number or a flag.
    fn number(self, bytes: &[u8]) -> Option<f64> {
        Some(match self {
            Kind::Float(_) | Kind::Volume => f64::from(f32_le(bytes, 0)?),
            Kind::Flag | Kind::Byte => f64::from(*bytes.first()?),
            Kind::Signed => f64::from(i8::from_le_bytes([*bytes.first()?])),
            Kind::Word => f64::from(u16_le(bytes, 0)?),
            Kind::Mask8 | Kind::Mask16 | Kind::Text(_) => return None,
        })
    }

    /// `bytes` read as this kind, when they have its size.
    fn show(self, bytes: &[u8]) -> Option<String> {
        if let Kind::Text(n) = self {
            return (!bytes.is_empty() && bytes.len() <= n).then(|| text(bytes));
        }
        if bytes.len() != self.size() {
            return None;
        }
        Some(match self {
            Kind::Float(unit) => format!("{}{unit}", f32_le(bytes, 0)?),
            Kind::Volume => volume(f32_le(bytes, 0)?),
            Kind::Flag => match bytes.first()? {
                0 => "off".into(),
                1 => "on".into(),
                n => n.to_string(),
            },
            Kind::Byte => bytes.first()?.to_string(),
            Kind::Signed => i8::from_le_bytes([*bytes.first()?]).to_string(),
            Kind::Word => u16_le(bytes, 0)?.to_string(),
            Kind::Mask8 => format!("0x{:02X}", bytes.first()?),
            Kind::Mask16 => format!("0x{:04X}", u16_le(bytes, 0)?),
            Kind::Text(_) => return None,
        })
    }
}

/// One field of a section's record: `count` consecutive values of `kind`
/// from byte `at`. An empty name stands for the record itself
/// (`channel_names[3]` rather than `channel_names.names[3]`).
#[derive(Debug, Clone, Copy)]
struct Field {
    name: &'static str,
    at: usize,
    kind: Kind,
    count: usize,
}

const fn one(name: &'static str, at: usize, kind: Kind) -> Field {
    Field {
        name,
        at,
        kind,
        count: 1,
    }
}

const fn many(name: &'static str, at: usize, kind: Kind, count: usize) -> Field {
    Field {
        name,
        at,
        kind,
        count,
    }
}

/// A record that reads better whole when a packet carries all of it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Whole {
    Band,
    Crosspoint,
}

/// A section's layout: an array of `dims` records of `stride` bytes (no
/// dims: one record, the whole section).
struct Layout {
    dims: &'static [usize],
    stride: usize,
    fields: &'static [Field],
    whole: Option<Whole>,
}

const CHANNELS: usize = wire::WIRE_MAX_CHANNELS as usize;
const INPUTS: usize = wire::WIRE_MAX_INPUT_CHANNELS as usize;
const OUTPUTS: usize = wire::WIRE_MAX_OUTPUT_CHANNELS as usize;
const BANDS: usize = wire::WIRE_MAX_BANDS as usize;
const XOVER_BANDS: usize = wire::WIRE_MAX_XOVER_BANDS as usize;
const PINS: usize = wire::WIRE_MAX_PIN_OUTPUTS as usize;
const SPDIF_SLOTS: usize = wire::WIRE_MAX_SPDIF_INSTANCES as usize;
const NAME_LEN: usize = wire::WIRE_NAME_LEN as usize;

const DB: Kind = Kind::Float(" dB");
const DBFS: Kind = Kind::Float(" dBFS");
const HZ: Kind = Kind::Float(" Hz");
const MS: Kind = Kind::Float(" ms");
const PCT: Kind = Kind::Float("%");

/// WireBandParams, for both the EQ and the crossovers.
const BAND: &[Field] = &[
    one("type", 0, Kind::Byte),
    one("bypass", 1, Kind::Flag),
    one("freq", 4, HZ),
    one("q", 8, Kind::Float("")),
    one("gain_db", 12, DB),
];

/// A section that is one record.
const fn single(fields: &'static [Field]) -> Layout {
    Layout {
        dims: &[],
        stride: 0,
        fields,
        whole: None,
    }
}

/// The record layout of each section, by its name in `SECTIONS`
/// (bulk_params.h).
const LAYOUTS: &[(&str, Layout)] = &[
    (
        "header",
        single(&[
            one("format_version", 0, Kind::Byte),
            one("platform_id", 1, Kind::Byte),
            one("num_channels", 2, Kind::Byte),
            one("num_output_channels", 3, Kind::Byte),
            one("num_input_channels", 4, Kind::Byte),
            one("max_bands", 5, Kind::Byte),
            one("payload_length", 6, Kind::Word),
            one("fw_version_major", 8, Kind::Word),
            one("fw_version_minor", 10, Kind::Word),
        ]),
    ),
    (
        "global",
        single(&[
            one("preamp_gain_db", 0, DB),
            one("bypass", 4, Kind::Flag),
            one("loudness_enabled", 5, Kind::Flag),
            one("loudness_output_mask", 6, Kind::Mask16),
            one("loudness_ref_spl", 8, Kind::Float(" dB SPL")),
            one("loudness_intensity_pct", 12, PCT),
        ]),
    ),
    (
        "crossfeed",
        single(&[
            one("enabled", 0, Kind::Flag),
            one("preset", 1, Kind::Byte),
            one("itd_enabled", 2, Kind::Flag),
            one("output_pair_mask", 3, Kind::Mask8),
            one("custom_fc", 4, HZ),
            one("custom_feed_db", 8, DB),
            many("pair_map", 12, Kind::Mask8, 4),
        ]),
    ),
    (
        "legacy",
        single(&[many("gain_db", 0, DB, 3), many("mute", 12, Kind::Flag, 3)]),
    ),
    ("delays", single(&[many("delay_ms", 0, MS, CHANNELS)])),
    (
        "crosspoints",
        Layout {
            dims: &[INPUTS, OUTPUTS],
            stride: 8,
            fields: &[
                one("enabled", 0, Kind::Flag),
                one("phase_invert", 1, Kind::Flag),
                one("gain_db", 4, DB),
            ],
            whole: Some(Whole::Crosspoint),
        },
    ),
    (
        "outputs",
        Layout {
            dims: &[OUTPUTS],
            stride: 12,
            fields: &[
                one("enabled", 0, Kind::Flag),
                one("mute", 1, Kind::Flag),
                one("gain_db", 4, DB),
                one("delay_ms", 8, MS),
            ],
            whole: None,
        },
    ),
    (
        "pins",
        single(&[
            one("num_pin_outputs", 0, Kind::Byte),
            many("pins", 1, Kind::Byte, PINS),
        ]),
    ),
    (
        "eq",
        Layout {
            dims: &[CHANNELS, BANDS],
            stride: 16,
            fields: BAND,
            whole: Some(Whole::Band),
        },
    ),
    (
        "channel_names",
        single(&[many("", 0, Kind::Text(NAME_LEN), CHANNELS)]),
    ),
    (
        "i2s_config",
        single(&[
            many("output_types", 0, Kind::Byte, SPDIF_SLOTS),
            one("bck_pin", 4, Kind::Byte),
            one("mck_pin", 5, Kind::Byte),
            one("mck_enabled", 6, Kind::Flag),
            one("mck_multiplier", 7, Kind::Byte),
            one("clock_pin_mode_p1", 8, Kind::Byte),
            one("bck_pin_slave", 9, Kind::Byte),
        ]),
    ),
    (
        "leveller",
        single(&[
            one("enabled", 0, Kind::Flag),
            one("speed", 1, Kind::Byte),
            one("lookahead", 2, Kind::Flag),
            one("amount", 4, PCT),
            one("max_gain_db", 8, DB),
            one("gate_threshold_db", 12, DBFS),
            one("detector_mask", 16, Kind::Mask8),
            one("apply_mask", 17, Kind::Mask8),
        ]),
    ),
    ("preamp", single(&[many("preamp_db", 0, DB, INPUTS)])),
    (
        "master_volume",
        single(&[one("master_volume_db", 0, Kind::Volume)]),
    ),
    (
        "input_config",
        single(&[
            one("input_source", 0, Kind::Byte),
            one("spdif_rx_pin", 1, Kind::Byte),
            one("i2s_rx_pin", 2, Kind::Byte),
            one("i2s_input_rate", 3, Kind::Byte),
            one("i2s_input_channels", 4, Kind::Byte),
            many("i2s_rx_pin_ext", 5, Kind::Byte, 3),
            many("spdif_rx_pin_ext", 8, Kind::Byte, 3),
            one("spdif_rx_enabled_ext_p1", 11, Kind::Byte),
            one("i2s_clock_mode", 12, Kind::Byte),
            one("adat_input_pin", 13, Kind::Byte),
            one("adat_input_enabled_p1", 14, Kind::Byte),
            one("adat_clock_mode_p1", 15, Kind::Byte),
        ]),
    ),
    (
        "lg_sound_sync",
        single(&[
            one("enabled", 0, Kind::Flag),
            one("present", 1, Kind::Flag),
            one("volume", 2, Kind::Byte),
            one("muted", 3, Kind::Flag),
        ]),
    ),
    (
        "user_volume",
        single(&[
            one("user_volume_db", 0, Kind::Volume),
            one("user_mute", 4, Kind::Flag),
        ]),
    ),
    (
        "dac_hw_mute",
        single(&[
            one("enabled", 0, Kind::Flag),
            one("active_low", 1, Kind::Flag),
            one("pin", 2, Kind::Byte),
            one("hold_ms", 4, Kind::Word),
            one("release_ms", 6, Kind::Word),
        ]),
    ),
    (
        "crossovers",
        Layout {
            dims: &[CHANNELS, XOVER_BANDS],
            stride: 16,
            fields: BAND,
            whole: Some(Whole::Band),
        },
    ),
    (
        "adat_config",
        single(&[one("enabled", 0, Kind::Flag), one("pin", 1, Kind::Byte)]),
    ),
    (
        "psybass",
        single(&[
            one("enabled", 0, Kind::Flag),
            one("output_mask", 2, Kind::Mask16),
            one("cutoff_hz", 4, HZ),
            one("harmonics_db", 8, DB),
            one("drive_db", 12, DB),
            one("character_pct", 16, PCT),
            one("original_db", 20, DB),
        ]),
    ),
    (
        "upmix",
        single(&[
            one("enabled", 0, Kind::Flag),
            one("center_mode", 1, Kind::Byte),
            one("surround_mode", 2, Kind::Byte),
            one("presence_q1", 3, Kind::Signed),
            one("strength_pct", 4, PCT),
            one("center_width_pct", 8, PCT),
            one("corr_threshold_pct", 12, PCT),
            one("attack_ms", 16, MS),
            one("release_ms", 20, MS),
            one("detector_hpf_hz", 24, HZ),
            one("surround_delay_ms", 28, MS),
            one("surround_hpf_hz", 32, HZ),
            one("surround_lpf_hz", 36, HZ),
            one("decorr_pct", 40, PCT),
        ]),
    ),
    (
        "subharm",
        single(&[
            one("enabled", 0, Kind::Flag),
            one("output_mask", 2, Kind::Mask16),
            one("low_db", 4, DB),
            one("high_db", 8, DB),
            one("boost_db", 12, DB),
            one("top_db", 16, DB),
            one("select_depth", 20, PCT),
            one("select_hold_ms", 24, MS),
            one("ceiling_db", 28, DBFS),
            one("select_mode", 32, Kind::Byte),
            one("link_pairs", 33, Kind::Flag),
        ]),
    ),
    (
        "tube",
        single(&[
            one("enabled", 0, Kind::Flag),
            one("tube_type", 1, Kind::Byte),
            one("rectifier", 2, Kind::Byte),
            one("xfmr_enabled", 3, Kind::Flag),
            one("output_mask", 4, Kind::Mask16),
            one("drive_db", 8, DB),
            one("bias_pct", 12, PCT),
            one("asym_db", 16, DB),
            one("hardness_pct", 20, PCT),
            one("sag_pct", 24, PCT),
            one("xfmr_damping", 28, Kind::Float("")),
            one("xfmr_res_hz", 32, HZ),
            one("mix_pct", 36, PCT),
            one("trim_db", 40, DB),
        ]),
    ),
    (
        "limiter",
        Layout {
            dims: &[OUTPUTS],
            stride: 12,
            fields: &[
                one("enabled", 0, Kind::Flag),
                one("link_group", 1, Kind::Byte),
                one("threshold_db", 4, DBFS),
                one("release_ms", 8, MS),
            ],
            whole: None,
        },
    ),
];

/// The record layout of the section called `section` (bulk_params.h).
fn layout(section: &str) -> Option<&'static Layout> {
    LAYOUTS
        .iter()
        .find(|(name, _)| *name == section)
        .map(|(_, l)| l)
}

/// The field a `PARAM_CHANGED` at `offset` touched, and its new value.
fn param(offset: usize, bytes: &[u8]) -> (String, String) {
    match locate(offset, bytes) {
        Ok(at) => (at.path(), at.shown),
        Err(fallback) => fallback,
    }
}

// -- WP SHOWFIX -- (finding 2) the same lookup, in parts, for the event log's
// plain words (`crate::events::words`)
/// Where a `PARAM_CHANGED` landed: the section, the record's indices, the
/// field and its element, and the new value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParamAt {
    /// The section's name in bulk_params.h (`user_volume`, `eq`).
    pub(crate) section: &'static str,
    /// The record's indices, outermost first (`eq`: channel, band).
    pub(crate) index: Vec<usize>,
    /// The field (empty: the record itself, or the whole record).
    pub(crate) field: &'static str,
    /// The element of an array field (`preamp_db[2]`).
    pub(crate) element: Option<usize>,
    /// The packet carried the whole record (a band, a crosspoint).
    pub(crate) whole: bool,
    /// The value as the monitor shows it ("-20.00 dB", "on").
    pub(crate) shown: String,
    /// The value as a number, for a number or a flag (1 on, 0 off).
    pub(crate) number: Option<f64>,
}

impl ParamAt {
    /// `section[i][j].field[k]`, as the monitor names it.
    fn path(&self) -> String {
        let mut path = self.section.to_string();
        for i in &self.index {
            let _ = write!(path, "[{i}]");
        }
        if !self.field.is_empty() {
            let _ = write!(path, ".{}", self.field);
        }
        if let Some(e) = self.element {
            let _ = write!(path, "[{e}]");
        }
        path
    }
}

/// The parts of a `PARAM_CHANGED` at `offset`, or the monitor's fallback
/// words (name, value) where they cannot be told.
pub(crate) fn param_at(offset: usize, bytes: &[u8]) -> Option<ParamAt> {
    locate(offset, bytes).ok()
}

fn locate(offset: usize, bytes: &[u8]) -> std::result::Result<ParamAt, (String, String)> {
    let raw = || hex(bytes);
    let Some(&(section, base, len)) = SECTIONS
        .iter()
        .find(|(_, start, len)| offset >= *start && offset < start.saturating_add(*len))
    else {
        return Err((format!("offset=0x{offset:04X}"), raw()));
    };
    let rel = offset.saturating_sub(base);
    let Some(shape) = layout(section) else {
        return Err((format!("{section}+0x{rel:X}"), raw()));
    };
    // One record unless the section is an array of them.
    let stride = if shape.dims.is_empty() {
        len
    } else {
        shape.stride
    };
    let record = rel.checked_div(stride).unwrap_or(0);
    let inside = rel.checked_rem(stride).unwrap_or(rel);
    let mut at = ParamAt {
        section,
        index: indices(record, shape.dims),
        field: "",
        element: None,
        whole: false,
        shown: String::new(),
        number: None,
    };
    if inside == 0
        && bytes.len() == stride
        && let Some(whole) = shape.whole
        && let Some(value) = show_whole(whole, bytes)
    {
        at.whole = true;
        at.shown = value;
        return Ok(at);
    }
    let Some(field) = shape.fields.iter().find(|f| {
        let end = f.at.saturating_add(f.kind.size().saturating_mul(f.count));
        inside >= f.at && inside < end
    }) else {
        return Err((format!("{}+0x{inside:X}", at.path()), raw()));
    };
    at.field = field.name;
    let into = inside.saturating_sub(field.at);
    let size = field.kind.size();
    if field.count > 1 {
        at.element = Some(into.checked_div(size).unwrap_or(0));
    }
    let within = into.checked_rem(size).unwrap_or(0);
    if within != 0 {
        return Err((format!("{}+0x{within:X}", at.path()), raw()));
    }
    at.shown = field.kind.show(bytes).unwrap_or_else(raw);
    at.number = field.kind.number(bytes);
    Ok(at)
}

/// `record` as one index per dimension, outermost first.
fn indices(mut record: usize, dims: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(dims.len());
    for &d in dims.iter().rev() {
        out.push(record.checked_rem(d).unwrap_or(record));
        record = record.checked_div(d).unwrap_or(0);
    }
    out.reverse();
    out
}

fn show_whole(whole: Whole, b: &[u8]) -> Option<String> {
    Some(match whole {
        Whole::Band => format!(
            "type={} bypass={} f={} Hz Q={} gain={} dB",
            b.first()?,
            b.get(1)?,
            f32_le(b, 4)?,
            f32_le(b, 8)?,
            f32_le(b, 12)?
        ),
        Whole::Crosspoint => format!(
            "enabled={} invert={} gain={} dB",
            b.first()?,
            b.get(1)?,
            f32_le(b, 4)?
        ),
    })
}

// ── Value helpers ────────────────────────────────────────────────────

fn f32_le(b: &[u8], at: usize) -> Option<f32> {
    let four: [u8; 4] = b.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(f32::from_le_bytes(four))
}

fn u16_le(b: &[u8], at: usize) -> Option<u16> {
    let two: [u8; 2] = b.get(at..at.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_le_bytes(two))
}

/// A volume in dB; -128 is the firmware's mute sentinel.
fn volume(db: f32) -> String {
    if db <= -128.0 {
        "MUTE".into()
    } else {
        format!("{db:+.2} dB")
    }
}

/// A NUL-padded name, quoted.
fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    format!(
        "\"{}\"",
        String::from_utf8_lossy(b.get(..end).unwrap_or_default())
    )
}

fn hex(b: &[u8]) -> String {
    if b.is_empty() {
        return "(empty)".into();
    }
    let mut s = String::with_capacity(b.len().saturating_mul(3));
    for (i, byte) in b.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{byte:02X}");
    }
    s
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_clean1 {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use dspi_proto::generated as g;
    use dspi_transport::clock::Instant;

    use super::*;

    fn note(seq: u8, event: Event) -> Notification {
        Notification {
            seq,
            event,
            lost: false,
            origin: None,
            received: Instant::now(),
        }
    }

    fn changed(offset: usize, source: Source, bytes: &[u8]) -> String {
        describe(&note(
            9,
            Event::ParamChanged {
                offset: offset as u16,
                source,
                bytes: bytes.to_vec(),
            },
        ))
    }

    fn unknown(id: u16, bytes: Vec<u8>) -> String {
        describe(&note(
            4,
            Event::Unknown {
                id: id as u8,
                bytes,
            },
        ))
    }

    #[test]
    fn clean1_param_changes_are_named() {
        let line = changed(
            g::OFF_MASTER_VOLUME,
            Source::HostSet,
            &(-7.0f32).to_le_bytes(),
        );
        assert!(
            line.contains("[HOST    ] master_volume.master_volume_db"),
            "{line}"
        );
        assert!(line.ends_with("-7.00 dB"), "{line}");
        assert!(
            changed(
                g::OFF_MASTER_VOLUME,
                Source::Gpio,
                &(-128.0f32).to_le_bytes()
            )
            .ends_with("MUTE")
        );

        // Arrays of records resolve every index.
        let out3_gain = g::OFF_OUTPUTS + 3 * 12 + 4;
        let line = changed(out3_gain, Source::Gpio, &(-3.5f32).to_le_bytes());
        assert!(line.contains("[GPIO    ] outputs[3].gain_db"), "{line}");
        assert!(line.ends_with("-3.5 dB"), "{line}");

        let band = g::OFF_EQ + (2 * BANDS + 5) * 16;
        let mut whole = vec![1u8, 0, 0, 0];
        for v in [1000.0f32, 0.7, -2.0] {
            whole.extend(v.to_le_bytes());
        }
        let line = changed(band, Source::Preset, &whole);
        assert!(line.contains("eq[2][5] "), "{line}");
        assert!(line.contains("f=1000 Hz"), "{line}");
        assert!(
            changed(band + 4, Source::HostSet, &440.0f32.to_le_bytes()).contains("eq[2][5].freq")
        );

        let xp = g::OFF_CROSSPOINTS + (OUTPUTS + 2) * 8 + 4;
        assert!(
            changed(xp, Source::HostSet, &0.0f32.to_le_bytes())
                .contains("crosspoints[1][2].gain_db")
        );
        assert!(
            changed(g::OFF_DELAYS + 8, Source::HostSet, &1.5f32.to_le_bytes())
                .contains("delays.delay_ms[2]")
        );
        let mut name = b"Sub".to_vec();
        name.resize(NAME_LEN, 0);
        let line = changed(g::OFF_CHANNEL_NAMES + NAME_LEN, Source::HostSet, &name);
        assert!(line.contains("channel_names[1]"), "{line}");
        assert!(line.ends_with("\"Sub\""), "{line}");
        assert!(
            changed(g::OFF_USER_VOLUME + 4, Source::Uac1, &[1])
                .contains("[UAC1    ] user_volume.user_mute")
        );
        assert!(
            changed(
                g::OFF_LIMITER + 12 + 4,
                Source::HostSet,
                &(-1.0f32).to_le_bytes()
            )
            .contains("limiter[1].threshold_db")
        );
        assert!(changed(g::OFF_GLOBAL + 6, Source::HostSet, &[0x05, 0]).ends_with("0x0005"));
    }

    #[test]
    fn clean1_layouts_match_the_wire_tables() {
        // Every field fits its section and record.
        for (section, _, len) in SECTIONS {
            let Some(shape) = layout(section) else {
                panic!("{section} has no layout");
            };
            let stride = if shape.dims.is_empty() {
                len
            } else {
                shape.stride
            };
            if !shape.dims.is_empty() {
                assert_eq!(
                    shape.dims.iter().product::<usize>() * stride,
                    len,
                    "{section}"
                );
            }
            for f in shape.fields {
                assert!(
                    f.at + f.kind.size() * f.count <= stride,
                    "{section}.{}",
                    f.name
                );
            }
        }
        // The tables dspi-proto keeps for the sections that changed shape.
        let check = |section: &str, table: &[(&str, usize, usize)]| {
            let shape = layout(section).unwrap();
            for (name, at, len) in table {
                if name.starts_with("reserved") {
                    continue;
                }
                let f = shape.fields.iter().find(|f| f.name == *name).unwrap();
                assert_eq!(
                    (f.at, f.kind.size() * f.count),
                    (*at, *len),
                    "{section}.{name}"
                );
            }
        };
        check("input_config", &dspi_proto::wire::INPUT_CONFIG_FIELDS);
        check("subharm", &dspi_proto::wire::SUBHARM_FIELDS);
        check("tube", &dspi_proto::wire::TUBE_FIELDS);
        check("limiter", &dspi_proto::wire::LIMITER_OUTPUT_FIELDS);
    }

    #[test]
    fn clean1_events_and_fallbacks() {
        let v1 = describe(&note(0, Event::MasterVolume(-12.0)));
        assert!(v1.starts_with("v1.MasterVolume"), "{v1}");
        assert!(v1.ends_with("-12.00 dB"), "{v1}");

        let preset = describe(&note(7, Event::PresetLoaded { slot: 2 }));
        assert!(preset.starts_with("[  7] v2.PresetLoaded"), "{preset}");
        assert!(preset.ends_with("slot=2"), "{preset}");
        let bulk = describe(&note(
            8,
            Event::BulkInvalidated {
                source: Source::BulkSet,
            },
        ));
        assert!(
            bulk.contains("v2.BulkInvalidated") && bulk.ends_with("source=BULK"),
            "{bulk}"
        );
        let siggen = describe(&note(
            1,
            Event::SiggenState {
                state: 2,
                reason: 9,
                signal_type: 3,
                channel: 0xFF,
            },
        ));
        assert!(siggen.contains("RUN reason=9 type=3 ch=-"), "{siggen}");
        let aux = describe(&note(
            1,
            Event::CsAux {
                slot: 4,
                state: 1,
                level_q8: 128 * 256,
                source: Source::Gpio,
            },
        ));
        assert!(
            aux.contains("slot=4 state=1 level=128.0% src=GPIO"),
            "{aux}"
        );
        let ir = describe(&note(
            1,
            Event::IrLearn {
                state: 2,
                protocol: 1,
                code: 0xABCD,
            },
        ));
        assert!(ir.contains("DONE protocol=NEC code=0x0000ABCD"), "{ir}");

        // Names arrive as events the session does not decode.
        let mut p = vec![0u8, nfy::PARAM_SRC_GPIO as u8, 0, 0];
        let mut field = [0u8; 32];
        field[..7].copy_from_slice(b"Kitchen");
        p.extend(field);
        let line = unknown(nfy::NOTIFY_EVT_DEVICE_NAME, p.clone());
        assert!(
            line.contains("v2.DeviceName") && line.contains("name=\"Kitchen\" src=GPIO"),
            "{line}"
        );
        p[0] = 3;
        let line = unknown(nfy::NOTIFY_EVT_INPUT_SOURCE_NAME, p);
        assert!(line.contains("source=3 name=\"Kitchen\""), "{line}");

        // Fallbacks: an unknown event, an offset past the image, a payload
        // that does not fit the field, an offset inside a field, a gap.
        assert_eq!(unknown(0x42, vec![1, 0xAB]), "[  4] v2?evt=0x42  01 AB");
        let past = changed(g::BULK_SIZE + 2, Source::Other(0x33), &[1, 2]);
        assert!(past.contains("[src=0x33] offset=0x"), "{past}");
        assert!(past.ends_with("01 02"), "{past}");
        assert!(changed(g::OFF_MASTER_VOLUME, Source::HostSet, &[1, 2]).ends_with("01 02"));
        assert!(
            changed(g::OFF_MASTER_VOLUME + 1, Source::HostSet, &[1])
                .contains("master_volume.master_volume_db+0x1")
        );
        assert!(
            changed(g::OFF_MASTER_VOLUME + 8, Source::HostSet, &[]).contains("master_volume+0x8")
        );
        let mut lost = note(5, Event::PresetLoaded { slot: 0 });
        lost.lost = true;
        lost.origin = Some(12);
        let line = describe(&lost);
        assert!(
            line.contains("(after a gap)") && line.ends_with("link session 12"),
            "{line}"
        );
        let mut bad = note(
            0,
            Event::Unknown {
                id: 0xFF,
                bytes: vec![3, 2, 0],
            },
        );
        bad.lost = true;
        assert_eq!(describe(&bad), "undecodable packet (3 bytes): 03 02 00");
    }

    #[test]
    fn clean1_the_monitor_names_the_analyser_configuration_event() {
        let cfg = RtaConfig {
            tap: g::rta::RTA_TAP_OUTPUT as u8,
            channel_mask: 0x0003,
            fft_order: 10,
            avg_ms: 250,
            peak_decay_db_s: 12,
            flags: 0,
        };
        let mut bytes = cfg.encode().to_vec();
        bytes.push(nfy::PARAM_SRC_MIDI as u8);
        let line = unknown(nfy::NOTIFY_EVT_RTA_CONFIG, bytes);
        assert!(line.contains("v2.RtaConfig"), "{line}");
        assert!(
            line.contains("tap=out mask=0x0003 order=10 avg=250ms decay=12dB/s"),
            "{line}"
        );
        assert!(line.ends_with("src=MIDI"), "{line}");
        // Short or of another version: named, with its bytes.
        let line = unknown(nfy::NOTIFY_EVT_RTA_CONFIG, vec![9, 1]);
        assert!(
            line.contains("v2.RtaConfig") && line.ends_with("09 01"),
            "{line}"
        );
    }

    #[test]
    fn clean1_no_offset_or_payload_panics() {
        let payloads: Vec<Vec<u8>> = (0..=40usize)
            .chain([52, 64, 255, 300])
            .map(|n| (0..n).map(|i| (i * 37) as u8).collect())
            .collect();
        for offset in 0..=u16::MAX {
            if offset as usize > g::BULK_SIZE + 64 && offset % 997 != 0 {
                continue;
            }
            for p in payloads
                .iter()
                .filter(|p| p.len() <= 17 || offset % 61 == 0)
            {
                let line = changed(offset as usize, Source::Other(0xEE), p);
                assert!(!line.is_empty());
            }
        }
        for id in 0..=u8::MAX {
            for p in &payloads {
                let mut n = note(
                    id,
                    Event::Unknown {
                        id,
                        bytes: p.clone(),
                    },
                );
                n.lost = id % 2 == 0;
                assert!(!describe(&n).is_empty());
            }
        }
        for v in [f32::NAN, f32::INFINITY, f32::MIN, -128.0, 0.0] {
            assert!(!describe(&note(0, Event::MasterVolume(v))).is_empty());
        }
    }

    #[test]
    fn clean1_the_log_is_bounded_and_ids_survive_a_clear() {
        let mut log = MonitorLog::default();
        assert_eq!(
            log.after(0),
            MonitorBatch {
                last_id: 0,
                entries: vec![]
            }
        );
        let total = MonitorLog::LIMIT + 25;
        for i in 0..total {
            log.record(&note(i as u8, Event::PresetLoaded { slot: 1 }));
        }
        let all = log.after(0);
        assert_eq!(all.entries.len(), MonitorLog::LIMIT);
        assert_eq!(all.entries[0].id, 26, "the oldest went first");
        assert_eq!(all.last_id, total as u64);
        assert!(all.entries.windows(2).all(|w| w[1].id == w[0].id + 1));

        let tail = log.after(total as u64 - 3);
        assert_eq!(tail.entries.len(), 3);
        assert_eq!(tail.last_id, total as u64);
        let none = log.after(total as u64);
        assert!(none.entries.is_empty());
        assert_eq!(none.last_id, total as u64);

        log.entries.clear();
        assert_eq!(log.after(5).last_id, 5, "nothing new: the caller's id");
        log.record(&note(0, Event::InputFormat { channels: 2 }));
        let after = log.after(total as u64);
        assert_eq!(after.entries.len(), 1);
        assert_eq!(after.entries[0].id, total as u64 + 1, "ids carry on");
    }

    #[test]
    fn clean1_clear_through_the_core() {
        let core = DspiCore::new();
        core.lock()
            .monitor
            .record(&note(1, Event::PresetLoaded { slot: 0 }));
        let first = core.monitor_read(0);
        assert_eq!(first.entries.len(), 1);
        core.monitor_clear();
        assert_eq!(core.monitor_read(first.last_id).last_id, first.last_id);
        assert!(core.monitor_read(0).entries.is_empty());
    }

    #[test]
    fn clean1_entries_carry_the_receive_time() {
        let mut log = MonitorLog::default();
        let mut old = note(1, Event::PresetLoaded { slot: 0 });
        old.received = Instant::now() - Duration::from_secs(30);
        log.record(&old);
        log.record(&note(2, Event::PresetLoaded { slot: 0 }));
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let got = log.after(0).entries;
        assert!(
            (got[0].time_ms - (now_ms - 30_000)).abs() < 2_000,
            "{got:?}"
        );
        assert!((got[1].time_ms - now_ms).abs() < 2_000, "{got:?}");
    }
}
