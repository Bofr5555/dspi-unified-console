//! WP PGTM: `uconsole info` and `uconsole raw --id` on every fake generation
//! (G1s, G1b with and without PAGEGUARD, G2s, G2b) and the DSPi-Zero, and a
//! paged device on the hub's in-process path (the empty page list).

use std::sync::Arc;
use std::time::{Duration, Instant};

use dspi_core::{DspiCore, DspiError, ProtocolGeneration};
use dspi_proto::paged::cmds;
use dspi_transport::fake::{Capture, FakeBoard, FakeGeneration, FakeOptions};
use dspi_transport::{FakeDevice, FakeRemote, Transport, paged};

use super::*;

fn connect_dev(dev: FakeDevice) -> (Arc<DspiCore>, FakeRemote) {
    let remote = dev.remote();
    let serial = dev.descriptor().serial.clone();
    let t = paged::open(Box::new(dev)).expect("the probe");
    let core = DspiCore::new();
    core.open_transport(t, serial, 0).expect("the connect");
    (core, remote)
}

fn connect(g: FakeGeneration) -> (Arc<DspiCore>, FakeRemote) {
    connect_dev(FakeDevice::demo_generation(g))
}

fn row<'a>(rows: &'a [(&'static str, String)], label: &str) -> &'a str {
    rows.iter()
        .find(|(l, _)| *l == label)
        .map(|(_, v)| v.as_str())
        .unwrap_or_else(|| panic!("no {label} row in {rows:?}"))
}

#[test]
fn wp_pgtm_info_shows_the_generation_of_each_fake() {
    for g in FakeGeneration::ALL {
        let (core, _remote) = connect(g);
        let p = core.device_protocol().unwrap();
        let view = view_of(&p);
        let rows = view.rows();
        assert_eq!(row(&rows, "Board"), "DSPi (board 0)", "{g:?}");
        if g.is_paged() {
            assert_eq!(p.generation, ProtocolGeneration::Paged, "{g:?}");
            assert_eq!(row(&rows, "Generation"), "paged: firmware 1.1.7 and later");
            assert_eq!(
                row(&rows, "Protocol"),
                format!("paged {}.{}", p.protocol_major, p.protocol_minor)
            );
            assert!(p.protocol_major >= 1, "{g:?}");
            let pages = row(&rows, "Pages");
            assert!(pages.contains("0x01 SYS r"), "{g:?}: {pages}");
            assert_eq!(pages.contains("0xE0 EXT r"), g.has_extensions(), "{g:?}");
            let ext = if g.has_extensions() {
                "tunnel 0xE010 on page 0xE0"
            } else {
                "none (no extension channel)"
            };
            assert_eq!(row(&rows, "Extensions"), ext, "{g:?}");
        } else {
            assert_eq!(p.generation, ProtocolGeneration::Legacy, "{g:?}");
            assert_eq!(row(&rows, "Generation"), "legacy: firmware 1.1.6 and older");
            assert_eq!(row(&rows, "Protocol"), "1.1.6 opcodes");
            assert_eq!(row(&rows, "Pages"), "none: page 0 only");
            let ext = if g.has_extensions() {
                "REQ_EXTENSION 0x00 on page 0"
            } else {
                "none (no extension channel)"
            };
            assert_eq!(row(&rows, "Extensions"), ext, "{g:?}");
        }
        assert!(view.notes().is_empty(), "{g:?}: the page list is there");
        let text = info_text("SER-1", &view);
        assert!(text.starts_with("Serial      SER-1\n"), "{text}");
        assert!(text.contains("Generation  "), "{text}");
        let j = info_json("SER-1", &p);
        assert_eq!(
            j["protocol"]["generation"],
            if g.is_paged() { "paged" } else { "legacy" }
        );
        assert_eq!(j["protocol"]["boardName"], "DSPi");
        assert_eq!(j["notes"].as_array().unwrap().len(), 0);
    }
}

#[test]
fn wp_pgtm_info_names_the_zero_on_either_generation() {
    for g in [FakeGeneration::Stock116, FakeGeneration::STOCK117] {
        let dev = FakeDevice::new(
            Capture::demo(),
            FakeOptions {
                board: FakeBoard::Zero,
                generation: Some(g),
                ..FakeOptions::default()
            },
        );
        let (core, _remote) = connect_dev(dev);
        let view = read_view(&core).unwrap();
        assert_eq!(row(&view.rows(), "Board"), "DSPi Zero (board 1)", "{g:?}");
        assert_eq!(
            view.generation == GenerationView::Paged,
            g.is_paged(),
            "{g:?}"
        );
    }
    // Nothing open: no rows for About
    assert_eq!(read_view(&DspiCore::new()), None);
}

/// The hub's in-process path (`LocalTransport`, the app's own): the core
/// says paged, the page list is empty, and the note says why (PG-C review
/// carry-over).
#[test]
fn wp_pgtm_a_paged_device_on_the_hubs_local_path_has_no_page_list() {
    let hub = dspi_hub::Hub::start(dspi_hub::HubConfig::in_process("pgtm")).unwrap();
    let dev = FakeDevice::demo_generation(FakeGeneration::STOCK117).with_serial("PGTM-LOCAL");
    hub.attach(Box::new(dev), 0x2E8B).unwrap();
    let start = Instant::now();
    let local = loop {
        match hub.open_local("PGTM-LOCAL") {
            Ok(l) => break l,
            Err(e) => {
                assert!(
                    start.elapsed() < Duration::from_secs(10),
                    "never there: {e:?}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let core = DspiCore::new();
    core.open_transport(Box::new(local), "PGTM-LOCAL".into(), 0)
        .expect("the connect through the hub");
    let p = core.device_protocol().unwrap();
    assert_eq!(p.generation, ProtocolGeneration::Paged);
    assert!(p.pages.is_empty(), "{:?}", p.pages);
    let view = view_of(&p);
    assert_eq!(row(&view.rows(), "Pages"), "none listed on this connection");
    assert_eq!(
        view.notes(),
        vec![dspi_tui::settings::about::NO_PAGE_LIST_NOTE]
    );
    let text = info_text("PGTM-LOCAL", &view);
    assert!(
        text.contains("\nNote: This connection says the device is paged"),
        "{text}"
    );
    assert_eq!(
        info_json("PGTM-LOCAL", &p)["notes"][0],
        dspi_tui::settings::about::NO_PAGE_LIST_NOTE
    );
    core.disconnect();
}

#[test]
fn wp_pgtm_raw_id_parses_numbers_and_names() {
    let p = |args: &[&str]| parse_raw_id(args);
    assert_eq!(
        p(&["raw", "--id", "0x0550", "4"]),
        Ok(RawId::Get {
            id: 0x0550,
            len: 4,
            value: 0
        })
    );
    assert_eq!(
        p(&["raw", "--id=ROUTE.MASTER_VOLUME", "4", "2", "--json"]),
        Ok(RawId::Get {
            id: cmds::ROUTE_MASTER_VOLUME.0,
            len: 4,
            value: 2
        })
    );
    assert_eq!(
        p(&["raw", "out", "--id", "0x0550", "0", "00", "00", "c0", "c0"]),
        Ok(RawId::Set {
            id: 0x0550,
            value: 0,
            data: vec![0, 0, 0xC0, 0xC0]
        })
    );
    for bad in [
        &["raw", "--id"][..],
        &["raw", "--id", "0x10000", "4"],
        &["raw", "--id", "NO.SUCH", "4"],
        &["raw", "--id", "0x0550"],
        &["raw", "--id", "0x0550", "70000"],
        &["raw", "out", "--id", "0x0550"],
        &["raw", "out", "--id", "0x0550", "0", "zz"],
        &["raw", "war", "--id", "0x0550", "0"],
    ] {
        assert!(p(bad).is_err(), "{bad:?}");
    }
    assert!(wants_id(&["raw", "--id", "1"]));
    assert!(wants_id(&["raw", "--id=1"]));
    assert!(!wants_id(&["raw", "0x87", "32"]));
}

#[test]
fn wp_pgtm_raw_id_reads_and_writes_on_paged_fakes() {
    for g in [FakeGeneration::STOCK117, FakeGeneration::Bofr117] {
        let (core, _remote) = connect(g);
        let id = cmds::ROUTE_MASTER_VOLUME.0;
        let db = (-6.0f32).to_le_bytes().to_vec();
        let set = RawId::Set {
            id,
            value: 0,
            data: db.clone(),
        };
        assert_eq!(run_raw_id(&core, &set), Ok(None), "{g:?}");
        let get = RawId::Get {
            id,
            len: 4,
            value: 0,
        };
        assert_eq!(run_raw_id(&core, &get), Ok(Some(db)), "{g:?}");

        // The core's refusals, in words, with nothing sent
        let flash = RawId::Set {
            id: cmds::STORE_PRESET_SAVE.0,
            value: 1,
            data: Vec::new(),
        };
        let e = run_raw_id(&core, &flash).unwrap_err();
        let words = refusal_words(cmds::STORE_PRESET_SAVE.0, &e).unwrap();
        assert!(words.contains("STORE.PRESET_SAVE was not sent"), "{words}");
        assert!(words.contains("needs a confirmation token"), "{words}");
        assert_eq!(crate::pro::error_exit(&e), exit::USAGE);

        let page0 = RawId::Get {
            id: 0x0042,
            len: 4,
            value: 0,
        };
        let e = run_raw_id(&core, &page0).unwrap_err();
        let words = refusal_words(0x0042, &e).unwrap();
        assert!(words.contains("is on page 0"), "{words}");
    }
}

#[test]
fn wp_pgtm_raw_id_on_a_legacy_fake_is_refused_in_words() {
    for g in [
        FakeGeneration::Stock116,
        FakeGeneration::Bofr116 { page_guard: false },
        FakeGeneration::Bofr116 { page_guard: true },
    ] {
        let (core, remote) = connect(g);
        let before = remote.paged_requests();
        let id = cmds::ROUTE_MASTER_VOLUME.0;
        let e = run_raw_id(
            &core,
            &RawId::Get {
                id,
                len: 4,
                value: 0,
            },
        )
        .unwrap_err();
        assert_eq!(e, DspiError::Unsupported, "{g:?}");
        let words = refusal_words(id, &e).unwrap();
        assert!(
            words.contains("ROUTE.MASTER_VOLUME was not sent"),
            "{words}"
        );
        assert!(
            words.contains("1.1.6 protocol, which has no pages"),
            "{words}"
        );
        assert_eq!(remote.paged_requests(), before, "{g:?}: nothing sent");
    }
}

#[test]
fn wp_pgtm_a_firmware_refusal_is_named_by_its_result_code() {
    let busy = dspi_proto::paged::legacy::RESULTS
        .iter()
        .find(|(n, _)| *n == "VC_ERR_BUSY")
        .map(|(_, c)| *c)
        .unwrap();
    let words = refusal_words(0x0550, &DspiError::Refused { status: busy }).unwrap();
    assert!(words.starts_with("the device refused 0x0550"), "{words}");
    assert!(words.ends_with(&format!("VC_ERR_BUSY ({busy})")), "{words}");
    let words = refusal_words(0x0550, &DspiError::Refused { status: 0 }).unwrap();
    assert!(words.ends_with("it gave no result code"), "{words}");
    let words = refusal_words(0x0550, &DspiError::Refused { status: 250 }).unwrap();
    assert!(words.ends_with("result code 250"), "{words}");
    // The general path says the rest (the lock, the read-only mode, the link)
    assert_eq!(refusal_words(0x0550, &DspiError::ReadOnly), None);
}
