//! -- WP SHOWFIX -- (finding 5) A measurement's signal generator in the
//! history as one row. Each sweep writes the generator's settings and starts
//! it (`sig.config`, `sig.control`), so one measurement filled a screen of
//! "Signal generator settings" and "Signal generator started". A run of such
//! changes by one writer with two starts or more is one `sig.run` entry:
//! its indices the outputs played on, its value the number of starts, its
//! label the row in words ("Measurement: Out 1–3, 6 sweeps").

use dspi_proto::packets::SiggenConfig;

use super::HistoryEntry;
use crate::channel_words::ChannelWords;
use crate::write_param::ParamValue;

/// The path of a run of the generator's changes.
pub(crate) const RUN_PATH: &str = "sig.run";

/// `SIGGEN_CTL_START`.
const START: u8 = 1;

/// `entries` (newest first) with each run of the generator's changes by one
/// writer that started it twice or more made one entry.
pub(crate) fn fold(entries: Vec<HistoryEntry>, words: &ChannelWords) -> Vec<HistoryEntry> {
    let mut out: Vec<HistoryEntry> = Vec::with_capacity(entries.len());
    let mut run: Vec<HistoryEntry> = Vec::new();
    for e in entries {
        let joins = e.path.starts_with("sig.")
            && run
                .first()
                .is_none_or(|r| r.actor == e.actor && r.source == e.source);
        if joins {
            run.push(e);
            continue;
        }
        flush(&mut run, &mut out, words);
        if e.path.starts_with("sig.") {
            run.push(e);
        } else {
            out.push(e);
        }
    }
    flush(&mut run, &mut out, words);
    out
}

fn flush(run: &mut Vec<HistoryEntry>, out: &mut Vec<HistoryEntry>, words: &ChannelWords) {
    let starts = run
        .iter()
        .filter(|e| e.path == "sig.control" && e.after == ParamValue::Choice(START))
        .count();
    if starts < 2 {
        out.append(run);
        return;
    }
    let configs: Vec<SiggenConfig> = run
        .iter()
        .filter(|e| e.path == "sig.config")
        .filter_map(|e| match &e.after {
            ParamValue::Bytes(b) => SiggenConfig::decode(b).ok(),
            _ => None,
        })
        .collect();
    let mask = configs.iter().fold(0u16, |m, c| m | c.channel_mask);
    let outputs: Vec<u8> = (0..16u8)
        .filter(|o| mask & 1u16.checked_shl(u32::from(*o)).unwrap_or(0) != 0)
        .collect();
    // SIGGEN_TYPE_SWEEP_LOG, _LIN, _STEP
    let sweeps = !configs.is_empty() && configs.iter().all(|c| (4..=6).contains(&c.signal_type));
    let what = match (sweeps, starts) {
        (true, _) => format!("{starts} sweeps"),
        (false, _) => format!("{starts} test signals"),
    };
    let label = if outputs.is_empty() {
        format!("Measurement: {what}")
    } else {
        format!("Measurement: {}, {what}", outputs_text(&outputs, words))
    };
    let mut newest = run.swap_remove(0);
    run.clear();
    newest.path = RUN_PATH.to_owned();
    newest.indices = outputs;
    newest.before = None;
    newest.after = ParamValue::Int(i64::try_from(starts).unwrap_or(i64::MAX));
    newest.undoable = false;
    newest.label = Some(label);
    out.push(newest);
}

/// The outputs by their names; numbered outputs in a row as a range
/// ("Out 1–3").
fn outputs_text(outputs: &[u8], words: &ChannelWords) -> String {
    let names: Vec<String> = outputs
        .iter()
        .map(|o| words.output(usize::from(*o)))
        .collect();
    let numbered = names
        .iter()
        .zip(outputs)
        .all(|(n, o)| *n == format!("Out {}", u16::from(*o).saturating_add(1)));
    let in_a_row = outputs.windows(2).all(|w| match w {
        [a, b] => b.checked_sub(*a) == Some(1),
        _ => true,
    });
    match (outputs.first(), outputs.last()) {
        (Some(first), Some(last)) if numbered && in_a_row && outputs.len() > 2 => format!(
            "Out {}\u{2013}{}",
            u16::from(*first).saturating_add(1),
            u16::from(*last).saturating_add(1)
        ),
        _ => names.join(", "),
    }
}

#[cfg(test)]
mod tests_wp_showfix {
    use super::*;
    use crate::journal::HistorySource;

    fn entry(id: u64, path: &str, after: ParamValue) -> HistoryEntry {
        HistoryEntry {
            path: path.into(),
            indices: Vec::new(),
            before: None,
            after,
            source: HistorySource::Local,
            at_ms: id,
            undoable: false,
            label: None,
            id,
            actor: 0,
            actor_label: None,
        }
    }

    fn config(mask: u16) -> ParamValue {
        let cfg = SiggenConfig {
            signal_type: 4,
            channel_mask: mask,
            ..SiggenConfig::default()
        };
        ParamValue::Bytes(cfg.encode().to_vec())
    }

    #[test]
    fn wp_showfix_a_measurement_is_one_history_row() {
        let words = ChannelWords {
            inputs: vec!["L".into(), "R".into()],
            outputs: vec![
                "Out 1".into(),
                "Out 2".into(),
                "Out 3".into(),
                "Out 4".into(),
            ],
            from_layout: Vec::new(),
        };
        // Newest first: a gain after the run, the run (three sweeps on Out 1-3), a gain before.
        let mut list = vec![entry(9, "out.gain", ParamValue::Float(-3.0))];
        for (i, mask) in [(8, 4), (6, 2), (4, 1)] {
            list.push(entry(i, "sig.control", ParamValue::Choice(START)));
            list.push(entry(i - 1, "sig.config", config(mask)));
        }
        list.push(entry(1, "out.gain", ParamValue::Float(0.0)));
        let folded = fold(list, &words);
        assert_eq!(folded.len(), 3);
        let run = &folded[1];
        assert_eq!(run.path, RUN_PATH);
        assert_eq!(run.indices, vec![0, 1, 2]);
        assert_eq!(
            run.label.as_deref(),
            Some("Measurement: Out 1\u{2013}3, 3 sweeps")
        );
        assert_eq!(run.id, 8);
        // Named outputs by name; one start alone stays as it is.
        let named = ChannelWords {
            outputs: vec!["Centre woofer".into(), "Centre tweeter".into()],
            ..words.clone()
        };
        let two = vec![
            entry(4, "sig.control", ParamValue::Choice(START)),
            entry(3, "sig.config", config(2)),
            entry(2, "sig.control", ParamValue::Choice(START)),
            entry(1, "sig.config", config(1)),
        ];
        assert_eq!(
            fold(two, &named)[0].label.as_deref(),
            Some("Measurement: Centre woofer, Centre tweeter, 2 sweeps")
        );
        let one = vec![
            entry(2, "sig.control", ParamValue::Choice(START)),
            entry(1, "sig.config", config(1)),
        ];
        assert_eq!(fold(one, &words).len(), 2);
    }
}
