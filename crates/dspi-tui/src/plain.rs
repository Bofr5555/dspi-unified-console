//! The plain line mode's words (WP UB11, UXP B11, owner D16): the overview
//! as sentences that a screen reader or a log reads in order, with no box,
//! braille or meter glyph. `uconsole --plain` (apps/terminal/src/plain.rs)
//! prints them; the interface's overview grid says the same in cells.

use dspi_proto::{FilterType, xover};
use dspi_session::DeviceState;

use crate::screens::{channel_hidden, channel_label, delivered_delay_ms};

/// A level in dB with one decimal and a sign only when it is below zero:
/// "gain 0.0 dB" reads better aloud than "+0.0 dB". Never "-0.0".
fn level(v: f64) -> String {
    format!("{:.1} dB", v + 0.0)
}

fn hz_words(v: f64) -> String {
    if v >= 1000.0 {
        let k = format!("{:.2}", v / 1000.0);
        let k = k.trim_end_matches('0').trim_end_matches('.');
        format!("{k} kHz")
    } else {
        format!("{} Hz", v.round() as i64)
    }
}

fn filters_word(n: usize) -> String {
    match n {
        0 => "no filters".into(),
        1 => "1 filter".into(),
        n => format!("{n} filters"),
    }
}

/// An active crossover band in words: "high-pass 80 Hz LR4".
fn crossover_words(p: &dspi_proto::value::EqParamPacket) -> String {
    let f = hz_words(f64::from(p.freq));
    match xover::meta(p.filter_type.to_raw()) {
        Some(m) => format!(
            "{} {f} {}{}",
            if m.high_pass { "high-pass" } else { "low-pass" },
            m.family.short().to_uppercase(),
            m.order
        ),
        None => match p.filter_type {
            FilterType::LowPass | FilterType::LowPass1 => format!("low-pass {f}"),
            FilterType::HighPass | FilterType::HighPass1 => format!("high-pass {f}"),
            other => format!("{} {f}", other.label()),
        },
    }
}

/// One channel as a sentence: "Out 1 · L: 2 filters (high-pass 80 Hz LR4),
/// gain 0.0 dB, not muted"; an input: "USB L: 1 filter, trim 0.0 dB"; an
/// output that is switched off: "Out 2 · R: off".
pub fn channel_sentence(state: &DeviceState, channel: usize) -> String {
    let ni = usize::from(state.caps.num_inputs);
    let active = |p: &&dspi_proto::value::EqParamPacket| p.filter_type != FilterType::Flat;
    let peq = state.bands(channel as u8).iter().filter(active).count();
    let label = channel_label(state, channel);
    let mut parts: Vec<String> = Vec::new();
    match channel.checked_sub(ni) {
        None => {
            parts.push(filters_word(peq));
            parts.push(format!(
                "trim {}",
                level(f64::from(state.preamp_db(channel)))
            ));
            let delay = state.delay_ms(channel);
            if delay != 0.0 {
                parts.push(format!("delay {:.1} ms", delivered_delay_ms(state, delay)));
            }
        }
        Some(o) => {
            let out = state.output(o);
            if !out.enabled {
                return format!("{label}: off");
            }
            let xo: Vec<String> = state
                .xover_bands(channel as u8)
                .iter()
                .filter(active)
                .map(crossover_words)
                .collect();
            let filters = filters_word(peq + xo.len());
            parts.push(if xo.is_empty() {
                filters
            } else {
                format!("{filters} ({})", xo.join(", "))
            });
            parts.push(format!("gain {}", level(f64::from(out.gain_db))));
            parts.push(if out.mute { "muted" } else { "not muted" }.into());
            if out.delay_ms != 0.0 {
                parts.push(format!(
                    "delay {:.1} ms",
                    delivered_delay_ms(state, out.delay_ms)
                ));
            }
        }
    }
    format!("{label}: {}", parts.join(", "))
}

