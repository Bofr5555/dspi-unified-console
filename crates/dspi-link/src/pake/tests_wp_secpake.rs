//! WP SECPAKE: the SPAKE2 pairing's crypto, without sockets (the hub's tests run
//! it over real TLS legs, `crates/dspi-hub/tests/wp_secpake.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use super::*;

const HID: &str = "a1b2c3d4e5f60718";
const NAME: &str = "Kitchen iPad";
const ROLE: &str = "control";
const PIN: &str = "482913";

fn binding(byte: u8) -> Binding {
    Binding::from_bytes([byte; BINDING_LEN])
}

/// One whole exchange; the hub's step 2 result and the client's check of it.
fn exchange(client_pin: &str, hub_pin: &str, client_leg: Binding, hub_leg: Binding) -> Result<(), PakeError> {
    let (client, msg_a) = ClientPake::start(client_pin, HID, NAME, ROLE, client_leg)?;
    let (hub, msg_b) = HubPake::respond(&pin_secret(hub_pin), HID, NAME, ROLE, &msg_a, hub_leg)?;
    let confirm = client.finish(&msg_b)?;
    let hub_mac = hub.confirm(confirm.confirm())?;
    confirm.verify_hub(&hub_mac)
}

#[test]
fn wp_secpake_a_right_pin_confirms_both_ways() {
    exchange(PIN, PIN, binding(7), binding(7)).unwrap();
}

#[test]
fn wp_secpake_a_wrong_pin_is_a_mismatch_at_the_hub() {
    assert_eq!(exchange("482914", PIN, binding(7), binding(7)), Err(PakeError::Mismatch));
}

#[test]
fn wp_secpake_two_tls_legs_fail_even_with_the_right_pin() {
    // A proxy relaying both SPAKE2 messages between two TLS connections: the
    // exporters differ, so the client's MAC is refused at the hub.
    assert_eq!(exchange(PIN, PIN, binding(1), binding(2)), Err(PakeError::Mismatch));
}

#[test]
fn wp_secpake_the_transcript_binds_name_role_and_hid() {
    let (client, msg_a) = ClientPake::start(PIN, HID, NAME, ROLE, binding(7)).unwrap();
    // The hub heard another role (a proxy rewrote it): no agreement.
    let (hub, msg_b) = HubPake::respond(&pin_secret(PIN), HID, NAME, "admin", &msg_a, binding(7)).unwrap();
    let confirm = client.finish(&msg_b).unwrap();
    assert_eq!(hub.confirm(confirm.confirm()), Err(PakeError::Mismatch));
    for (hid, name) in [("ffffffffffffffff", NAME), (HID, "Someone else")] {
        let (client, msg_a) = ClientPake::start(PIN, HID, NAME, ROLE, binding(7)).unwrap();
        let (hub, msg_b) = HubPake::respond(&pin_secret(PIN), hid, name, ROLE, &msg_a, binding(7)).unwrap();
        let confirm = client.finish(&msg_b).unwrap();
        assert_eq!(hub.confirm(confirm.confirm()), Err(PakeError::Mismatch));
    }
}

#[test]
fn wp_secpake_the_hubs_mac_is_checked_by_the_client() {
    let (client, msg_a) = ClientPake::start(PIN, HID, NAME, ROLE, binding(7)).unwrap();
    let (_hub, msg_b) = HubPake::respond(&pin_secret(PIN), HID, NAME, ROLE, &msg_a, binding(7)).unwrap();
    let confirm = client.finish(&msg_b).unwrap();
    // The client's own MAC sent back, a zero MAC, a short one: all refused.
    let own = confirm.confirm().to_vec();
    assert_eq!(confirm.verify_hub(&own), Err(PakeError::Mismatch));
    assert_eq!(confirm.verify_hub(&[0u8; CONFIRM_LEN]), Err(PakeError::Mismatch));
    assert_eq!(confirm.verify_hub(&[0u8; 3]), Err(PakeError::Mismatch));
}

