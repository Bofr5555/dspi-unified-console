//! -- WP SHOWFIX -- (finding 4) One change of the history in words, naming
//! the channels as the person knows them ([`crate::channel_words`]): what an
//! AI agent's change says in the app's toast and its activity log.
//!
//! "Centre: PEQ band 1 Bell 2 kHz +3 dB Q 1", "LFE → Left woofer off",
//! "Centre woofer: gain −6 dB". Bands, presets and outputs count from 1.
// Indices are bytes: "+ 1" to number them as the screens do cannot overflow.
#![allow(clippy::arithmetic_side_effects)]

use dspi_proto::packets::MatrixRoutePacket;
use dspi_proto::registry::by_path;

use crate::channel_words::ChannelWords;
use crate::events::words::{db_text, filter_name, hz, ms_text, trim, volume};
use crate::journal::{HistoryEntry, band_from_bytes};
use crate::write_param::ParamValue;

/// The first crossover band in a journal path's indices (`XOVER_BAND_BASE`).
const XOVER_BASE: u8 = 20;

/// `e` in words, its channels named by `w`.
pub fn change_text(e: &HistoryEntry, w: &ChannelWords) -> String {
    let ix = |k: usize| e.indices.get(k).copied().map(usize::from);
    let wire = |k: usize| ix(k).map(|c| w.wire(c));
    let output = |k: usize| ix(k).map(|o| w.output(o));
    let input = |k: usize| ix(k).map(|i| w.input(i));
    let on = |b: bool, yes: &str, no: &str| if b { yes.to_owned() } else { no.to_owned() };
    let words = match (e.path.as_str(), &e.after) {
        ("sig.run", _) => e.label.clone(),
        ("eq.band", ParamValue::Bytes(b)) => {
            let band = e.indices.get(1).copied().unwrap_or(0);
            band_from_bytes(b).map(|p| {
                let what = band_name(band);
                let set = if p.filter_type == 0 {
                    "off".to_owned()
                } else {
                    let mut t = format!(
                        "{} {}",
                        filter_name(f64::from(p.filter_type)),
                        hz(f64::from(p.freq))
                    );
                    if band < XOVER_BASE {
                        if p.gain != 0.0 {
                            t.push_str(&format!(" {}", db_text(f64::from(p.gain))));
                        }
                        t.push_str(&format!(" Q {}", trim(f64::from(p.q), 2)));
                    }
                    if p.bypass {
                        t.push_str(", bypassed");
                    }
                    t
                };
                format!("{}: {what} {set}", wire(0).unwrap_or_default())
            })
        }
        (path, value) if path.starts_with("eq.") => {
            let band = e.indices.get(1).copied().unwrap_or(0);
            let set = match (path, value) {
                ("eq.type", ParamValue::Choice(t)) => filter_name(f64::from(*t)),
                ("eq.freq", ParamValue::Float(f)) => format!("frequency {}", hz(f64::from(*f))),
                ("eq.gain" | "eq.fp", ParamValue::Float(g)) => {
                    format!("gain {}", db_text(f64::from(*g)))
                }
                ("eq.q", ParamValue::Float(q)) => format!("Q {}", trim(f64::from(*q), 2)),
                ("eq.bypass", ParamValue::Bool(b)) => on(*b, "bypassed", "in use"),
                _ => "changed".to_owned(),
            };
            Some(format!(
                "{}: {} {set}",
                wire(0).unwrap_or_default(),
                band_name(band)
            ))
        }
        ("mix", ParamValue::Bytes(b)) => MatrixRoutePacket::decode(b).ok().map(|p| {
            let route = format!(
                "{} \u{2192} {}",
                w.input(usize::from(p.input)),
                w.output(usize::from(p.output))
            );
            if !p.enabled {
                return format!("{route} off");
            }
            let invert = if p.phase_invert { ", inverted" } else { "" };
            format!("{route} on at {}{invert}", db_text(f64::from(p.gain_db)))
        }),
        ("out.gain", ParamValue::Float(g)) => {
            output(0).map(|o| format!("{o}: gain {}", db_text(f64::from(*g))))
        }
        ("out.delay", ParamValue::Float(ms)) => {
            output(0).map(|o| format!("{o}: delay {}", ms_text(f64::from(*ms))))
        }
        ("out.mute", ParamValue::Bool(m)) => {
            output(0).map(|o| format!("{o} {}", on(*m, "muted", "unmuted")))
        }
        ("out.enable", ParamValue::Bool(b)) => {
            output(0).map(|o| format!("{o} switched {}", on(*b, "on", "off")))
        }
        ("pre", ParamValue::Float(g)) => {
            input(0).map(|i| format!("{i}: preamp {}", db_text(f64::from(*g))))
        }
        ("pre.all", ParamValue::Float(g)) => Some(format!("Preamp {}", db_text(f64::from(*g)))),
        ("ch.delay", ParamValue::Float(ms)) => {
            wire(0).map(|c| format!("{c}: delay {}", ms_text(f64::from(*ms))))
        }
        ("ch.name", ParamValue::Text(t)) => {
            wire(0).map(|c| format!("{c} renamed \u{201c}{t}\u{201d}"))
        }
        ("vol.user", ParamValue::Float(db)) => Some(format!("Volume {}", volume(f64::from(*db)))),
        ("vol.master", ParamValue::Float(db)) => {
            Some(format!("Master volume {}", volume(f64::from(*db))))
        }
        ("vol.mute", ParamValue::Bool(m)) => Some(on(*m, "Volume muted", "Volume unmuted")),
        ("bypass", ParamValue::Bool(b)) => Some(on(*b, "EQ bypassed", "EQ in use")),
        (path, _) if path.starts_with("preset.") => ix(0).map(|s| {
            let n = s + 1;
            match path {
                "preset.load" => format!("Preset {n} loaded"),
                "preset.save" => format!("Saved to preset {n}"),
                "preset.delete" => format!("Preset {n} cleared"),
                "preset.name" => format!("Preset {n} renamed"),
                _ => format!("Preset {n} changed"),
            }
        }),
        _ => None,
    };
    words.unwrap_or_else(|| fallback(e, w))
}