/// The overview: the listening volume, what is unsaved, then a sentence per
/// input and per output. An unused output (nothing routed to it, no test
/// signal on it) is left out as the interface's overview leaves it out,
/// and one line says how many were; `all` lists them too.
pub fn overview(state: &DeviceState, all: bool) -> Vec<String> {
    let (user, muted) = state.user_volume();
    let mut out = vec![format!(
        "Volume {}, {}.",
        level(f64::from(user)),
        if muted { "muted" } else { "not muted" }
    )];
    out.push(unsaved_sentence(state));
    let ni = usize::from(state.caps.num_inputs);
    let no = usize::from(state.caps.num_outputs);
    let mut hidden = 0;
    for ch in 0..ni + no {
        if !all && channel_hidden(state, ch) {
            hidden += 1;
            continue;
        }
        if ch >= ni && channel_hidden(state, ch) {
            out.push(format!("{}: unused", channel_label(state, ch)));
            continue;
        }
        out.push(channel_sentence(state, ch));
    }
    match hidden {
        0 => {}
        1 => out.push("1 unused output is not listed (overview all lists it).".into()),
        n => out.push(format!(
            "{n} unused outputs are not listed (overview all lists them)."
        )),
    }
    out
}

/// "No unsaved changes." or "Unsaved changes: Out 1 · L gain, Volume." in
/// the unsaved-changes list's own words.
pub fn unsaved_sentence(state: &DeviceState) -> String {
    let diff = state.unsaved_diff();
    if diff.is_empty() {
        return "No unsaved changes.".into();
    }
    let words: Vec<String> = diff.iter().map(|d| d.text.clone()).collect();
    format!("Unsaved changes: {}.", words.join("; "))
}

/// The plain mode's own words, for `help`, after the shared grammar's.
pub const HELP: &[(&str, &str)] = &[
    (
        "overview",
        "Every channel as a sentence (overview all: the unused outputs too)",
    ),
    (
        "show N",
        "One channel: in 1, out 2, or its number in the list",
    ),
    (
        "save",
        "Save to Device: the parameters into the active preset",
    ),
    (
        "snapshot",
        "Snapshots: save NAME, list, recall ID, diff ID, delete ID",
    ),
    (
        "ab",
        "A/B: ab ID compares the device now (A) with a snapshot (B)",
    ),
    ("diff", "What differs from the saved preset or a snapshot"),
    (
        "lock",
        "The installer lock: status, unlock, set-pin, clear-pin",
    ),
    ("limits", "The installer's limits: show, set FILE, clear"),
    ("status", "The device, its link and its lock"),
    ("params", "Every parameter, its range and what it does"),
    ("help", "This list"),
    ("quit", "Leave (asks once when something is unsaved)"),
];

