//! What a filed measurement says to an agent (WP FFIX, FUNC1 bugs 3, 9 and
//! 15): each output's arrival, the chain it was measured through (its
//! high-pass named, as the response includes it), and for a run of one
//! timing origin the delays that align the device's speakers.
//!
//! - **Arrivals** (`arrival_ms`) are the sound's arrival at the microphone
//!   from its own cycle start, with the delay the output had then. They
//!   compare between outputs of **one run**: one speaker's outputs in a
//!   walked measurement (a run per speaker), or every output of a measurement
//!   started with `one_run`. Two runs (and two measurements) start at
//!   different moments of the microphone's path, so theirs do not compare.
//! - **The high-pass in the response.** A measured response is the driver
//!   through the output's filters as they were: an active high-pass shapes
//!   its low end. The results say so, and name it, rather than divide it out
//!   (the auto-tune de-embeds for itself; a response with the filter taken out
//!   would show a tweeter's low end that it must never be played at).
//! - **The alignment across speakers** (a `one_run` measurement): each
//!   speaker's arrival is its lowest way's (not a subwoofer: its arrival is
//!   too broad to time, it is aligned at its crossover by the summed check);
//!   the earlier speakers get the difference as more delay on every one of
//!   their outputs, so each speaker's own alignment stays.

use dspi_core::{DspState, SpeakerLayout};
use serde_json::{Value, json};

use crate::policy;
use crate::tools::read::{shown_all, type_word};

/// The chain a document's output was measured through, in words: its active
/// high-passes ("24 dB/octave high-pass at 2000 Hz"), if any.
fn highpass_words(chain: &Value) -> Vec<String> {
    ["peq", "crossover"]
        .iter()
        .filter_map(|k| chain.get(*k).and_then(Value::as_array))
        .flatten()
        .filter(|b| b.get("bypass").and_then(Value::as_bool) != Some(true))
        .filter_map(|b| {
            let t = u8::try_from(b.get("filterType")?.as_u64()?).ok()?;
            let slope = policy::highpass_slope(t)?;
            let hz = b.get("freq")?.as_f64()?;
            Some(format!(
                "{slope} dB/octave high-pass at {hz:.0} Hz ({})",
                type_word(t)
            ))
        })
        .collect()
}

/// The note on a measured response that includes a high-pass.
pub(crate) fn highpass_note(chain: &Value) -> Option<String> {
    let hp = highpass_words(chain);
    (!hp.is_empty()).then(|| {
        format!(
            "this response includes the output's {} as it was measured: below that corner the \
             roll-off is the filter's, not the driver's own (the low end is not de-embedded)",
            hp.join(" and ")
        )
    })
}

/// One output of a filed measurement, as an agent reads it.
fn output_summary(o: &Value) -> Value {
    let chain = o.get("chain").cloned().unwrap_or(Value::Null);
    json!({
        // -- WP SHOWFIX -- the document's output from 0; the agent's from 1
        "output": o.get("output").and_then(Value::as_u64).map(|n| n.saturating_add(1)),
        "role": o.get("role"),
        "speaker": o.get("speaker").and_then(Value::as_str).map(crate::untrusted::name),
        "arrival_ms": o.get("arrivalMs"),
        "measured_delay_ms": chain.get("delayMs"),
        "highpass": highpass_words(&chain),
    })
}

/// A filed measurement document's outputs and kind, for `dspi_list_measurements`
/// and the end of a request.
pub(crate) fn summary(doc_text: &str) -> Option<Value> {
    let doc: Value = serde_json::from_str(doc_text).ok()?;
    let m = doc.get("measurement")?;
    let outputs: Vec<Value> = m
        .get("outputs")?
        .as_array()?
        .iter()
        .map(output_summary)
        .collect();
    let summed = m.pointer("/stimulus/kind").and_then(Value::as_str) == Some("summed");
    Some(json!({
        "kind": m.pointer("/stimulus/kind"),
        "timing_reliable": m.pointer("/capture/timingReliable"),
        "outputs": outputs,
        "note": if summed {
            "a summed run: one response of the outputs together, filed under the first output"
        } else {
            "arrival_ms compare between outputs of one run only: one speaker's, or all of a \
             one_run measurement"
        },
    }))
}

