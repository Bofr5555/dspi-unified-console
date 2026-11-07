//! WP CG23: the raw notification tap and the connection generation, on the
//! fake device and the fake hub.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dspi_proto::generated::{self as g, notify as nfy};
use dspi_transport::FakeDevice;

use super::*;

const WAIT: Duration = Duration::from_secs(8);
const GPIO: u8 = nfy::PARAM_SRC_GPIO as u8;

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Process notifications until the tap has held `n` entries since this call.
fn tapped(core: &DspiCore, n: usize) -> Vec<RawNotification> {
    let mut got = Vec::new();
    wait_until(&format!("{n} tapped notifications"), || {
        core.process_notifications();
        got.extend(core.notify_tap_drain());
        got.len() >= n
    });
    got
}

/// A core on the demo fake device with the tap on, and the device's remote.
fn tapped_fake() -> (Arc<DspiCore>, dspi_transport::FakeRemote) {
    let dev = FakeDevice::demo();
    let remote = dev.remote();
    let core = DspiCore::new();
    core.attach_probed(Box::new(dev), "FAKE".into(), 0).unwrap();
    core.notify_tap_set(true);
    (core, remote)
}

#[test]
fn wp_cg23_our_echo_is_own_and_a_knob_is_not() {
    let (core, remote) = tapped_fake();
    core.raw_set(
        op::REQ_SET_MASTER_VOLUME,
        0,
        (-17.0f32).to_le_bytes().to_vec(),
    )
    .unwrap();
    // The firmware sends the v1 volume packet and its v2 PARAM_CHANGED.
    let ours = tapped(&core, 2);
    let changed: Vec<_> = ours
        .iter()
        .filter(|n| n.packet.get(1) == Some(&evt::PARAM_CHANGED))
        .collect();
    assert_eq!(changed.len(), 1, "{ours:?}");
    assert!(changed[0].own, "our own write's echo: {ours:?}");
    assert_eq!(changed[0].origin, None, "USB carries no origin");
    assert_eq!(
        u16::from_le_bytes([changed[0].packet[4], changed[0].packet[5]]),
        g::OFF_MASTER_VOLUME as u16
    );

    assert!(remote.poke(g::OFF_MASTER_VOLUME, &(-9.0f32).to_le_bytes(), GPIO));
    let knob = tapped(&core, 1);
    assert_eq!(knob.len(), 1, "{knob:?}");
    assert!(!knob[0].own, "a control surface is not us");
    assert_eq!(knob[0].packet[8], GPIO);
    assert!(!knob[0].lost);
    let now = clock::system_time()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    assert!((now - knob[0].received_ms).abs() < 5_000, "{knob:?}");
    // The mirror saw the same packet.
    assert_eq!(core.get_state().master_volume_db, -9.0);
}

