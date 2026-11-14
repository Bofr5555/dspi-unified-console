//! `uconsole autoeq preview|apply` on a channel pair (WP P1TM): the core's
//! `autoeq_preview` and `autoeq_apply` (P1AQ). A shape this build does not
//! know leaves its band flat, never the old filter underneath; an output
//! pair's gain stays as calibrated unless `--level`, and the profile's trim
//! is shown instead; the device's state before is filed as a snapshot.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_cmd::COMMAND;
use dspi_core::{AutoEqPreview, AutoEqProfile, DspiCore};
use dspi_proto::channel::ChannelMap;
use dspi_session::autoeq::Entry;
use dspi_tui::shell::connect::{Target, printable};

use super::{Stores, fail, open, usage};
use crate::exit;

/// What `autoeq preview|apply` was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Request<'a> {
    /// Two channels by name; `None` is inputs 1 and 2.
    pub pair: Option<(&'a str, &'a str)>,
    /// Show what would change, write nothing.
    pub preview: bool,
    /// Write the profile's level on an output pair too.
    pub level: bool,
}

pub(crate) fn cmd(target: &Target, entry: &Entry, req: Request<'_>, json: bool) -> u8 {
    let opened = match open(target, &Stores::default_dirs()) {
        Ok(o) => o,
        Err(c) => return c,
    };
    autoeq_on(&opened.core, entry, req, json)
}

/// The device's channels, named as the command grammar names them.
fn channel_map(core: &DspiCore) -> Option<ChannelMap> {
    let s = core.get_state();
    ChannelMap::new(
        s.num_input_channels,
        s.num_output_channels,
        s.num_input_channels.checked_add(s.num_output_channels)?,
    )
    .map(|m| m.with_names(s.channel_names.clone()))
}

/// A channel by its address (`in.1`, `out.3`, a slug of its name) or name.
fn channel(map: &ChannelMap, wanted: &str) -> Option<u8> {
    let want = wanted.trim().to_ascii_lowercase();
    (0..map.num_channels())
        .find(|&ch| map.slug(ch) == want || map.label(ch).to_ascii_lowercase() == want)
}

/// The pair a request names: its two channels, or inputs 1 and 2 (the USB
/// pair, what is listened to; the core's default pair).
fn pair_of(map: &ChannelMap, pair: Option<(&str, &str)>) -> Result<(u8, u8), u8> {
    if let Some((l, r)) = pair {
        let find = |w: &str| {
            channel(map, w).ok_or_else(|| usage(&format!("no channel called `{}`", printable(w))))
        };
        return Ok((find(l)?, find(r)?));
    }
    if map.num_inputs() < 2 {
        return Err(usage(
            "this device has no pair of inputs; name two channels",
        ));
    }
    Ok((0, 1))
}

pub(crate) fn autoeq_on(core: &DspiCore, entry: &Entry, req: Request<'_>, json: bool) -> u8 {
    let Some(map) = channel_map(core) else {
        eprintln!("{COMMAND}: the device's channels could not be read");
        return exit::TRANSPORT;
    };
    let pairs = match pair_of(&map, req.pair) {
        Ok(p) => [p],
        Err(c) => return c,
    };
    let profile = AutoEqProfile::from_entry(entry);
    let level = req.level.then_some(true);
    let mut out = Vec::new();
    for (l, r) in pairs {
        let names = format!("{} and {}", map.slug(l), map.slug(r));
        if req.preview {
            match core.autoeq_preview(profile.clone(), l, r, level) {
                Ok(p) => {
                    if !json {
                        say(&p, &names, "Would apply", &profile.name);
                    }
                    out.push(serde_json::to_value(&p).unwrap_or_default());
                }
                Err(e) => return fail(&e),
            }
            continue;
        }
        match core.autoeq_apply(profile.clone(), l, r, level) {
            Ok(a) => {
                if !json {
                    say(&a.preview, &names, "Applied", &profile.name);
                    for note in &a.notes {
                        println!("  {}", printable(note));
                    }
                    if !a.snapshot_id.is_empty() {
                        println!(
                            "  The settings before are snapshot {}: `{COMMAND} snapshot recall {}` \
                             puts them back.",
                            a.snapshot_id, a.snapshot_id
                        );
                    }
                }
                let mut v = serde_json::to_value(&a).unwrap_or_default();
                if let Some(o) = v.as_object_mut() {
                    // The document is the snapshot's; its id says where it is.
                    o.remove("snapshotDocument");
                }
                out.push(v);
            }
            Err(e) => return fail(&e),
        }
    }
    if json {
        println!("{}", serde_json::Value::Array(out));
    }
    exit::OK
}

