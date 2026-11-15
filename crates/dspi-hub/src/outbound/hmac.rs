//! HMAC-SHA256 (RFC 2104) over the `sha2` crate the hub already has (WP P2WH): the
//! `X-DSPi-Signature` of a webhook. Hand-rolled because the `hmac` crate is not in the
//! graph; tested against RFC 4231's vectors.

use data_encoding::HEXLOWER;
use sha2::{Digest, Sha256};

/// SHA-256's block size in bytes.
const BLOCK: usize = 64;

/// HMAC-SHA256 of the concatenation of `parts`, keyed with `key`.
pub fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    // A key longer than a block is hashed first; a shorter one is padded with zeros.
    let hashed: [u8; 32];
    let key = if key.len() > BLOCK {
        hashed = Sha256::digest(key).into();
        hashed.as_slice()
    } else {
        key
    };
    let mut block = [0u8; BLOCK];
    for (b, k) in block.iter_mut().zip(key) {
        *b = *k;
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|b| b ^ 0x36));
    for part in parts {
        inner.update(part);
    }
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(block.map(|b| b ^ 0x5c));
    outer.update(inner);
    outer.finalize().into()
}

/// The `X-DSPi-Signature` value: `sha256=<hex HMAC-SHA256(secret, timestamp + "." + body)>`.
/// The key is the secret's text as the hub showed it.
pub fn signature(secret: &str, timestamp: u64, body: &[u8]) -> String {
    let ts = timestamp.to_string();
    let mac = hmac_sha256(secret.as_bytes(), &[ts.as_bytes(), b".", body]);
    format!("sha256={}", HEXLOWER.encode(&mac))
}

#[cfg(test)]
mod tests_wp_p2wh {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        HEXLOWER.decode(text.as_bytes()).unwrap()
    }

    fn mac(key: &[u8], data: &[u8]) -> String {
        HEXLOWER.encode(&hmac_sha256(key, &[data]))
    }

    /// RFC 4231 section 4, test cases 1 to 7 (HMAC-SHA-256).
    #[test]
    fn wp_p2wh_rfc4231_vectors() {
        assert_eq!(
            mac(&[0x0b; 20], b"Hi There"),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            mac(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            mac(&[0xaa; 20], &[0xdd; 50]),
            "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe"
        );
        assert_eq!(
            mac(
                &hex("0102030405060708090a0b0c0d0e0f10111213141516171819"),
                &[0xcd; 50]
            ),
            "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b"
        );
        // Case 5 is truncated to 128 bits.
        assert_eq!(
            &mac(&[0x0c; 20], b"Test With Truncation")[..32],
            "a3b6167473100ee06e0c796c2955552b"
        );
        // Cases 6 and 7: a key longer than a block is hashed first.
        assert_eq!(
            mac(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            ),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
        assert_eq!(
            mac(
                &[0xaa; 131],
                b"This is a test using a larger than block-size key and a larger than block-size data. The key needs to be hashed before being used by the HMAC algorithm."
            ),
            "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2"
        );
    }

    #[test]
    fn wp_p2wh_parts_are_concatenated_and_a_block_sized_key_is_used_as_is() {
        let whole = hmac_sha256(b"k", &[b"12.{}"]);
        assert_eq!(hmac_sha256(b"k", &[b"12", b".", b"{}"]), whole);
        assert_eq!(
            signature("k", 12, b"{}"),
            format!("sha256={}", HEXLOWER.encode(&whole))
        );
        // Exactly 64 bytes is not hashed; 65 is. They differ.
        assert_ne!(
            hmac_sha256(&[1; 64], &[b"x"]),
            hmac_sha256(&[1; 65], &[b"x"])
        );
    }
}