#[test]
fn wp_cg23_packets_are_the_firmwares_byte_for_byte() {
    let (core, remote) = tapped_fake();
    let v2 = |id: u16, body: &[u8]| {
        let mut p = vec![PROTOCOL_VERSION, id as u8, 0, 0];
        p.extend_from_slice(body);
        p
    };
    let mut param = v2(nfy::NOTIFY_EVT_PARAM_CHANGED, &[0, 0, 4, 0, GPIO, 0, 0, 0]);
    param[4..6].copy_from_slice(&(g::OFF_MASTER_VOLUME as u16).to_le_bytes());
    param.extend_from_slice(&(-3.0f32).to_le_bytes());
    let mut volume_v1 = vec![evt::MASTER_VOLUME, 0, 0, 0];
    volume_v1.extend_from_slice(&(-3.0f32).to_le_bytes());
    let sent = [
        param,
        volume_v1,
        v2(nfy::NOTIFY_EVT_BULK_INVALIDATED, &[GPIO, 0, 0, 0]),
        v2(nfy::NOTIFY_EVT_PRESET_LOADED, &[3, 0, 0, 0]),
        v2(nfy::NOTIFY_EVT_INPUT_FORMAT, &[4, 0, 0, 0]),
        v2(nfy::NOTIFY_EVT_SIGGEN_STATE, &[1, 2, 3, 0xFF]),
        v2(nfy::NOTIFY_EVT_ADAT_STATE, &[1, 0, 26, 0]),
        v2(nfy::NOTIFY_EVT_I2S_SLAVE_STATE, &[2, 0x80, 0xBB, 0, 0]),
        v2(
            nfy::NOTIFY_EVT_CS_IR_LEARN,
            &[1, 2, 0, 0, 0x78, 0x56, 0x34, 0x12],
        ),
        v2(nfy::NOTIFY_EVT_ADAT_INPUT_STATE, &[3, 0x44, 0xAC, 0, 0, 1]),
        v2(nfy::NOTIFY_EVT_CS_AUX, &[2, 1, 0x00, 0x32, GPIO]),
        v2(
            nfy::NOTIFY_EVT_DEVICE_NAME,
            &[0, GPIO, 0, 0, b'K', b'3', 0, 0],
        ),
        // An event this build does not know.
        v2(0x3E, &[9, 8, 7]),
    ];
    for p in &sent {
        remote.push(p.clone());
    }
    let got = tapped(&core, sent.len());
    assert_eq!(got.len(), sent.len());
    for (p, n) in sent.iter().zip(&got) {
        if p[0] == PROTOCOL_VERSION {
            // The device numbered it: byte 3 is its sequence number.
            assert_eq!(n.packet.len(), p.len(), "{p:02X?} -> {:02X?}", n.packet);
            assert_eq!(n.packet[..3], p[..3], "{p:02X?} -> {:02X?}", n.packet);
            assert_eq!(n.packet[4..], p[4..], "{p:02X?} -> {:02X?}", n.packet);
        } else {
            assert_eq!(&n.packet, p);
        }
        assert!(!n.own);
    }
    let seqs: Vec<u8> = got
        .iter()
        .filter(|n| n.packet[0] == PROTOCOL_VERSION)
        .map(|n| n.packet[3])
        .collect();
    assert!(
        seqs.windows(2).all(|w| w[1] == w[0].wrapping_add(1)),
        "{seqs:?}"
    );
}

#[test]
fn wp_cg23_an_undecodable_packet_is_kept_whole() {
    let (core, remote) = tapped_fake();
    let odd = vec![7, 1, 2, 3, 4];
    remote.push(odd.clone());
    let got = tapped(&core, 1);
    assert_eq!(got[0].packet, odd);
    assert!(got[0].lost, "the reader counts it as a loss");
}

#[test]
fn wp_cg23_off_by_default_and_off_forgets() {
    let (core, remote) = tapped_fake();
    core.notify_tap_set(false);
    let fresh = DspiCore::new();
    assert!(fresh.notify_tap_drain().is_empty());

    assert!(remote.poke(g::OFF_MASTER_VOLUME, &(-8.0f32).to_le_bytes(), GPIO));
    wait_until("the mirror", || {
        core.process_notifications();
        core.get_state().master_volume_db == -8.0
    });
    assert!(core.notify_tap_drain().is_empty(), "off keeps nothing");

    core.notify_tap_set(true);
    assert!(remote.poke(g::OFF_MASTER_VOLUME, &(-7.0f32).to_le_bytes(), GPIO));
    wait_until("the mirror", || {
        core.process_notifications();
        core.get_state().master_volume_db == -7.0
    });
    core.notify_tap_set(false);
    core.notify_tap_set(true);
    assert!(core.notify_tap_drain().is_empty(), "off forgot it");
}

#[test]
fn wp_cg23_overflow_sets_lost() {
    let (core, remote) = tapped_fake();
    let total = NotifyTap::LIMIT + 88;
    for i in 0..total {
        remote.push(vec![
            PROTOCOL_VERSION,
            nfy::NOTIFY_EVT_SIGGEN_STATE as u8,
            0,
            0,
            0,
            0,
            0,
            (i % 250) as u8,
        ]);
    }
    // Drain the device without draining the tap.
    let mut seen = 0;
    wait_until("every packet processed", || {
        core.process_notifications();
        seen = core.monitor_read(0).entries.len();
        seen >= total
    });
    assert_eq!(seen, total);
    let got = core.notify_tap_drain();
    assert_eq!(got.len(), NotifyTap::LIMIT);
    assert!(got[0].lost, "the oldest kept says older ones are gone");
    assert!(got[1..].iter().all(|n| !n.lost));
    assert_eq!(got.last().unwrap().packet[7], ((total - 1) % 250) as u8);
    assert_eq!(got[0].packet[7], ((total - NotifyTap::LIMIT) % 250) as u8);
    assert!(core.notify_tap_drain().is_empty(), "a drain empties it");
}

