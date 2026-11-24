//! -- WP SHOWFIX -- (finding 2) A device notification in plain words, for the
//! event log every client shows ("Volume set to 0.0 dB", "Signal generator
//! started on Out 4"). The Interrupt Monitor keeps the raw line
//! (`crate::monitor::describe`); the event record carries that line too, as
//! its `raw`, for a details row.
//!
//! Outputs and inputs are named as the screens number them ("Out 1", "In 3").
//! A record is written once and kept, so it names no channel by a name the
//! person may change later.
// Indices from the wire are bytes and small: "+ 1" to number them as the
// screens do cannot overflow.
#![allow(clippy::arithmetic_side_effects)]

use dspi_proto::generated::notify as nfy;
use dspi_proto::packets::SIGGEN_TYPES;
use dspi_session::notify::{Event, Notification, Source};

use crate::monitor::{ParamAt, param_at};

/// `note` in plain words, or `None` where there are none better than the raw
/// line (an event this build does not know).
pub(crate) fn plain(note: &Notification, inputs: u8) -> Option<String> {
    let line = match &note.event {
        Event::Idle => return None,
        Event::MasterVolume(db) => format!("Master volume set to {}", volume(f64::from(*db))),
        Event::ParamChanged { offset, bytes, .. } => {
            param(&param_at(usize::from(*offset), bytes)?, inputs)?
        }
        Event::BulkInvalidated { source } => match source {
            Source::Preset => "All settings replaced by a preset load".to_owned(),
            Source::Factory => "All settings reset to the factory's".to_owned(),
            _ => "All settings replaced at once".to_owned(),
        },
        Event::PresetLoaded { slot } => format!("Preset {} loaded", u16::from(*slot) + 1),
        Event::InputFormat { channels } => format!("The USB input now has {channels} channels"),
        Event::SiggenState {
            state,
            reason,
            signal_type,
            channel,
        } => siggen(*state, *reason, *signal_type, *channel),
        Event::AdatState { active, .. } => {
            if *active {
                "ADAT output started".to_owned()
            } else {
                "ADAT output stopped".to_owned()
            }
        }
        Event::I2sSlaveState { state, rate_hz } => match state {
            3 => format!("I\u{b2}S input clock locked at {}", hz(f64::from(*rate_hz))),
            1 => "I\u{b2}S input clock: finding the clock".to_owned(),
            2 => "I\u{b2}S input clock: lost, finding it again".to_owned(),
            _ => "I\u{b2}S input clock inactive".to_owned(),
        },
        Event::IrLearn { state, .. } => match state {
            1 => "IR learning: waiting for a remote's button".to_owned(),
            2 => "IR learning: a button was learned".to_owned(),
            3 => "IR learning timed out".to_owned(),
            _ => "IR learning ended".to_owned(),
        },
        Event::AdatInputState { state, rate_hz, .. } => match state {
            3 => format!("ADAT input locked at {}", hz(f64::from(*rate_hz))),
            1 | 2 => "ADAT input: finding the clock".to_owned(),
            4 => "ADAT input: lost, finding it again".to_owned(),
            _ => "ADAT input inactive".to_owned(),
        },
        Event::CsAux { slot, level_q8, .. } => format!(
            "Control input {}: level {:.0}%",
            u16::from(*slot) + 1,
            f64::from(*level_q8) / 256.0
        ),
        Event::Unknown { id, bytes } => {
            let id = u16::from(*id);
            if id == nfy::NOTIFY_EVT_RTA_CONFIG {
                "Analyser settings changed".to_owned()
            } else if id == nfy::NOTIFY_EVT_DEVICE_NAME {
                format!(
                    "Device name set to \u{201c}{}\u{201d}",
                    name(bytes.get(4..)?)
                )
            } else if id == nfy::NOTIFY_EVT_INPUT_SOURCE_NAME {
                format!(
                    "An input source was named \u{201c}{}\u{201d}",
                    name(bytes.get(4..)?)
                )
            } else {
                return None;
            }
        }
    };
    Some(line)
}