fn say(p: &AutoEqPreview, names: &str, verb: &str, name: &str) {
    let bands = p.channels.first().map_or(0, |c| c.bands.len());
    println!("{verb} {} to {names}: {bands} bands", printable(name));
    match p.level_target.as_str() {
        "none" => println!(
            "  Level left as it is; the profile expects {:+.1} dB lower if it clips.",
            p.suggested_trim_db
        ),
        "output_gain" => println!("  Output gain {:+.1} dB", p.suggested_trim_db),
        _ => println!("  Preamp {:+.1} dB", p.suggested_trim_db),
    }
    for line in &p.notes {
        println!("  {}", printable(line));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::super::tests::demo;
    use super::*;

    fn entry(kinds: &[&str]) -> Entry {
        let filters: Vec<serde_json::Value> = kinds
            .iter()
            .enumerate()
            .map(|(i, k)| {
                serde_json::json!({ "type": k, "freq": 100.0 * (i as f64 + 1.0), "q": 1.0, "gain": -3.0 })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "id": "test/one", "manufacturer": "Test", "model": "One",
            "preamp": -4.5, "filters": filters,
        }))
        .unwrap()
    }

    #[test]
    fn wp_p1tm_autoeq_clears_an_unknown_shapes_band() {
        let (core, _dir) = demo("autoeq");
        let map = channel_map(&core).unwrap();
        // Band 2 of input 1 holds a filter before.
        let held = dspi_core::records::FilterParams {
            filter_type: dspi_proto::FilterType::Peaking.to_raw(),
            bypass: false,
            freq: 3000.0,
            q: 2.0,
            gain: 6.0,
            qp: 0.0,
        };
        core.set_filter(0, 1, held).unwrap();
        let e = entry(&["PK", "WEIRD", "PK"]);
        let req = Request {
            pair: None,
            preview: true,
            level: false,
        };
        // A preview writes nothing.
        assert_eq!(autoeq_on(&core, &e, req, true), exit::OK);
        assert_eq!(core.get_state().filters[0][1].gain, 6.0);
        let req = Request {
            preview: false,
            ..req
        };
        assert_eq!(autoeq_on(&core, &e, req, false), exit::OK);
        let st = core.get_state();
        // The unknown shape's band is flat on inputs 1 and 2; the others
        // hold the profile's.
        for ch in [0usize, 1] {
            assert_eq!(
                st.filters[ch][1].filter_type,
                dspi_proto::FilterType::Flat.to_raw(),
                "ch {ch}"
            );
            assert_eq!(
                st.filters[ch][0].filter_type,
                dspi_proto::FilterType::Peaking.to_raw()
            );
        }
        // The state before was filed as a snapshot.
        assert_eq!(core.snapshot_list(String::new()).len(), 1);
        assert!(map.num_inputs() >= 2);
    }

    #[test]
    fn wp_p1tm_autoeq_on_an_output_pair_keeps_their_gain() {
        let (core, _dir) = demo("autoeq-out");
        let map = channel_map(&core).unwrap();
        let (l, r) = (map.slug(map.num_inputs()), map.slug(map.num_inputs() + 1));
        let gains = core.get_state().output_gain_db.clone();
        let e = entry(&["PK"]);
        let req = Request {
            pair: Some((&l, &r)),
            preview: false,
            level: false,
        };
        assert_eq!(autoeq_on(&core, &e, req, true), exit::OK);
        assert_eq!(core.get_state().output_gain_db, gains);
        let req = Request {
            pair: Some(("nope", &r)),
            ..req
        };
        assert_eq!(autoeq_on(&core, &e, req, true), exit::USAGE);
    }
}
