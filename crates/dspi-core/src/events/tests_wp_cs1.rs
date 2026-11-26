//! WP CS1: WP FWX's clip events and reset readout in WP P0M's event log,
//! against `FakeDevice` as wp/FWX firmware.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use dspi_proto::generated::notify as nfy;
use dspi_transport::fake::VregSim;
use dspi_transport::{Capture, FakeDevice, FakeOptions, FakeRemote};

use super::*;
use crate::consts::NOTIFY_INPUT_FORMAT;
use crate::ext::tests_wp_c8::k340_options;

const WAIT: Duration = Duration::from_secs(8);

fn fwx(diag_ext: bool) -> FakeDevice {
    let options = FakeOptions {
        diag_ext,
        bulk_source: true,
        ..k340_options()
    };
    FakeDevice::new(Capture::demo(), options)
}

fn attach(core: &DspiCore, dev: FakeDevice, serial: &str) -> FakeRemote {
    let remote = dev.remote();
    core.attach_probed(Box::new(dev), serial.into(), 0).unwrap();
    core.process_notifications();
    remote
}

/// A fresh directory. `ci/run.sh` points `TMPDIR` at the home volume, not tmpfs.
fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wp-cs1-core").join(format!(
        "{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// A clip event as wp/FWX firmware sends it: `[2, 0xE0, 0, seq, channel,
/// count, 0, 0, mask LE]`.
fn clip(mask: u32) -> Vec<u8> {
    let mut p = vec![
        2,
        0xE0,
        0,
        0,
        mask.trailing_zeros() as u8,
        mask.count_ones() as u8,
        0,
        0,
    ];
    p.extend(mask.to_le_bytes());
    p
}

/// Push `INPUT_FORMAT` and process until it is seen: everything pushed
/// before it has been processed too.
fn settle(core: &DspiCore, remote: &FakeRemote) {
    remote.push(vec![
        2,
        nfy::NOTIFY_EVT_INPUT_FORMAT as u8,
        0,
        0,
        2,
        0,
        0,
        0,
    ]);
    let start = Instant::now();
    while core.process_notifications().flags & NOTIFY_INPUT_FORMAT == 0 {
        assert!(start.elapsed() < WAIT, "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn entries(core: &DspiCore, kind: &str) -> Vec<EventEntry> {
    core.event_log_read(String::new(), 0, EVENT_READ_MAX)
        .entries
        .into_iter()
        .filter(|e| e.kind == kind)
        .collect()
}

#[test]
fn wp_cs1_a_clip_event_reaches_the_log_one_record_per_channel_of_its_mask() {
    let core = DspiCore::new();
    let remote = attach(&core, fwx(true), "CS1");
    let inputs = core.get_state().num_input_channels;
    // One event naming outputs 1 and 4.
    let mask = (1u32 << inputs) | (1u32 << (inputs + 3));
    remote.push(clip(mask));
    settle(&core, &remote);
    let clips = entries(&core, "clip");
    assert_eq!(clips.len(), 2, "{clips:?}");
    assert_eq!(clips[0].channel, Some(inputs));
    assert_eq!(clips[1].channel, Some(inputs + 3));
    assert_eq!(clips[0].text, "Clip on output 1");
    assert_eq!(clips[1].text, "Clip on output 4");
    assert!(clips.iter().all(|c| c.source == "notification"));
    // The meters' clip hold has them too, and FWX's hand-off still works.
    assert_eq!(core.take_clip_mask(), mask);

    // The same channels again while held: no new records.
    remote.push(clip(1u32 << inputs));
    settle(&core, &remote);
    assert_eq!(entries(&core, "clip").len(), 2);
}

#[test]
fn wp_cs1_a_clip_event_from_a_device_without_the_feature_bit_is_not_logged() {
    let core = DspiCore::new();
    let remote = attach(&core, fwx(false), "CS1-OLD");
    let inputs = core.get_state().num_input_channels;
    remote.push(clip(1u32 << inputs));
    settle(&core, &remote);
    assert!(entries(&core, "clip").is_empty());
}

#[test]
fn wp_cs1_a_restart_while_away_carries_the_reset_readouts_reason() {
    let d = dir("reset");
    let core = DspiCore::new();
    core.event_log_set_dir(d.to_string_lossy().into_owned())
        .unwrap();
    // Up an hour, then away.
    let dev = fwx(true);
    dev.remote().vreg_sim(VregSim::Advance(3600));
    attach(&core, dev, "CS1-RESET");
    core.disconnect();
    // The same board, back after its watchdog fired.
    let dev = fwx(true);
    dev.remote().vreg_sim(VregSim::Hang);
    attach(&core, dev, "CS1-RESET");
    let info = core.reset_info_at_connect().unwrap().unwrap();
    assert_eq!(info.reason, "watchdog");
    let resets = entries(&core, "reset");
    assert_eq!(resets.len(), 1, "{resets:?}");
    assert_eq!(resets[0].source, "watchdog");
    assert!(
        resets[0].text.contains(&info.description),
        "{}",
        resets[0].text
    );
}

/// Without the core-voltage extension the readout's uptime is the one
/// compared.
#[test]
fn wp_cs1_the_readouts_uptime_stands_in_for_the_core_voltage_status() {
    let d = dir("uptime");
    let core = DspiCore::new();
    core.event_log_set_dir(d.to_string_lossy().into_owned())
        .unwrap();
    let opened = |uptime: Option<(u32, u8)>, reset: Option<crate::DeviceResetInfo>| Opened {
        serial: "CS1-NOVREG".into(),
        transport: "usb".into(),
        fw: None,
        inputs: 2,
        platform_id: 1,
        uptime,
        reset,
    };
    core.events.connected(opened(Some((3600, 0)), None));
    core.events.disconnected("CS1-NOVREG", "closed");
    let info = crate::DeviceResetInfo {
        reason: "brownout".into(),
        reason_code: 2,
        description: "the supply dipped".into(),
        uptime_s: 5,
        ..Default::default()
    };
    core.events.connected(opened(None, Some(info)));
    let resets = entries(&core, "reset");
    assert_eq!(resets.len(), 1, "{resets:?}");
    assert_eq!(resets[0].source, "brownout");
    assert_eq!(resets[0].uptime_s, Some(5));
    assert_eq!(
        resets[0].text,
        "The device restarted while away (the supply dipped); up 5 s"
    );
}

// -- WP FIN015 -- (REVF D2)
/// The first event of a connection says how it came in words: "over USB",
/// "through a hub", "to the demo", never the transport's id ("over fake");
/// the record's `source` keeps the id.
#[test]
fn wp_fin015_a_connection_says_its_transport_in_words() {
    let d = dir("fin015-over");
    let core = DspiCore::new();
    core.event_log_set_dir(d.to_string_lossy().into_owned())
        .unwrap();
    for (transport, fw, line) in [
        ("usb", Some("1.1.6"), "Connected over USB (firmware 1.1.6)"),
        ("midi", None, "Connected over USB MIDI"),
        (
            "link",
            Some("1.2.0"),
            "Connected through a hub (firmware 1.2.0)",
        ),
        (
            "fake",
            Some("1.1.6"),
            "Connected to the demo (firmware 1.1.6)",
        ),
    ] {
        core.events.connected(Opened {
            serial: "FIN015".into(),
            transport: transport.into(),
            fw: fw.map(Into::into),
            inputs: 2,
            platform_id: 1,
            uptime: None,
            reset: None,
        });
        let last = entries(&core, "connected").pop().unwrap();
        assert_eq!(last.text, line);
        assert_eq!(last.source, transport);
        core.events.disconnected("FIN015", "closed");
    }
}