/// The signal generator's state change.
fn siggen(state: u8, reason: u8, signal_type: u8, channel: u8) -> String {
    let signal = SIGGEN_TYPES
        .iter()
        .find(|(_, t)| *t == signal_type)
        .map(|(n, _)| signal_word(n));
    let on = if channel == 0xFF {
        String::new()
    } else {
        format!(" on Out {}", u16::from(channel) + 1)
    };
    let what = signal.map(|s| format!(" ({s})")).unwrap_or_default();
    match state {
        // FADE_IN, RUN
        1 | 2 => format!("Signal generator started{on}{what}"),
        // GAP
        3 => format!("Signal generator paused between repeats{on}"),
        // FADE_OUT
        4 => format!("Signal generator stopping{on}"),
        // IDLE, by reason
        _ => match reason {
            2 => "Signal generator finished".to_owned(),
            3 => "Signal generator stopped by a preset load".to_owned(),
            4 => "Signal generator stopped: its settings changed".to_owned(),
            _ => "Signal generator stopped".to_owned(),
        },
    }
}

fn signal_word(token: &str) -> String {
    match token {
        "white" => "white noise".into(),
        "pink" => "pink noise".into(),
        "sweep-log" => "sweep".into(),
        "sweep-lin" => "linear sweep".into(),
        "sweep-step" => "stepped sweep".into(),
        "polarity" => "polarity test".into(),
        other => other.replace('-', " "),
    }
}

/// A parameter write in words, by its section.
fn param(at: &ParamAt, inputs: u8) -> Option<String> {
    let first = at.index.first().copied();
    let second = at.index.get(1).copied();
    let on = |what: &str| {
        at.number
            .map(|n| format!("{what} {}", if n != 0.0 { "on" } else { "off" }))
    };
    let field = || words(at.field);
    Some(match at.section {
        "user_volume" => match at.field {
            "user_volume_db" => match at.number {
                Some(db) if db <= -128.0 => "Volume muted".to_owned(),
                Some(db) => format!("Volume set to {}", volume(db)),
                None => "Volume changed".to_owned(),
            },
            "user_mute" => match at.number {
                Some(n) if n != 0.0 => "Volume muted".to_owned(),
                _ => "Volume unmuted".to_owned(),
            },
            _ => "Volume changed".to_owned(),
        },
        "master_volume" => match at.number {
            Some(db) => format!("Master volume set to {}", volume(db)),
            None => "Master volume changed".to_owned(),
        },
        "preamp" => match (at.element, at.number) {
            (Some(i), Some(db)) => format!("Preamp of In {} set to {}", i + 1, db_text(db)),
            _ => "Preamp changed".to_owned(),
        },
        "delays" => match (at.element, at.number) {
            (Some(ch), Some(ms)) => format!("Delay of {} set to {}", wire(ch, inputs), ms_text(ms)),
            _ => "Delay changed".to_owned(),
        },
        "outputs" => {
            let out = format!("Out {}", first? + 1);
            match (at.field, at.number) {
                ("enabled", Some(n)) => {
                    format!("{out} switched {}", if n != 0.0 { "on" } else { "off" })
                }
                ("mute", Some(n)) => {
                    format!("{out} {}", if n != 0.0 { "muted" } else { "unmuted" })
                }
                ("gain_db", Some(db)) => format!("{out} gain set to {}", db_text(db)),
                ("delay_ms", Some(ms)) => format!("{out} delay set to {}", ms_text(ms)),
                _ => format!("{out} changed"),
            }
        }
        "crosspoints" => {
            let route = format!("Route In {} \u{2192} Out {}", first? + 1, second? + 1);
            match (at.field, at.number) {
                _ if at.whole => format!("{route}: {}", crosspoint(&at.shown)),
                ("enabled", Some(n)) => format!("{route} {}", if n != 0.0 { "on" } else { "off" }),
                ("phase_invert", Some(n)) => {
                    format!(
                        "{route} {}",
                        if n != 0.0 { "inverted" } else { "not inverted" }
                    )
                }
                ("gain_db", Some(db)) => format!("{route} gain set to {}", db_text(db)),
                _ => format!("{route} changed"),
            }
        }
        "eq" | "crossovers" => {
            let what = if at.section == "eq" {
                "EQ band"
            } else {
                "Crossover filter"
            };
            let head = format!("{what} {} on {}", second? + 1, wire(first?, inputs));
            match (at.field, at.number) {
                _ if at.whole => format!("{head}: {}", band(&at.shown)),
                ("bypass", Some(n)) => {
                    format!("{head} {}", if n != 0.0 { "bypassed" } else { "in use" })
                }
                ("type", Some(t)) => format!("{head}: {}", filter_name(t)),
                ("freq", Some(f)) => format!("{head}: frequency {}", hz(f)),
                ("gain_db", Some(db)) => format!("{head}: gain {}", db_text(db)),
                ("q", Some(q)) => format!("{head}: Q {}", trim(q, 2)),
                _ => format!("{head} changed"),
            }
        }
        "channel_names" => match at.element {
            Some(ch) => format!("{} renamed \u{201c}{}\u{201d}", wire(ch, inputs), at.shown),
            None => "A channel was renamed".to_owned(),
        },
        "global" => match at.field {
            "preamp_gain_db" => match at.number {
                Some(db) => format!("Preamp set to {}", db_text(db)),
                None => "Preamp changed".to_owned(),
            },
            "bypass" => on("EQ bypass")?,
            "loudness_enabled" => on("Loudness")?,
            _ => format!("Loudness: {} changed", field()),
        },
        "limiter" => {
            let out = first
                .map(|o| format!(" on Out {}", o + 1))
                .unwrap_or_default();
            match at.field {
                "enabled" => format!(
                    "Limiter{out} {}",
                    if at.number? != 0.0 { "on" } else { "off" }
                ),
                _ => format!("Limiter{out}: {} set to {}", field(), at.shown),
            }
        }
        "crossfeed" | "leveller" | "psybass" | "upmix" | "subharm" | "tube" => {
            let effect = match at.section {
                "crossfeed" => "Crossfeed",
                "leveller" => "Leveller",
                "psybass" => "Bass enhancer",
                "upmix" => "Upmixer",
                "subharm" => "Subharmonic synthesiser",
                _ => "Tube",
            };
            match at.field {
                "enabled" => on(effect)?,
                _ => format!("{effect}: {} set to {}", field(), at.shown),
            }
        }
        "input_config" => "Input settings changed".to_owned(),
        "i2s_config" => "I\u{b2}S output settings changed".to_owned(),
        "pins" => "Output pins changed".to_owned(),
        "adat_config" => "ADAT settings changed".to_owned(),
        "dac_hw_mute" => "DAC mute settings changed".to_owned(),
        "lg_sound_sync" => "LG Sound Sync settings changed".to_owned(),
        "legacy" => "Settings changed".to_owned(),
        _ => return None,
    })
}

