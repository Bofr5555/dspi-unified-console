//! The notification monitor (the Interrupt Monitor window): a log of the
//! device's notification packets, one line each in the macOS Console's
//! format, with `PARAM_CHANGED` named by its bulk-image offset.
//!
//! [`DspiCore::process_notifications`] records every packet it drains (the
//! reader thread drops the idle keep-alive), stamped with the time the
//! reader received it. The log keeps the last [`MonitorLog::LIMIT`] entries;
//! ids keep counting across [`DspiCore::monitor_clear`].
//!
//! Owned by WP C7 (docs/core-merge/PLAN.md).

use std::collections::VecDeque;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use dspi_proto::generated as g;
use dspi_session::notify::{Event, Notification, Source};

use crate::DspiCore;
use crate::notify::{EVT_DEVICE_NAME, EVT_INPUT_SOURCE_NAME};
use crate::records::*;

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// The log entries recorded after `after_id`, oldest first, and the id
    /// to pass next time (`after_id` itself when there are none). Pass 0
    /// for everything still held. Entries are recorded when
    /// [`DspiCore::process_notifications`] drains the queue. Never fails.
    pub fn monitor_read(&self, after_id: u64) -> MonitorBatch {
        self.lock().monitor.read_after(after_id)
    }

    /// Forget the logged entries. Ids are not reused: a reader's last id
    /// stays valid.
    pub fn monitor_clear(&self) {
        self.lock().monitor.clear();
    }
}

/// The notification monitor's log: what arrived, decoded, with ids for
/// `monitor_read(after_id)`. Bounded.
#[derive(Default)]
pub(crate) struct MonitorLog {
    entries: VecDeque<MonitorEntry>,
    /// The id of the newest entry ever recorded (0 before the first).
    last_id: u64,
}

impl MonitorLog {
    /// The most entries kept; the oldest go first.
    pub(crate) const LIMIT: usize = 2000;

    /// Log one drained notification.
    pub(crate) fn record(&mut self, note: &Notification) {
        self.push(epoch_ms(note.received), describe(note));
    }

    fn push(&mut self, time_ms: i64, text: String) {
        self.last_id = self.last_id.saturating_add(1);
        while self.entries.len() >= Self::LIMIT {
            self.entries.pop_front();
        }
        self.entries.push_back(MonitorEntry {
            id: self.last_id,
            time_ms,
            text,
        });
    }

