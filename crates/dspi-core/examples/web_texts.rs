//! WP WB5: the core's shared wording as JSON, for the web client until WB3's wasm
//! binding answers it at run time. `ci/drift.sh` writes it to
//! `ci/generated/web/link-words.json` and checks it, like the other generated code.

use dspi_core::link_phase_texts;
use serde_json::{Value, json};

fn main() {
    let link: Vec<Value> = link_phase_texts()
        .into_iter()
        .map(|t| {
            json!({
                "key": t.key,
                "phase": t.phase,
                "text": t.text,
                "action_key": t.action_key,
                "action": t.action,
            })
        })
        .collect();
    let doc = json!({ "link": link });
    let text = serde_json::to_string_pretty(&doc).expect("JSON of plain strings");
    println!("{text}");
}