/// `show` and its words: the channel `word` names, unified (inputs first).
/// "in 1", "out 2" (counted from 1, as everywhere a person reads them), or
/// a bare number counting every channel from 1.
pub fn channel_named(state: &DeviceState, words: &[&str]) -> Option<usize> {
    let ni = usize::from(state.caps.num_inputs);
    let no = usize::from(state.caps.num_outputs);
    let n = |w: &str| w.parse::<usize>().ok().filter(|n| *n >= 1);
    match words {
        [w] => {
            let lower = w.to_ascii_lowercase();
            if let Some(rest) = lower.strip_prefix("out") {
                n(rest).filter(|o| *o <= no).map(|o| ni + o - 1)
            } else if let Some(rest) = lower.strip_prefix("in") {
                n(rest).filter(|i| *i <= ni).map(|i| i - 1)
            } else {
                n(w).filter(|c| *c <= ni + no).map(|c| c - 1)
            }
        }
        [kind, w] => {
            let k = kind.to_ascii_lowercase();
            match k.as_str() {
                "out" | "output" => n(w).filter(|o| *o <= no).map(|o| ni + o - 1),
                "in" | "input" => n(w).filter(|i| *i <= ni).map(|i| i - 1),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::fixture;

    fn glyph_free(line: &str) -> bool {
        // Box drawing, block elements and braille: what a screen reader
        // spells out cell by cell.
        !line.chars().any(|c| {
            ('\u{2500}'..='\u{259F}').contains(&c) || ('\u{2800}'..='\u{28FF}').contains(&c)
        })
    }

    #[test]
    fn ub11_the_overview_is_sentences_without_glyphs() {
        let state = fixture::state();
        let lines = overview(&state, false);
        assert!(lines[0].starts_with("Volume "), "{lines:#?}");
        assert_eq!(lines[1], "No unsaved changes.");
        assert!(lines.iter().all(|l| glyph_free(l)), "{lines:#?}");
        let ni = usize::from(state.caps.num_inputs);
        // every input, then the outputs the interface's overview shows
        for ch in 0..ni {
            assert!(
                lines.contains(&channel_sentence(&state, ch)),
                "input {ch}: {lines:#?}"
            );
        }
        let out1 = channel_sentence(&state, ni);
        assert!(
            out1.starts_with(&format!("{}: ", channel_label(&state, ni))),
            "{out1}"
        );
        assert!(out1.contains("gain ") && out1.contains("muted"), "{out1}");
    }

    #[test]
    fn ub11_an_unused_output_is_counted_not_listed_and_all_lists_it() {
        let state = fixture::state();
        let ni = usize::from(state.caps.num_inputs);
        let no = usize::from(state.caps.num_outputs);
        let hidden: Vec<usize> = (ni..ni + no)
            .filter(|c| channel_hidden(&state, *c))
            .collect();
        assert!(!hidden.is_empty(), "the fixture has unused outputs");
        let lines = overview(&state, false);
        for ch in &hidden {
            let label = channel_label(&state, *ch);
            assert!(
                !lines.iter().any(|l| l.starts_with(&format!("{label}: "))),
                "{label} listed: {lines:#?}"
            );
        }
        let count = lines.last().expect("a last line");
        assert!(count.contains("unused output"), "{count}");
        assert!(count.starts_with(&hidden.len().to_string()), "{count}");
        let all = overview(&state, true);
        for ch in &hidden {
            let label = channel_label(&state, *ch);
            assert!(all.contains(&format!("{label}: unused")), "{all:#?}");
        }
        assert!(!all.iter().any(|l| l.contains("not listed")), "{all:#?}");
    }

    #[test]
    fn ub11_a_sentence_says_gain_mute_crossover_and_off_in_words() {
        let mut state = fixture::state();
        let ni = usize::from(state.caps.num_inputs);
        // Out 1 at -3 dB, muted, a high-pass LR4 at 2.5 kHz in its first
        // crossover slot and one PEQ band
        let mut b = state.bulk.as_bytes().to_vec();
        let at = dspi_proto::generated::SECTIONS
            .iter()
            .find(|(n, _, _)| *n == "outputs")
            .map(|(_, o, _)| *o)
            .expect("the outputs section");
        b[at + 1] = 1; // mute
        b[at + 4..at + 8].copy_from_slice(&(-3.0f32).to_le_bytes());
        state.replace_bulk(dspi_proto::wire::BulkPacket::decode(b).expect("packet"));
        let s = channel_sentence(&state, ni);
        assert!(s.contains("gain -3.0 dB, muted"), "{s}");
        assert!(!s.contains("-0.0"), "{s}");
        // switched off
        let mut b = state.bulk.as_bytes().to_vec();
        b[at] = 0;
        state.replace_bulk(dspi_proto::wire::BulkPacket::decode(b).expect("packet"));
        assert_eq!(
            channel_sentence(&state, ni),
            format!("{}: off", channel_label(&state, ni))
        );
        assert_eq!(level(-0.0), "0.0 dB");
        assert_eq!(hz_words(2500.0), "2.5 kHz");
        assert_eq!(hz_words(80.0), "80 Hz");
    }

    #[test]
    fn ub11_show_names_a_channel_counted_from_one() {
        let state = fixture::state();
        let ni = usize::from(state.caps.num_inputs);
        assert_eq!(channel_named(&state, &["out", "1"]), Some(ni));
        assert_eq!(channel_named(&state, &["out2"]), Some(ni + 1));
        assert_eq!(channel_named(&state, &["in", "1"]), Some(0));
        assert_eq!(channel_named(&state, &["1"]), Some(0));
        assert_eq!(channel_named(&state, &["out", "0"]), None);
        assert_eq!(channel_named(&state, &["out", "99"]), None);
        assert_eq!(channel_named(&state, &["sideways"]), None);
    }
}