/// A wire channel as the screens name it: inputs first, then outputs.
fn wire(ch: usize, inputs: u8) -> String {
    match ch.checked_sub(usize::from(inputs)) {
        Some(o) if inputs > 0 => format!("Out {}", o + 1),
        _ => format!("In {}", ch + 1),
    }
}

/// A field's name in words: "max_gain_db" is "max gain".
fn words(field: &str) -> String {
    let field = ["_db", "_pct", "_ms", "_hz", "_q1"]
        .iter()
        .find_map(|unit| field.strip_suffix(unit))
        .unwrap_or(field);
    field.replace('_', " ")
}

/// The monitor's whole band ("type=3 bypass=0 f=2000 Hz Q=1 gain=3 dB") in words.
fn band(shown: &str) -> String {
    let get = |key: &str| {
        shown
            .split_whitespace()
            .find_map(|w| w.strip_prefix(key))
            .and_then(|v| v.parse::<f64>().ok())
    };
    let (Some(t), Some(f)) = (get("type="), get("f=")) else {
        return "changed".to_owned();
    };
    if t == 0.0 {
        return "off".to_owned();
    }
    let mut line = format!("{} {}", filter_name(t), hz(f));
    if let Some(g) = get("gain=").filter(|g| *g != 0.0) {
        line.push(' ');
        line.push_str(&db_text(g));
    }
    if let Some(q) = get("Q=") {
        line.push_str(&format!(" Q {}", trim(q, 2)));
    }
    if get("bypass=").is_some_and(|b| b != 0.0) {
        line.push_str(", bypassed");
    }
    line
}

/// The monitor's whole crosspoint ("enabled=1 invert=0 gain=-6 dB") in words.
fn crosspoint(shown: &str) -> String {
    let get = |key: &str| {
        shown
            .split_whitespace()
            .find_map(|w| w.strip_prefix(key))
            .and_then(|v| v.parse::<f64>().ok())
    };
    if get("enabled=").is_none_or(|e| e == 0.0) {
        return "off".to_owned();
    }
    let mut line = format!("on at {}", db_text(get("gain=").unwrap_or(0.0)));
    if get("invert=").is_some_and(|i| i != 0.0) {
        line.push_str(", inverted");
    }
    line
}