#[test]
fn wp_cg23_a_binding_that_dropped_a_drain_marks_the_next_entry() {
    let (core, remote) = tapped_fake();
    core.notify_tap_note_dropped();
    assert!(remote.poke(g::OFF_MASTER_VOLUME, &(-6.0f32).to_le_bytes(), GPIO));
    assert!(remote.poke(g::OFF_MASTER_VOLUME, &(-5.0f32).to_le_bytes(), GPIO));
    let got = tapped(&core, 2);
    assert_eq!(
        got.iter().map(|n| n.lost).collect::<Vec<_>>(),
        [true, false]
    );
}

#[test]
fn wp_cg23_the_generation_changes_on_reopen_and_never_repeats() {
    let core = DspiCore::new();
    assert_eq!(core.connection_generation(), 0, "nothing open");
    let mut seen = BTreeSet::new();
    for round in 0..3 {
        core.open_demo().unwrap();
        let g = core.connection_generation();
        assert_ne!(g, 0);
        assert!(seen.insert(g), "round {round}: {g} again");
        core.disconnect();
        assert_eq!(core.connection_generation(), 0);

        core.attach_probed(Box::new(FakeDevice::demo()), "FAKE".into(), 0)
            .unwrap();
        let g = core.connection_generation();
        assert!(seen.insert(g), "round {round}: {g} again");
        // Opening again over an open connection replaces it.
        core.open_demo().unwrap();
        let g = core.connection_generation();
        assert!(seen.insert(g), "round {round}: {g} again");
        core.disconnect();
    }
    // Another core never hands out one of these either.
    let other = DspiCore::new();
    other.open_demo().unwrap();
    assert!(!seen.contains(&other.connection_generation()));
}

#[cfg(feature = "link")]
mod link {
    use dspi_link::fakehub::{FakeHub, HubConfig};

    use super::*;
    use crate::link::Tuning;

    fn bound(hub: &FakeHub) -> Arc<DspiCore> {
        let core = DspiCore::new();
        core.link_connect_with(
            hub.url(),
            "wp-cg23".into(),
            "test".into(),
            String::new(),
            String::new(),
            Tuning {
                backoff_divisor: 50,
                ..Tuning::default()
            },
        )
        .unwrap();
        wait_until("the device list", || !core.link_devices().is_empty());
        let d = core.link_devices().remove(0);
        core.link_select_device(d.handle, d.serial).unwrap();
        core.fetch_all().unwrap();
        core
    }

    #[test]
    fn wp_cg23_another_sessions_change_carries_its_origin() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let core = bound(&hub);
        core.notify_tap_set(true);
        let ours = hub.sessions();
        assert_eq!(ours.len(), 1, "{ours:?}");
        let other = ours[0].wrapping_add(7);

        hub.external_change(
            g::OFF_MASTER_VOLUME as u16,
            &(-11.0f32).to_le_bytes(),
            GPIO,
            other,
        );
        let got = tapped(&core, 1);
        assert_eq!(got[0].origin, Some(other), "{got:?}");
        assert!(!got[0].own);
        assert_eq!(got[0].packet[1], evt::PARAM_CHANGED);
        assert_eq!(got[0].packet[8], GPIO);

        // Our own write comes back with our session as its origin.
        core.raw_set(
            op::REQ_SET_MASTER_VOLUME,
            0,
            (-12.0f32).to_le_bytes().to_vec(),
        )
        .unwrap();
        let echo = tapped(&core, 1);
        let changed: Vec<_> = echo
            .iter()
            .filter(|n| n.packet.get(1) == Some(&evt::PARAM_CHANGED))
            .collect();
        assert!(!changed.is_empty(), "{echo:?}");
        assert!(
            changed.iter().all(|n| n.own && n.origin == Some(ours[0])),
            "{echo:?}"
        );
    }

    #[test]
    fn wp_cg23_the_generation_changes_on_a_link_rebind() {
        let hub = FakeHub::start(HubConfig::default()).unwrap();
        let core = bound(&hub);
        let mut seen = BTreeSet::new();
        let first = core.connection_generation();
        assert_ne!(first, 0);
        seen.insert(first);
        for _ in 0..2 {
            let before = core.connection_generation();
            hub.close_all(1001);
            wait_until("the rebind", || {
                core.link_status().phase == crate::LinkPhase::Bound
                    && core.connection_generation() != before
                    && core.connection_generation() != 0
            });
            let g = core.connection_generation();
            assert!(seen.insert(g), "{g} again");
        }
    }
}