    fn read_after(&self, after_id: u64) -> MonitorBatch {
        let entries: Vec<MonitorEntry> = self
            .entries
            .iter()
            .filter(|e| e.id > after_id)
            .cloned()
            .collect();
        MonitorBatch {
            last_id: entries.last().map_or(after_id, |e| e.id),
            entries,
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

/// `at` as ms since the Unix epoch (0 if the clock is before it).
fn epoch_ms(at: Instant) -> i64 {
    let age = Instant::now().saturating_duration_since(at);
    SystemTime::now()
        .checked_sub(age)
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

// ── Formatting ───────────────────────────────────────────────────────

fn source_label(s: Source) -> String {
    match s {
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
        Source::Other(b) => format!("src=0x{b:02X}"),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn le_f32(p: &[u8]) -> Option<f32> {
    Some(f32::from_le_bytes(p.get(..4)?.try_into().ok()?))
}

fn le_u16(p: &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes(p.get(..2)?.try_into().ok()?))
}

fn volume(db: f32) -> String {
    if db <= -128.0 {
        "MUTE".into()
    } else {
        format!("{db:+7.2} dB")
    }
}

fn name_text(p: &[u8]) -> String {
    let end = p.iter().position(|&c| c == 0).unwrap_or(p.len());
    format!(
        "\"{}\"",
        String::from_utf8_lossy(p.get(..end).unwrap_or_default())
    )
}

/// One notification as a log line (without the timestamp).
pub(crate) fn describe(note: &Notification) -> String {
    let seq = format!("[{:3}]", note.seq);
    let mut line = match &note.event {
        Event::Idle => "Idle".into(),
        Event::MasterVolume(db) => format!("{:<14}  {}", "v1.MasterVolume", volume(*db)),
        Event::ParamChanged {
            offset,
            source,
            bytes,
        } => {
            let (name, value) = param(usize::from(*offset), bytes);
            format!("{seq} [{:<8}] {name:<32}  {value}", source_label(*source))
        }
        Event::BulkInvalidated { source } => format!(
            "{seq} v2.BulkInvalidated            source={}",
            source_label(*source)
        ),
        Event::PresetLoaded { slot } => format!("{seq} v2.PresetLoaded                slot={slot}"),
        Event::InputFormat { channels } => {
            format!("{seq} v2.InputFormat                 channels={channels}")
        }
        Event::SiggenState {
            state,
            reason,
            signal_type,
            channel,
        } => {
            let state = ["IDLE", "FADE_IN", "RUN", "GAP", "FADE_OUT"]
                .get(usize::from(*state))
                .map_or_else(|| format!("state={state}"), |s| s.to_string());
            let reason = ["-", "HOST", "COMPLETED", "PRESET", "RECONFIG"]
                .get(usize::from(*reason))
                .map_or_else(|| format!("reason={reason}"), |s| s.to_string());
            let ch = if *channel == 0xFF {
                "-".to_string()
            } else {
                channel.to_string()
            };
            format!(
                "{seq} v2.SiggenState                 {state} reason={reason} type={signal_type} ch={ch}"
            )
        }
        Event::AdatState {
            enabled,
            active,
            pin,
        } => format!(
            "{seq} v2.AdatState                   enabled={} active={} pin={pin}",
            u8::from(*enabled),
            u8::from(*active)
        ),
        Event::I2sSlaveState { state, rate_hz } => {
            let state = ["INACTIVE", "ACQUIRING", "RELOCKING", "LOCKED"]
                .get(usize::from(*state))
                .map_or_else(|| format!("state={state}"), |s| s.to_string());
            format!("{seq} v2.I2sSlaveState              {state} rate={rate_hz}")
        }
        Event::IrLearn {
            state,
            protocol,
            code,
        } => {
            let proto = ["NONE", "NEC", "RC5", "RC6", "HASH"]
                .get(usize::from(*protocol))
                .copied()
                .unwrap_or("?");
            if *state == 2 {
                format!("{seq} v2.IrLearn                     DONE {proto} code=0x{code:08X}")
            } else {
                format!("{seq} v2.IrLearn                     TIMEOUT")
            }
        }
        Event::AdatInputState {
            state,
            rate_hz,
            clock_mode,
        } => {
            let state = ["INACTIVE", "ACQUIRING", "SYNCING", "LOCKED", "RELOCKING"]
                .get(usize::from(*state))
                .map_or_else(|| format!("state={state}"), |s| s.to_string());
            let mode = if *clock_mode == 1 { "slave" } else { "master" };
            format!("{seq} v2.AdatInputState             {state} rate={rate_hz} mode={mode}")
        }
        Event::CsAux {
            slot,
            state,
            level_q8,
            source,
        } => {
            let pct = f32::from(*level_q8) / 256.0;
            format!(
                "{seq} v2.CsAux                      slot={slot} state={state} level={pct:.1}% src={}",
                source_label(*source)
            )
        }
        Event::Unknown { id, bytes } if *id == EVT_INPUT_SOURCE_NAME || *id == EVT_DEVICE_NAME => {
            let what = if *id == EVT_DEVICE_NAME {
                "v2.DeviceName                 ".to_string()
            } else {
                format!(
                    "v2.InputSourceName            source={}",
                    bytes.first().copied().unwrap_or(0)
                )
            };
            let src = source_label(Source::from_raw(bytes.get(1).copied().unwrap_or(0)));
            let name = name_text(bytes.get(4..).unwrap_or_default());
            format!("{seq} {what} {name} src={src}")
        }
        // The reader's marker for a packet it could not decode.
        Event::Unknown { id: 0xFF, bytes } if note.lost => {
            format!("(undecodable, {} bytes)  {}", bytes.len(), hex(bytes))
        }
        Event::Unknown { id, bytes } => format!("{seq} v2?evt=0x{id:02X}  {}", hex(bytes)),
    };
    if note.lost {
        line.push_str("  (packets lost before this one)");
    }
    line
}

fn f(p: &[u8], suffix: &str) -> String {
    match le_f32(p) {
        Some(v) => format!("{v:+.3}{suffix}"),
        None => "(short)".into(),
    }
}

fn flag(p: &[u8]) -> String {
    match p.first() {
        Some(0) => "0 (false)".into(),
        Some(_) => "1 (true)".into(),
        None => "(empty)".into(),
    }
}

fn byte(p: &[u8]) -> String {
    p.first()
        .map_or_else(|| "(empty)".into(), |v| v.to_string())
}

fn band(p: &[u8]) -> String {
    match (
        p.first(),
        p.get(1),
        le_f32(p.get(4..).unwrap_or_default()),
        le_f32(p.get(8..).unwrap_or_default()),
        le_f32(p.get(12..).unwrap_or_default()),
    ) {
        (Some(t), Some(b), Some(fr), Some(q), Some(gain)) => {
            format!("type={t}  byp={b}  f={fr:.1} Hz  Q={q:.2}  g={gain:+.2} dB")
        }
        _ => hex(p),
    }
}

/// `offset - base` when it lies inside `base..base + len`.
fn within(off: usize, base: usize, len: usize) -> Option<usize> {
    off.checked_sub(base).filter(|r| *r < len)
}

/// `(rel / size, rel % size)`, for the record index and the offset inside
/// it (`size` is a non-zero constant of the layout).
fn split(rel: usize, size: usize) -> (usize, usize) {
    (
        rel.checked_div(size).unwrap_or(0),
        rel.checked_rem(size).unwrap_or(0),
    )
}

/// A bulk-image offset's field name and its value, formatted by type.
fn param(off: usize, p: &[u8]) -> (String, String) {
    let n = |s: &str| s.to_string();
    let size = p.len();
    if off < g::OFF_GLOBAL {
        return (format!("header+0x{off:02X}"), hex(p));
    }
    if let Some(k) = within(off, g::OFF_GLOBAL, g::LEN_GLOBAL) {
        match k {
            0 => return (n("global.preamp_gain_db"), f(p, " dB")),
            4 => return (n("global.bypass"), flag(p)),
            5 => return (n("global.loudness_enabled"), flag(p)),
            6 => return (n("global.loudness_output_mask"), hex(p)),
            8 => return (n("global.loudness_ref_spl"), f(p, " dB SPL")),
            12 => return (n("global.loudness_intensity_pct"), f(p, "%")),
            _ => {}
        }
    }
    if let Some(k) = within(off, g::OFF_CROSSFEED, g::LEN_CROSSFEED) {
        match k {
            0 => return (n("crossfeed.enabled"), flag(p)),
            1 => return (n("crossfeed.preset"), byte(p)),
            2 => return (n("crossfeed.itd_enabled"), flag(p)),
            4 => return (n("crossfeed.custom_fc"), f(p, " Hz")),
            8 => return (n("crossfeed.custom_feed_db"), f(p, " dB")),
            _ => {}
        }
    }
    if let Some(k) = within(off, g::OFF_LEGACY, 12) {
        let (i, sub) = split(k, 4);
        if sub == 0 && size == 4 {
            return (format!("legacy.gain_db[{i}]"), f(p, " dB"));
        }
    }
    if let Some(k) = within(off, g::OFF_LEGACY + 12, 3)
        && size == 1
    {
        return (format!("legacy.mute[{k}]"), flag(p));
    }
    if let Some(k) = within(off, g::OFF_DELAYS, g::LEN_DELAYS) {
        let (i, sub) = split(k, 4);
        if sub == 0 && size == 4 {
            return (format!("delays.delay_ms[{i}]"), f(p, " ms"));
        }
    }
    if let Some(rel) = within(off, g::OFF_CROSSPOINTS, g::LEN_CROSSPOINTS) {
        let (cell, sub) = split(rel, 8);
        let (input, output) = split(cell, g::wire::WIRE_MAX_OUTPUT_CHANNELS as usize);
        if sub == 0
            && let (Some(en), Some(inv), Some(gain)) =
                (p.first(), p.get(1), le_f32(p.get(4..).unwrap_or_default()))
        {
            return (
                format!("crosspoints[{input}][{output}]"),
                format!(
                    "en={} inv={} {gain:+6.2} dB",
                    u8::from(*en != 0),
                    u8::from(*inv != 0)
                ),
            );
        }
        return (format!("crosspoints[{input}][{output}]+0x{sub:X}"), hex(p));
    }
    if let Some(rel) = within(off, g::OFF_OUTPUTS, g::LEN_OUTPUTS) {
        let (i, sub) = split(rel, 12);
        return match sub {
            0 => (format!("outputs[{i}].enabled"), flag(p)),
            1 => (format!("outputs[{i}].mute"), flag(p)),
            4 => (format!("outputs[{i}].gain_db"), f(p, " dB")),
            8 => (format!("outputs[{i}].delay_ms"), f(p, " ms")),
            _ => (format!("outputs[{i}]+0x{sub:X}"), hex(p)),
        };
    }
    if off == g::OFF_PINS && size == 1 {
        return (n("pins.num_pin_outputs"), byte(p));
    }
    if let Some(k) = within(off, g::OFF_PINS + 1, 5)
        && size == 1
    {
        return (format!("pins.pins[{k}]"), byte(p));
    }
    if let Some(rel) = within(off, g::OFF_EQ, g::LEN_EQ) {
        let (slot, sub) = split(rel, 16);
        let (ch, b) = split(slot, g::wire::WIRE_MAX_BANDS as usize);
        if sub == 0 && size == 16 {
            return (format!("eq[{ch}][{b}]"), band(p));
        }
        return (format!("eq[{ch}][{b}]+0x{sub:X}"), hex(p));
    }
    if let Some(rel) = within(off, g::OFF_CHANNEL_NAMES, g::LEN_CHANNEL_NAMES) {
        let (i, sub) = split(rel, 32);
        if sub == 0 {
            return (format!("channel_names[{i}]"), name_text(p));
        }
    }
    if let Some(k) = within(off, g::OFF_I2S_CONFIG, g::LEN_I2S_CONFIG) {
        let v = p.first().copied().unwrap_or(0);
        match k {
            0..=3 if size == 1 => {
                let kind = if v == 1 { "I2S" } else { "SPDIF" };
                return (
                    format!("i2s_config.output_types[{k}]"),
                    format!("{v} ({kind})"),
                );
            }
            4 => return (n("i2s_config.bck_pin"), byte(p)),
            5 => return (n("i2s_config.mck_pin"), byte(p)),
            6 => return (n("i2s_config.mck_enabled"), flag(p)),
            7 => {
                let mult = if v == 1 { "256x" } else { "128x" };
                return (n("i2s_config.mck_multiplier"), format!("{v} ({mult})"));
            }
            _ => {}
        }
    }
    if let Some(k) = within(off, g::OFF_LEVELLER, g::LEN_LEVELLER) {
        match k {
            0 => return (n("leveller.enabled"), flag(p)),
            1 => return (n("leveller.speed"), byte(p)),
            2 => return (n("leveller.lookahead"), flag(p)),
            4 => return (n("leveller.amount"), f(p, "%")),
            8 => return (n("leveller.max_gain_db"), f(p, " dB")),
            12 => return (n("leveller.gate_threshold_db"), f(p, " dB")),
            16 => return (n("leveller.detector_mask"), hex(p)),
            17 => return (n("leveller.apply_mask"), hex(p)),
            _ => {}
        }
    }
    if let Some(k) = within(off, g::OFF_PREAMP, g::LEN_PREAMP) {
        let (i, sub) = split(k, 4);
        if sub == 0 && size == 4 {
            return (format!("preamp.preamp_db[{i}]"), f(p, " dB"));
        }
    }
    if off == g::OFF_MASTER_VOLUME && size == 4 {
        return (
            n("master_volume.master_volume_db"),
            le_f32(p).map_or_else(|| hex(p), volume),
        );
    }
    if let Some(k) = within(off, g::OFF_INPUT_CONFIG, g::LEN_INPUT_CONFIG)
        && size == 1
    {
        let v = p.first().copied().unwrap_or(0);
        let name = match k {
            0 => {
                let src = ["USB", "SPDIF", "I2S", "ADAT", "SPDIF2", "SPDIF3", "SPDIF4"]
                    .get(usize::from(v))
                    .copied()
                    .unwrap_or("?");
                return (n("input_config.input_source"), format!("{v} ({src})"));
            }
            1 => n("input_config.spdif_rx_pin"),
            2 => n("input_config.i2s_rx_pin"),
            3 => {
                let rate = ["44100", "48000", "96000"]
                    .get(usize::from(v))
                    .copied()
                    .unwrap_or("?");
                return (n("input_config.i2s_input_rate"), format!("{v} ({rate} Hz)"));
            }
            4 => n("input_config.i2s_input_channels"),
            5..=7 => format!("input_config.i2s_rx_pin_ext[{}]", k.saturating_sub(5)),
            8..=10 => format!("input_config.spdif_rx_pin_ext[{}]", k.saturating_sub(8)),
            11 => n("input_config.spdif_ext_enable_p1"),
            12 => n("input_config.i2s_clock_mode"),
            13 => {
                let pin = if v == 0 {
                    "unset".into()
                } else {
                    format!("GPIO {v}")
                };
                return (n("input_config.adat_input_pin"), pin);
            }
            14 => {
                let what = match v {
                    0 => "absent",
                    2 => "enabled",
                    _ => "disabled",
                };
                return (
                    n("input_config.adat_input_enabled_p1"),
                    format!("{v} ({what})"),
                );
            }
            _ => {
                let what = match v {
                    0 => "absent",
                    2 => "slave",
                    _ => "master",
                };
                return (
                    n("input_config.adat_clock_mode_p1"),
                    format!("{v} ({what})"),
                );
            }
        };
        return (name, v.to_string());
    }
    if let Some(k) = within(off, g::OFF_LG_SOUND_SYNC, 4)
        && size == 1
    {
        return match k {
            0 => (n("lg_sound_sync.enabled"), flag(p)),
            1 => (n("lg_sound_sync.present"), flag(p)),
            2 => {
                let v = p.first().copied().unwrap_or(0xFF);
                let text = if v == 0xFF {
                    "-".into()
                } else {
                    format!("{v} / 100")
                };
                (n("lg_sound_sync.volume"), text)
            }
            _ => (n("lg_sound_sync.muted"), flag(p)),
        };
    }
    if off == g::OFF_USER_VOLUME && size == 4 {
        return (
            n("user_volume.user_volume_db"),
            le_f32(p).map_or_else(|| hex(p), volume),
        );
    }
    if off == g::OFF_USER_VOLUME + 4 && size == 1 {
        return (n("user_volume.user_mute"), flag(p));
    }
    if let Some(k) = within(off, g::OFF_DAC_HW_MUTE, g::LEN_DAC_HW_MUTE) {
        let ms = || le_u16(p).map_or_else(|| hex(p), |v| format!("{v} ms"));
        match k {
            0 => return (n("dac_mute.enabled"), flag(p)),
            1 => return (n("dac_mute.active_low"), flag(p)),
            2 => return (n("dac_mute.pin"), byte(p)),
            4 => return (n("dac_mute.hold_ms"), ms()),
            6 => return (n("dac_mute.release_ms"), ms()),
            _ => {}
        }
    }
    if let Some(rel) = within(off, g::OFF_CROSSOVERS, g::LEN_CROSSOVERS) {
        let (slot, sub) = split(rel, 16);
        let (ch, b) = split(slot, g::wire::WIRE_MAX_XOVER_BANDS as usize);
        if sub == 0 && size == 16 {
            return (format!("xover[{ch}][{b}]"), band(p));
        }
        return (format!("xover[{ch}][{b}]+0x{sub:X}"), hex(p));
    }
    if off == g::OFF_ADAT_CONFIG && size == 1 {
        return (n("adat_config.enabled"), flag(p));
    }
    if off == g::OFF_ADAT_CONFIG + 1 && size == 1 {
        let v = p.first().copied().unwrap_or(0);
        let pin = if v == 0 {
            "default".into()
        } else {
            format!("GPIO {v}")
        };
        return (n("adat_config.pin"), pin);
    }
    if let Some(k) = within(off, g::OFF_PSYBASS, g::LEN_PSYBASS) {
        match k {
            0 => return (n("psybass.enabled"), flag(p)),
            2 => return (n("psybass.output_mask"), hex(p)),
            4 => return (n("psybass.cutoff_hz"), f(p, " Hz")),
            8 => return (n("psybass.harmonics_db"), f(p, " dB")),
            12 => return (n("psybass.drive_db"), f(p, " dB")),
            16 => return (n("psybass.character_pct"), f(p, "%")),
            20 => return (n("psybass.original_db"), f(p, " dB")),
            _ => {}
        }
    }
    if let Some(k) = within(off, g::OFF_UPMIX, g::LEN_UPMIX) {
        return match k {
            0 => (n("upmix.enabled"), flag(p)),
            1 => (n("upmix.engine"), byte(p)),
            2 => (n("upmix.mode"), byte(p)),
            3 => (
                n("upmix.center_trim_halfdb"),
                p.first()
                    .map(|&v| format!("{:+.1} dB", f32::from(v.cast_signed()) * 0.5))
                    .unwrap_or_default(),
            ),
            _ => {
                let (i, sub) = split(k.saturating_sub(4), 4);
                if sub == 0 {
                    (format!("upmix.param[{}]", i.saturating_add(3)), f(p, ""))
                } else {
                    (format!("upmix+0x{k:X}"), hex(p))
                }
            }
        };
    }
    if let Some(k) = within(off, g::OFF_SUBHARM, g::LEN_SUBHARM) {
        match k {
            0 => return (n("subharm.enabled"), flag(p)),
            2 => return (n("subharm.output_mask"), hex(p)),
            4 => return (n("subharm.low_db"), f(p, " dB")),
            8 => return (n("subharm.high_db"), f(p, " dB")),
            12 => return (n("subharm.boost_db"), f(p, " dB")),
            16 => return (n("subharm.top_db"), f(p, " dB")),
            20 => return (n("subharm.select_depth"), f(p, "%")),
            24 => return (n("subharm.select_hold_ms"), f(p, " ms")),
            28 => return (n("subharm.ceiling_db"), f(p, " dBFS")),
            32 => return (n("subharm.select_mode"), hex(p)),
            33 => return (n("subharm.link_pairs"), flag(p)),
            _ => {}
        }
    }
    if let Some(k) = within(off, g::OFF_TUBE, g::LEN_TUBE) {
        match k {
            0 => return (n("tube.enabled"), flag(p)),
            1 => return (n("tube.tube_type"), byte(p)),
            2 => return (n("tube.rectifier"), byte(p)),
            3 => return (n("tube.xfmr_enabled"), flag(p)),
            4 => return (n("tube.output_mask"), hex(p)),
            8 => return (n("tube.drive_db"), f(p, " dB")),
            12 => return (n("tube.bias_pct"), f(p, "%")),
            16 => return (n("tube.asym_db"), f(p, " dB")),
            20 => return (n("tube.hardness_pct"), f(p, "%")),
            24 => return (n("tube.sag_pct"), f(p, "%")),
            28 => return (n("tube.xfmr_damping"), f(p, "")),
            32 => return (n("tube.xfmr_res_hz"), f(p, " Hz")),
            36 => return (n("tube.mix_pct"), f(p, "%")),
            40 => return (n("tube.trim_db"), f(p, " dB")),
            _ => {}
        }
    }
    if let Some(rel) = within(off, g::OFF_LIMITER, g::LEN_LIMITER) {
        let (o, sub) = split(rel, 12);
        return match sub {
            0 => (format!("limiter[{o}].enabled"), flag(p)),
            1 => (
                format!("limiter[{o}].link_group"),
                p.first()
                    .map(|&v| {
                        if v == 0 {
                            "off".into()
                        } else {
                            format!("group {v}")
                        }
                    })
                    .unwrap_or_default(),
            ),
            4 => (format!("limiter[{o}].threshold_db"), f(p, " dBFS")),
            8 => (format!("limiter[{o}].release_ms"), f(p, " ms")),
            s => (format!("limiter[{o}]+0x{s:X}"), hex(p)),
        };
    }
    (format!("offset=0x{off:04X} size={size}"), hex(p))
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_c7 {
    use super::*;

    fn note(event: Event) -> Notification {
        Notification {
            seq: 7,
            event,
            lost: false,
            origin: None,
            received: Instant::now(),
        }
    }

    fn changed(off: usize, src: u8, bytes: &[u8]) -> String {
        describe(&note(Event::ParamChanged {
            offset: off as u16,
            source: Source::from_raw(src),
            bytes: bytes.to_vec(),
        }))
    }

    #[test]
    fn wp_c7_param_changes_are_named() {
        let line = changed(g::OFF_MASTER_VOLUME, 7, &(-12.5f32).to_le_bytes());
        assert!(
            line.starts_with("[  7] [UAC1    ] master_volume.master_volume_db"),
            "{line}"
        );
        assert!(line.ends_with(" -12.50 dB"), "{line}");
        let gain = changed(g::OFF_OUTPUTS + 12 * 2 + 4, 1, &3.0f32.to_le_bytes());
        assert!(
            gain.contains("outputs[2].gain_db") && gain.ends_with("+3.000 dB"),
            "{gain}"
        );
        let mut band = [0u8; 16];
        band[0] = 1;
        band[4..8].copy_from_slice(&1000.0f32.to_le_bytes());
        band[8..12].copy_from_slice(&0.7f32.to_le_bytes());
        band[12..16].copy_from_slice(&(-3.0f32).to_le_bytes());
        let eq = changed(g::OFF_EQ + (12 * 3 + 2) * 16, 5, &band);
        assert!(
            eq.contains("eq[3][2]") && eq.contains("f=1000.0 Hz") && eq.contains("g=-3.00 dB"),
            "{eq}"
        );
        let midi = changed(g::OFF_GLOBAL + 4, 10, &[1]);
        assert!(
            midi.contains("[MIDI    ] global.bypass") && midi.ends_with("1 (true)"),
            "{midi}"
        );
        let x = changed(
            g::OFF_CROSSPOINTS + (9 + 2) * 8,
            1,
            &[1, 0, 0, 0, 0, 0, 0, 0],
        );
        assert!(x.contains("crosspoints[1][2]"), "{x}");
        let lim = changed(g::OFF_LIMITER + 12 * 8 + 4, 1, &(-1.0f32).to_le_bytes());
        assert!(lim.contains("limiter[8].threshold_db"), "{lim}");
    }

    #[test]
    fn wp_c7_events_and_fallbacks() {
        let s = describe(&note(Event::SiggenState {
            state: 2,
            reason: 0,
            signal_type: 4,
            channel: 0xFF,
        }));
        assert!(
            s.contains("v2.SiggenState                 RUN reason=- type=4 ch=-"),
            "{s}"
        );
        assert!(describe(&note(Event::PresetLoaded { slot: 3 })).ends_with("slot=3"));
        let u = describe(&note(Event::Unknown {
            id: 0x0E,
            bytes: vec![0xAB],
        }));
        assert!(u.contains("v2?evt=0x0E  AB"), "{u}");
        let v1 = describe(&note(Event::MasterVolume(-10.0)));
        assert!(
            v1.starts_with("v1.MasterVolume") && v1.ends_with("-10.00 dB"),
            "{v1}"
        );
        let mut lost = note(Event::Unknown {
            id: 0xFF,
            bytes: vec![3, 2],
        });
        lost.lost = true;
        let l = describe(&lost);
        assert!(
            l.starts_with("(undecodable, 2 bytes)  03 02") && l.ends_with("lost before this one)"),
            "{l}"
        );
        let mut name = vec![2, 5, 0, 0];
        name.extend(b"TV\0\0");
        let n = describe(&note(Event::Unknown {
            id: EVT_INPUT_SOURCE_NAME,
            bytes: name,
        }));
        assert!(
            n.contains("InputSourceName") && n.contains("source=2") && n.contains("\"TV\""),
            "{n}"
        );
    }

    #[test]
    fn wp_c7_no_offset_or_payload_panics() {
        for off in (0..g::BULK_SIZE + 64).step_by(1) {
            for len in [0usize, 1, 2, 3, 4, 8, 16, 52] {
                let _ = param(off, &vec![0xA5; len]);
            }
        }
    }

    #[test]
    fn wp_c7_the_log_is_bounded_and_ids_survive_a_clear() {
        let mut log = MonitorLog::default();
        for i in 0..MonitorLog::LIMIT + 5 {
            log.push(i as i64, format!("{i}"));
        }
        let all = log.read_after(0);
        assert_eq!(all.entries.len(), MonitorLog::LIMIT);
        assert_eq!(all.entries[0].id, 6);
        assert_eq!(all.last_id, (MonitorLog::LIMIT + 5) as u64);
        let none = log.read_after(all.last_id);
        assert!(none.entries.is_empty());
        assert_eq!(none.last_id, all.last_id);
        log.clear();
        assert!(log.read_after(0).entries.is_empty());
        log.push(0, "x".into());
        let next = log.read_after(all.last_id);
        assert_eq!(next.entries.len(), 1);
        assert_eq!(next.entries[0].id, all.last_id + 1);
    }

    #[test]
    fn wp_c7_entries_carry_the_receive_time() {
        let mut log = MonitorLog::default();
        log.record(&note(Event::Idle));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let e = &log.read_after(0).entries[0];
        assert!((now - e.time_ms).abs() < 5_000, "{} vs {now}", e.time_ms);
        assert_eq!(e.text, "Idle");
    }
}