/// The done criterion's offline half: a machine in the middle that plays the hub
/// to a pairing client, sends its own SPAKE2 message for a guess `g`, and keeps
/// the client's confirmation. It then searches PINs offline, recomputing its side
/// for each candidate with the very same randomness (the most the protocol lets
/// it vary). Its message commits it to `g`: the client's key is
/// `x·(yG + N·(g - pin))`, and the `x·N` term is out of its reach unless
/// `g == pin`, so no other candidate verifies. The control shows the search is a
/// real one: with `g == pin` it finds the PIN.
fn offline_search(guess: &str, candidates: &[String]) -> Vec<String> {
    let leg = binding(9); // the attacker's own TLS leg to the client: it knows it
    let (client, msg_a) = ClientPake::start(PIN, HID, NAME, ROLE, leg).unwrap();
    let seed = [42u8; 32];
    let (_, msg_b) =
        HubPake::respond_with(&pin_secret(guess), HID, NAME, ROLE, &msg_a, leg, &mut Drbg::from_seed(seed)).unwrap();
    let captured = client.finish(&msg_b).unwrap().confirm().to_vec();
    candidates
        .iter()
        .filter(|c| {
            let (side, _) =
                HubPake::respond_with(&pin_secret(c), HID, NAME, ROLE, &msg_a, leg, &mut Drbg::from_seed(seed))
                    .unwrap();
            side.confirm(&captured).is_ok()
        })
        .cloned()
        .collect()
}

fn candidates() -> Vec<String> {
    let mut all: Vec<String> = (0..400).map(|n| format!("{n:06}")).collect();
    all.push(PIN.to_owned());
    all.push("000000".to_owned());
    all
}

#[test]
fn wp_secpake_a_mitm_cannot_search_the_pin_offline() {
    assert!(offline_search("111111", &candidates()).is_empty());
}

#[test]
fn wp_secpake_the_offline_search_finds_a_right_guess_control() {
    assert_eq!(offline_search(PIN, &candidates()), vec![PIN.to_owned()]);
}

#[test]
fn wp_secpake_malformed_messages_are_errors_never_panics() {
    let secret = pin_secret(PIN);
    for len in [0usize, 1, 32, 34, 64] {
        let msg = vec![0x41u8; len];
        assert_eq!(HubPake::respond(&secret, HID, NAME, ROLE, &msg, binding(1)).err(), Some(PakeError::Malformed));
    }
    // The wrong side's byte (a client's message replayed to a client).
    let (client, msg_a) = ClientPake::start(PIN, HID, NAME, ROLE, binding(1)).unwrap();
    assert_eq!(client.finish(&msg_a).err(), Some(PakeError::Malformed));
    // Arbitrary bytes: an error or an answer, whatever they decode to.
    let mut rng = Drbg::from_seed([3u8; 32]);
    for _ in 0..300 {
        let mut msg = [0u8; MESSAGE_LEN];
        rng.try_fill_bytes(&mut msg).unwrap();
        msg[0] = 0x41;
        let _ = HubPake::respond(&secret, HID, NAME, ROLE, &msg, binding(1));
    }
    assert_eq!(decode("!!!!", MESSAGE_LEN), Err(PakeError::Malformed));
    assert_eq!(decode(&"A".repeat(200), MESSAGE_LEN), Err(PakeError::Malformed));
    assert_eq!(decode(&encode(&[1u8; 32]), MESSAGE_LEN), Err(PakeError::Malformed));
    assert_eq!(decode(&encode(&[1u8; 33]), MESSAGE_LEN).unwrap(), vec![1u8; 33]);
}

#[test]
fn wp_secpake_the_password_is_what_the_hub_keeps_of_a_pin() {
    // SHA-256("123456"), as dspi-hub's `auth.rs` hashes a window's PIN.
    let expected = "8d969eef6ecad3c29a3a629280e686cf0c3f5d5a86aff3ca12020c923adc6c92";
    assert_eq!(data_encoding::HEXLOWER.encode(&pin_secret("123456")), expected);
}

#[test]
fn wp_secpake_debug_shows_no_key_material() {
    let (client, msg_a) = ClientPake::start(PIN, HID, NAME, ROLE, binding(5)).unwrap();
    let (hub, msg_b) = HubPake::respond(&pin_secret(PIN), HID, NAME, ROLE, &msg_a, binding(5)).unwrap();
    let client_debug = format!("{client:?}");
    let confirm = client.finish(&msg_b).unwrap();
    let all = format!("{client_debug} {confirm:?} {hub:?} {:?}", binding(5));
    assert!(!all.contains(PIN), "{all}");
    assert!(all.contains("redacted"), "{all}");
    let mac_hex = data_encoding::HEXLOWER.encode(confirm.confirm());
    assert!(!all.contains(&mac_hex) && !all.contains(&encode(confirm.confirm())));
    assert!(!all.contains("05, 05") && !all.contains("5, 5, 5"));
}
