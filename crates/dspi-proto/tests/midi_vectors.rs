//! The firmware's MIDI control-channel vectors, through this crate's codec.

use dspi_proto::frame::{Frame, SysexDecoder, encode_sysex};

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn every_vector_encodes_and_decodes_exactly() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/midi/midi_vectors.json");
    let text = std::fs::read_to_string(path).unwrap();
    let mut n = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let name = v["name"].as_str().unwrap();
        let mfr = hex(v["mfr"].as_str().unwrap());
        let tag = v["tag"].as_u64().unwrap() as u8;
        let frame_bytes = hex(v["frame"].as_str().unwrap());
        let sysex = hex(v["sysex"].as_str().unwrap());

        let frame = Frame::decode(&frame_bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(frame.encode(), frame_bytes, "{name}: frame re-encodes");
        assert_eq!(encode_sysex(&mfr, tag, &frame), sysex, "{name}: SysEx");

        let mut d = SysexDecoder::new(&mfr);
        let got = d.feed(&sysex);
        assert_eq!(got.len(), 1, "{name}");
        assert_eq!(got[0].tag, tag, "{name}");
        assert_eq!(got[0].frame.as_ref().unwrap(), &frame, "{name}");
        n += 1;
    }
    assert!(n >= 13, "only {n} vectors read");
}