pub(crate) fn filter_name(code: f64) -> String {
    if !(0.0..=255.0).contains(&code) {
        return "unknown filter".to_owned();
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    crate::names::filter_type_name(code as u8)
}

/// A device name's bytes, NUL-padded.
fn name(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(bytes.get(..end).unwrap_or_default())
        .trim()
        .to_owned()
}

/// A volume: one decimal, a typographic minus ("−20.0 dB").
pub(crate) fn volume(db: f64) -> String {
    if db <= -128.0 {
        return "mute".to_owned();
    }
    format!("{} dB", signed(db, 1, false))
}

/// A gain: "+3 dB", "−6 dB", "0 dB".
pub(crate) fn db_text(db: f64) -> String {
    format!("{} dB", signed(db, 1, true))
}

pub(crate) fn ms_text(ms: f64) -> String {
    format!("{} ms", trim(ms, 2))
}

pub(crate) fn hz(f: f64) -> String {
    if f >= 1000.0 {
        format!("{} kHz", trim(f / 1000.0, 2))
    } else {
        format!("{} Hz", trim(f, 0))
    }
}

/// `v` with up to `places` decimals, trailing zeros dropped.
pub(crate) fn trim(v: f64, places: usize) -> String {
    let t = format!("{v:.places$}");
    let t = if t.contains('.') {
        t.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        t
    };
    t.replace('-', "\u{2212}")
}

/// `v` with `places` decimals (trailing zeros dropped when `short`), a
/// typographic minus, and a plus when `short` and above 0.
fn signed(v: f64, places: usize, short: bool) -> String {
    let v = if v == 0.0 { 0.0 } else { v };
    let body = if short {
        trim(v.abs(), places)
    } else {
        format!("{:.places$}", v.abs())
    };
    if body.trim_start_matches(['0', '.']).is_empty() {
        return body;
    }
    if v < 0.0 {
        format!("\u{2212}{body}")
    } else if short {
        format!("+{body}")
    } else {
        body
    }
}

#[cfg(test)]
mod tests_wp_showfix {
    use super::*;
    use dspi_proto::generated::{SECTIONS, wire};
    use dspi_transport::clock::Instant;

    fn note(event: Event) -> Notification {
        Notification {
            seq: 44,
            event,
            lost: false,
            origin: None,
            received: Instant::now(),
        }
    }

    fn at(section: &str) -> usize {
        SECTIONS.iter().find(|(n, ..)| *n == section).unwrap().1
    }

    #[test]
    fn wp_showfix_events_read_as_plain_words() {
        let siggen = note(Event::SiggenState {
            state: 2,
            reason: 0,
            signal_type: 4,
            channel: 3,
        });
        assert_eq!(
            plain(&siggen, 8).unwrap(),
            "Signal generator started on Out 4 (sweep)"
        );
        let done = note(Event::SiggenState {
            state: 0,
            reason: 2,
            signal_type: 4,
            channel: 0xFF,
        });
        assert_eq!(plain(&done, 8).unwrap(), "Signal generator finished");
        let volume = note(Event::ParamChanged {
            offset: u16::try_from(at("user_volume")).unwrap(),
            source: Source::HostSet,
            bytes: 0f32.to_le_bytes().to_vec(),
        });
        assert_eq!(plain(&volume, 8).unwrap(), "Volume set to 0.0 dB");
        let down = note(Event::ParamChanged {
            offset: u16::try_from(at("user_volume")).unwrap(),
            source: Source::HostSet,
            bytes: (-20f32).to_le_bytes().to_vec(),
        });
        assert_eq!(plain(&down, 8).unwrap(), "Volume set to \u{2212}20.0 dB");
        assert_eq!(
            plain(&note(Event::PresetLoaded { slot: 0 }), 8).unwrap(),
            "Preset 1 loaded"
        );
        // A whole PEQ band on wire channel 2 (input 3 of 8), band 0.
        let mut band = vec![3u8, 0, 0, 0];
        for v in [2000f32, 1.0, 3.0] {
            band.extend_from_slice(&v.to_le_bytes());
        }
        let eq = note(Event::ParamChanged {
            offset: u16::try_from(at("eq") + 2 * usize::from(wire::WIRE_MAX_BANDS) * 16).unwrap(),
            source: Source::HostSet,
            bytes: band,
        });
        let words = plain(&eq, 8).unwrap();
        assert!(words.starts_with("EQ band 1 on In 3: "), "{words}");
        assert!(words.ends_with(" 2 kHz +3 dB Q 1"), "{words}");
        assert_eq!(plain(&note(Event::Idle), 8), None);
    }
}
