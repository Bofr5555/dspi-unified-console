//! Prompts: recipes the user picks in the agent's host. Static text in the
//! server's words; the arguments are numbers only, so nothing a device or a
//! file holds can reach a prompt.

use serde_json::{Map, Value, json};

use crate::protocol::{INVALID_PARAMS, RpcError};

struct Prompt {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    /// `(name, description, required)`; every argument is a number.
    arguments: &'static [(&'static str, &'static str, bool)],
    text: &'static str,
}

const PROMPTS: &[Prompt] = &[
    Prompt {
        name: "match_channel_response",
        title: "Match a channel's response",
        description: "Look at one output's response and propose PEQ bands toward a flat target",
        arguments: &[("output", "The output, from 0", true)],
        text: "Read dspi://guide. Call dspi_get_overview, then dspi_get_channel and \
               dspi_get_response for output {output}. If a measurement of it exists \
               (dspi_list_measurements), read its response too. Describe how the response \
               differs from flat and propose PEQ bands (type, frequency, gain, Q) that would \
               bring it closer, cuts before boosts. This server cannot apply them yet: list \
               them for the user.",
    },
    Prompt {
        name: "tune_speaker_system",
        title: "Review a speaker system",
        description: "Review every output's crossover, levels and delays against the speaker setup",
        arguments: &[],
        text: "Read dspi://guide. Call dspi_get_overview. For every output, call dspi_get_channel. \
               Check that each output's crossover suits its role from the speaker setup (a high \
               role keeps a high-pass at or above 1 kHz), that the ways of one speaker cross at \
               matching frequencies, and that levels and delays look deliberate. Report what \
               looks wrong and what you would change.",
    },
    Prompt {
        name: "tame_peak",
        title: "Tame a peak",
        description: "Propose a cut for a peak at a frequency on one output",
        arguments: &[
            ("output", "The output, from 0", true),
            ("freq_hz", "The peak's frequency in Hz", true),
        ],
        text: "Read dspi://guide. Call dspi_get_channel and dspi_get_response for output \
               {output}. Find the peak near {freq_hz} Hz and propose one peaking cut (frequency, \
               negative gain, Q) in a free PEQ slot that flattens it, without touching the \
               crossover.",
    },
    Prompt {
        name: "set_up_installation",
        title: "Review an installation",
        description: "Review the open devices as one installation",
        arguments: &[],
        text: "Read dspi://guide. Call dspi_list_devices, then dspi_get_overview for each device. \
               Describe the installation: which device drives which speakers, and anything that \
               looks inconsistent between devices (levels, delays, crossovers of matching ways).",
    },
];

/// `prompts/list`'s entries.
pub fn list() -> Vec<Value> {
    PROMPTS
        .iter()
        .map(|p| {
            let args: Vec<Value> = p
                .arguments
                .iter()
                .map(|(n, d, r)| json!({"name": n, "description": d, "required": r}))
                .collect();
            json!({"name": p.name, "title": p.title, "description": p.description,
                   "arguments": args})
        })
        .collect()
}

/// A number argument as text: what the host sent, if it reads as a finite
/// number in a sane range (hosts send prompt arguments as strings).
fn number(value: &Value) -> Option<String> {
    let n = match value {
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        other => other.as_f64()?,
    };
    (n.is_finite() && (0.0..=100_000.0).contains(&n)).then(|| {
        if n.fract() == 0.0 {
            format!("{}", n as u64)
        } else {
            format!("{n}")
        }
    })
}

/// `prompts/get`.
pub fn get(params: &Map<String, Value>) -> Result<Value, RpcError> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let Some(p) = PROMPTS.iter().find(|p| p.name == name) else {
        return Err(RpcError::new(
            INVALID_PARAMS,
            "no such prompt; prompts/list lists them",
        ));
    };
    let empty = Map::new();
    let args = params
        .get("arguments")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let mut text = p.text.to_owned();
    for (arg, _, required) in p.arguments {
        match args.get(*arg).map(number) {
            Some(Some(n)) => text = text.replace(&format!("{{{arg}}}"), &n),
            Some(None) => {
                return Err(RpcError::new(
                    INVALID_PARAMS,
                    format!("`{arg}` must be a number"),
                ));
            }
            None if *required => {
                return Err(RpcError::new(
                    INVALID_PARAMS,
                    format!("`{arg}` is required"),
                ));
            }
            None => {}
        }
    }
    Ok(json!({
        "description": p.description,
        "messages": [{"role": "user", "content": {"type": "text", "text": text}}],
    }))
}