/// The delays that align the speakers of a `one_run` measurement `doc_text`
/// on the device as it is now (`st`, at most `max_delay_ms`): per speaker its
/// arrival and the delay it is given, and the writes; `None` when the
/// document has no timing to align by.
///
/// `layout` (the user's confirmed speaker layout) adds each speaker's outputs
/// the run left out, so a speaker measured by its woofer alone keeps its
/// tweeter aligned to it.
pub(crate) fn alignment(
    doc_text: &str,
    st: &DspState,
    max_delay_ms: f32,
    layout: Option<&SpeakerLayout>,
) -> Option<Value> {
    let doc: Value = serde_json::from_str(doc_text).ok()?;
    let m = doc.get("measurement")?;
    if m.pointer("/capture/timingReliable")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return None;
    }
    let outs = m.get("outputs")?.as_array()?;
    let rank = |r: &str| match r {
        "low" => 0,
        "full" => 1,
        "mid" => 2,
        "high" => 3,
        _ => 9,
    };
    let now_delay = |o: u8| {
        st.output_delay_ms
            .get(usize::from(o))
            .copied()
            .unwrap_or(0.0)
    };
    // speaker -> (its reference arrival now, its outputs)
    let mut speakers: Vec<(String, f64, Vec<u8>)> = Vec::new();
    let mut left_out: Vec<String> = Vec::new();
    let mut names: Vec<String> = outs
        .iter()
        .filter_map(|o| o.get("speaker").and_then(Value::as_str).map(str::to_owned))
        .collect();
    names.sort();
    names.dedup();
    for name in names {
        let ways: Vec<&Value> = outs
            .iter()
            .filter(|o| o.get("speaker").and_then(Value::as_str) == Some(name.as_str()))
            .collect();
        let mut outputs: Vec<u8> = ways
            .iter()
            .filter_map(|o| u8::try_from(o.get("output")?.as_u64()?).ok())
            .collect();
        for o in layout.into_iter().flat_map(|l| &l.outputs) {
            if o.speaker == name && !outputs.contains(&o.output) {
                outputs.push(o.output);
            }
        }
        outputs.sort_unstable();
        let reference = ways
            .iter()
            .filter(|o| o.get("arrivalMs").and_then(Value::as_f64).is_some())
            .filter(|o| o.get("role").and_then(Value::as_str) != Some("sub"))
            .min_by_key(|o| rank(o.get("role").and_then(Value::as_str).unwrap_or("")));
        let Some(r) = reference else {
            left_out.push(crate::untrusted::name(&name));
            continue;
        };
        let (Some(arrival), Some(o)) = (
            r.get("arrivalMs").and_then(Value::as_f64),
            r.get("output")
                .and_then(Value::as_u64)
                .and_then(|o| u8::try_from(o).ok()),
        ) else {
            continue;
        };
        let measured = r
            .pointer("/chain/delayMs")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        speakers.push((name, arrival - measured + f64::from(now_delay(o)), outputs));
    }
    if speakers.len() < 2 {
        return None;
    }
    let latest = speakers
        .iter()
        .map(|s| s.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut rows = Vec::new();
    let mut changes = Vec::new();
    let mut over = Vec::new();
    for (name, arrival, outputs) in &speakers {
        let add = latest - arrival;
        rows.push(json!({"speaker": crate::untrusted::name(name), "arrival_ms": (arrival * 1e4).round() / 1e4,
                         "add_ms": (add * 1e4).round() / 1e4, "outputs": shown_all(outputs)}));
        for o in outputs {
            let to = f64::from(now_delay(*o)) + add;
            if add < 0.000_05 {
                continue;
            }
            if to > f64::from(max_delay_ms) {
                over.push(*o);
                continue;
            }
            changes.push(
                json!({"op": "set_delay", "output": u16::from(*o).saturating_add(1),
                       "delay_ms": (to * 1e4).round() / 1e4}),
            );
        }
    }
    let mut note = "the speakers' arrivals at the microphone, each by its lowest way; the \
                    earlier ones get the difference as more delay on every one of their \
                    outputs (dspi_apply_changes with `changes` as given), so each speaker's own \
                    alignment stays. Only for the listening position it was measured at."
        .to_owned();
    if !left_out.is_empty() {
        note.push_str(&format!(
            " Left out (a subwoofer, or no arrival): {}; align a subwoofer at its crossover \
             (dspi_tune_sum_check).",
            left_out.join(", ")
        ));
    }
    if !over.is_empty() {
        let outs: Vec<String> = over
            .iter()
            .map(|o| format!("Out {}", u16::from(*o).saturating_add(1)))
            .collect();
        note.push_str(&format!(
            " {} would need more than the device's {max_delay_ms} ms: not offered.",
            outs.join(", ")
        ));
    }
    Some(json!({"speakers": rows, "changes": changes, "note": note}))
}