/// A path these words do not know: the registry's label, where, the value.
fn fallback(e: &HistoryEntry, w: &ChannelWords) -> String {
    let label = by_path(&e.path).map_or_else(|| e.path.clone(), |p| p.label.to_string());
    let where_ = if e.path.starts_with("out.") || e.path.starts_with("limit.") {
        e.indices.first().map(|o| w.output(usize::from(*o)))
    } else {
        None
    };
    let value = match &e.after {
        ParamValue::Bool(b) => Some(if *b { "on" } else { "off" }.to_owned()),
        ParamValue::Float(f) => Some(trim(f64::from(*f), 2)),
        ParamValue::Int(n) => Some(n.to_string()),
        ParamValue::Text(t) => Some(format!("\u{201c}{t}\u{201d}")),
        _ => None,
    };
    let head = where_.map_or(label.clone(), |o| format!("{o}: {}", lower(&label)));
    value.map_or(head.clone(), |v| format!("{head} {v}"))
}

fn lower(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_lowercase().chain(chars).collect())
        .unwrap_or_default()
}

/// "PEQ band 2", or a crossover's "crossover filter 1".
fn band_name(band: u8) -> String {
    match band.checked_sub(XOVER_BASE) {
        Some(x) => format!("crossover filter {}", u16::from(x) + 1),
        None => format!("PEQ band {}", u16::from(band) + 1),
    }
}

/// Several changes of one call in words: up to three, then how many more.
pub fn changes_text(entries: &[HistoryEntry], w: &ChannelWords) -> String {
    let mut lines: Vec<String> = Vec::new();
    for e in entries {
        let line = change_text(e, w);
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    match lines.len() {
        0..=3 => lines.join("; "),
        n => format!(
            "{}; and {} more",
            lines.get(..2).unwrap_or_default().join("; "),
            n - 2
        ),
    }
}

#[cfg(test)]
mod tests_wp_showfix {
    use super::*;
    use crate::journal::HistorySource;

    fn entry(path: &str, indices: Vec<u8>, after: ParamValue) -> HistoryEntry {
        HistoryEntry {
            path: path.into(),
            indices,
            before: None,
            after,
            source: HistorySource::Local,
            at_ms: 0,
            undoable: true,
            label: None,
            id: 1,
            actor: 1,
            actor_label: None,
        }
    }

    fn words() -> ChannelWords {
        ChannelWords {
            inputs: ["L", "R", "C", "LFE"].map(String::from).to_vec(),
            outputs: ["Left woofer", "Left tweeter", "Centre woofer"]
                .map(String::from)
                .to_vec(),
            from_layout: vec![0, 1, 2],
        }
    }

    #[test]
    fn wp_showfix_an_agents_change_reads_as_the_person_knows_it() {
        let w = words();
        let band = crate::records::FilterParams {
            filter_type: 3,
            bypass: false,
            freq: 2000.0,
            q: 1.0,
            gain: 3.0,
            qp: 0.0,
        };
        let peq = entry(
            "eq.band",
            vec![2, 0],
            ParamValue::Bytes(crate::journal::band_bytes(&band)),
        );
        let text = change_text(&peq, &w);
        assert!(text.starts_with("C: PEQ band 1 "), "{text}");
        assert!(text.ends_with(" 2 kHz +3 dB Q 1"), "{text}");
        let off = MatrixRoutePacket {
            input: 3,
            output: 0,
            enabled: false,
            phase_invert: false,
            gain_db: -6.0,
        };
        let route = entry("mix", vec![3, 0], ParamValue::Bytes(off.encode().to_vec()));
        assert_eq!(change_text(&route, &w), "LFE \u{2192} Left woofer off");
        let gain = entry("out.gain", vec![2], ParamValue::Float(-6.0));
        assert_eq!(change_text(&gain, &w), "Centre woofer: gain \u{2212}6 dB");
        let xo = crate::records::FilterParams {
            filter_type: 35,
            freq: 400.0,
            ..band
        };
        let xover = entry(
            "eq.band",
            vec![5, 20],
            ParamValue::Bytes(crate::journal::band_bytes(&xo)),
        );
        assert_eq!(
            change_text(&xover, &w),
            // -- INT016 -- (A10 one filter vocabulary)
            "Left tweeter: crossover filter 1 LR4 High pass 400 Hz"
        );
        let many = vec![gain.clone(), route.clone(), peq.clone(), xover];
        assert!(changes_text(&many, &w).ends_with("; and 2 more"));
    }
}
