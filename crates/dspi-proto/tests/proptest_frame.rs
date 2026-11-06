//! WP P1: property tests for the control frame and the SysEx decoder
//! (PLAN §1.9). The decoder reads bytes from an app-supplied MIDI pipe, so it
//! must survive anything in any split (§1.7).

use dspi_proto::frame::{
    DEFAULT_MFR_ID, Frame, MAX_SYSEX_LEN, SYSEX_VERSION, SysexDecoder, encode_sysex, pack7, unpack7,
};
use proptest::prelude::*;

/// Payloads up to a whole bulk image and a bit; the length field is 16 bits,
/// and no legal frame comes near that.
fn payload() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        4 => prop::collection::vec(any::<u8>(), 0..80),
        1 => prop::collection::vec(any::<u8>(), 0..7000),
    ]
}

fn frame() -> impl Strategy<Value = Frame> {
    prop_oneof![
        (any::<u8>(), any::<u16>(), any::<u16>(), payload()).prop_map(
            |(request, value, index, payload)| Frame::Set {
                request,
                value,
                index,
                payload,
            }
        ),
        (any::<u8>(), any::<u16>(), any::<u16>(), any::<u16>()).prop_map(
            |(request, value, index, length)| Frame::Get {
                request,
                value,
                index,
                length,
            }
        ),
        any::<u8>().prop_map(|status| Frame::SetResponse { status }),
        (any::<u8>(), payload())
            .prop_map(|(status, payload)| Frame::GetResponse { status, payload }),
        payload().prop_map(|packet| Frame::Notification { packet }),
    ]
}

/// A manufacturer id as the spec allows: one byte, or `00` and two more.
fn mfr() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        (0x01u8..0x7F).prop_map(|b| vec![b]),
        (0u8..0x80, 0u8..0x80).prop_map(|(a, b)| vec![0x00, a, b]),
    ]
}

/// Cut `bytes` at the given points (taken modulo its length, sorted).
fn split(bytes: &[u8], cuts: &[usize]) -> Vec<Vec<u8>> {
    if bytes.is_empty() {
        return vec![Vec::new()];
    }
    let mut at: Vec<usize> = cuts.iter().map(|c| c % bytes.len()).collect();
    at.push(0);
    at.push(bytes.len());
    at.sort_unstable();
    at.dedup();
    at.windows(2).map(|w| bytes[w[0]..w[1]].to_vec()).collect()
}

proptest! {
    #[test]
    fn proptest_wp_p1_frame_decode_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..200)) {
        let _ = Frame::decode(&bytes);
    }

    /// Bytes that look like a frame's start (a known type byte and a length
    /// field that may or may not agree) reach the deeper paths.
    #[test]
    fn proptest_wp_p1_frame_decode_never_panics_on_frame_like_bytes(
        ty in prop::sample::select(vec![0x01u8, 0x02, 0x81, 0x82, 0x40]),
        rest in prop::collection::vec(any::<u8>(), 0..120),
    ) {
        let mut b = vec![ty];
        b.extend(rest);
        let _ = Frame::decode(&b);
    }

    #[test]
    fn proptest_wp_p1_frame_round_trips(f in frame()) {
        prop_assert_eq!(f.check_len(), Ok(()));
        prop_assert_eq!(Frame::decode(&f.encode()), Ok(f.clone()));
        let uart = f.encode_uart();
        prop_assert_eq!(uart[0], dspi_proto::frame::SYNC);
        prop_assert_eq!(Frame::decode(&uart[1..]), Ok(f));
    }

    /// One flipped bit anywhere is caught (by the length checks or the CRC).
    #[test]
    fn proptest_wp_p1_a_corrupted_frame_is_refused(f in frame(), at in any::<usize>(), bit in 0u8..8) {
        let mut b = f.encode();
        let i = at % b.len();
        b[i] ^= 1 << bit;
        prop_assert!(Frame::decode(&b).is_err());
    }

    #[test]
    fn proptest_wp_p1_pack7_round_trips(data in prop::collection::vec(any::<u8>(), 0..2000)) {
        let p = pack7(&data);
        prop_assert!(p.iter().all(|b| b & 0x80 == 0), "packed bytes are 7-bit");
        prop_assert_eq!(unpack7(&p), Some(data));
    }

    /// `unpack7` never panics, and what it accepts is exactly `pack7`'s
    /// canonical form.
    #[test]
    fn proptest_wp_p1_unpack7_never_panics(data in prop::collection::vec(any::<u8>(), 0..300)) {
        if let Some(out) = unpack7(&data) {
            prop_assert_eq!(pack7(&out), data);
        }
    }

    #[test]
    fn proptest_wp_p1_unpack7_never_panics_on_7bit_bytes(data in prop::collection::vec(0u8..0x80, 0..300)) {
        if let Some(out) = unpack7(&data) {
            prop_assert_eq!(pack7(&out), data);
        }
    }

    #[test]
    fn proptest_wp_p1_sysex_feed_never_panics(
        mfr in mfr(),
        bytes in prop::collection::vec(any::<u8>(), 0..2000),
        cuts in prop::collection::vec(any::<usize>(), 0..20),
    ) {
        let mut d = SysexDecoder::new(&mfr);
        for piece in split(&bytes, &cuts) {
            let _ = d.feed(&piece);
        }
    }

    /// Streams built from SysEx-shaped pieces (our header, foreign headers,
    /// status and real-time bytes) get past the header checks.
    #[test]
    fn proptest_wp_p1_sysex_feed_never_panics_on_sysex_like_bytes(
        pieces in prop::collection::vec(prop_oneof![
            Just(vec![0xF0, 0x7D, b'D', b'S', b'P']),
            Just(vec![0xF0, 0x7D, b'D', b'S', b'P', SYSEX_VERSION]),
            Just(vec![0xF7]),
            Just(vec![0xF0]),
            (0x80u8..=0xFF).prop_map(|b| vec![b]),
            prop::collection::vec(0u8..0x80, 0..40),
        ], 0..40),
        cuts in prop::collection::vec(any::<usize>(), 0..20),
    ) {
        let bytes: Vec<u8> = pieces.concat();
        let mut d = SysexDecoder::new(DEFAULT_MFR_ID);
        for piece in split(&bytes, &cuts) {
            for m in d.feed(&piece) {
                // Whatever is reported decoded or failed with a reason; it
                // never carries more than the cap could hold.
                if let Ok(f) = m.frame {
                    prop_assert!(f.encode().len() < MAX_SYSEX_LEN);
                }
            }
        }
    }

    /// A valid message between noise, with real-time bytes inside it and the
    /// stream split anywhere, is found exactly once.
    #[test]
    fn proptest_wp_p1_a_message_inside_noise_is_found_exactly_once(
        mfr in mfr(),
        tag in 1u8..0x7F,
        f in frame(),
        before in prop::collection::vec(any::<u8>(), 0..200),
        after in prop::collection::vec(any::<u8>(), 0..200),
        realtime in prop::collection::vec((any::<usize>(), 0xF8u8..=0xFF), 0..8),
        cuts in prop::collection::vec(any::<usize>(), 0..20),
    ) {
        let msg = encode_sysex(&mfr, tag, &f);
        prop_assume!(msg.len() <= MAX_SYSEX_LEN);
        let mut inner = msg.clone();
        for (at, rt) in realtime {
            // Strictly inside: after F0, before F7.
            let i = 1 + at % (inner.len() - 1);
            inner.insert(i, rt);
        }
        let mut stream = before;
        stream.extend(&inner);
        stream.extend(after);

        let mut d = SysexDecoder::new(&mfr);
        let mut found = 0;
        for piece in split(&stream, &cuts) {
            found += d
                .feed(&piece)
                .into_iter()
                .filter(|m| m.tag == tag && m.frame.as_ref() == Ok(&f))
                .count();
        }
        prop_assert_eq!(found, 1);
    }

    /// A message of ours over the cap is dropped, not buffered: nothing is
    /// reported for it, and the next message decodes.
    #[test]
    fn proptest_wp_p1_an_oversized_message_is_dropped(
        extra in 1usize..20_000,
        cuts in prop::collection::vec(any::<usize>(), 0..20),
    ) {
        let mut stream = vec![0xF0, 0x7D, b'D', b'S', b'P', SYSEX_VERSION, 3];
        let body = MAX_SYSEX_LEN - stream.len() - 1 + extra;
        stream.extend((0..body).map(|i| (i % 0x80) as u8));
        stream.push(0xF7);
        prop_assert!(stream.len() > MAX_SYSEX_LEN);
        let next = Frame::SetResponse { status: 0 };
        stream.extend(encode_sysex(DEFAULT_MFR_ID, 4, &next));

        let mut d = SysexDecoder::new(DEFAULT_MFR_ID);
        let mut got = Vec::new();
        for piece in split(&stream, &cuts) {
            got.extend(d.feed(&piece));
        }
        prop_assert_eq!(got.len(), 1);
        prop_assert_eq!(got[0].tag, 4);
        prop_assert_eq!(&got[0].frame, &Ok(next));
    }
}
