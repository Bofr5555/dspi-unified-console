//! WP P1OF tests: offline editing and templates.
#![allow(clippy::arithmetic_side_effects)]

use std::sync::Arc;

use dspi_proto::FilterType;
use dspi_transport::FakeOptions;
use serde_json::Value;

use super::*;
use crate::records::FilterParams;

const MAC_DOC: &str = include_str!("../../../../tests/fixtures/preset/mac-swift-v32.dspipreset");

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn opened(text: &str, file: &str) -> (Arc<DspiCore>, Value) {
    let core = DspiCore::new();
    let reply = core.open_document(text.into(), file.into()).unwrap();
    (core, parse(&reply))
}

fn peaking(freq: f32, gain: f32) -> FilterParams {
    FilterParams {
        filter_type: FilterType::Peaking.to_raw(),
        bypass: false,
        freq,
        q: 1.0,
        gain,
        qp: 0.707,
    }
}

/// A folder for files under the build's scratch (`TMPDIR` is
/// `~/.cache/dspi-build/work/<branch>/tmp` in `ci/run.sh`'s container), never
/// on tmpfs: `None` (the test is skipped) where `temp_dir` is `/tmp` or
/// `/dev/shm`.
fn scratch(name: &str) -> Option<std::path::PathBuf> {
    let base = std::env::temp_dir();
    if base.starts_with("/tmp") || base.starts_with("/dev/shm") {
        eprintln!("skipped: temp_dir {} is tmpfs", base.display());
        return None;
    }
    let dir = base.join(format!("wp-p1of-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Some(dir)
}

// ── Opening, editing, saving ──────────────────────────────────────────

#[test]
fn wp_p1of_mac_fixture_edit_eq_save_diff_shows_exactly_the_edit() {
    let (core, reply) = opened(MAC_DOC, "Mac reference.dspipreset");
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["name"], "Mac reference");
    assert_eq!(reply["platform"], "RP2350");
    assert_eq!(reply["inputs"], 8);
    assert_eq!(reply["outputs"], 9);
    assert_eq!(reply["kind"], "preset");
    // Nothing is unsaved straight after opening.
    assert!(
        core.preset_changes().is_empty(),
        "{:?}",
        core.preset_changes()
    );

    // Unedited, the saved document says what the original does.
    let unedited = core.save_document("test".into(), 0).unwrap();
    let d = parse(&files::config_diff(MAC_DOC.into(), unedited));
    assert_eq!(d["ok"], true);
    assert_eq!(d["count"], 0, "{d:#}");

    // One EQ band on output 2 (wire channel 9), band 4.
    core.set_filter(9, 4, peaking(2500.0, -3.5)).unwrap();
    assert!(!core.preset_changes().is_empty());
    let saved = core.save_document("test".into(), 0).unwrap();
    let d = parse(&files::config_diff(MAC_DOC.into(), saved.clone()));
    assert_eq!(d["ok"], true);
    assert_eq!(d["count"], 1, "{d:#}");
    let row = &d["rows"][0];
    assert_eq!(row["block"], "eq");
    assert!(
        row["after"].as_str().unwrap().contains("2500"),
        "the row is the edit: {row}"
    );
    assert_eq!(d["blocks"]["eq"], 1);
    // Saving makes the edit the baseline; the saved text is a preset again.
    assert!(core.preset_changes().is_empty());
    assert_eq!(
        parse(&files::config_inspect(saved.clone()))["kind"],
        "preset"
    );
    assert_eq!(parse(&saved)["meta"]["name"], "Mac reference");
}

#[test]
fn wp_p1of_a_session_on_a_document_says_so_and_touches_no_usb() {
    let core = DspiCore::new();
    assert_eq!(parse(&core.offline_document())["offline"], false);
    core.open_document(MAC_DOC.into(), "/home/u/Desk/hall.dspipreset".into())
        .unwrap();
    let info = parse(&core.offline_document());
    assert_eq!(info["offline"], true);
    assert_eq!(info["fileName"], "/home/u/Desk/hall.dspipreset");
    assert_eq!(info["name"], "Mac reference");
    assert_eq!(info["kind"], "preset");
    assert_eq!(info["platform"], "RP2350");
    {
        let inner = core.lock();
        let conn = inner.conn.as_ref().unwrap();
        assert_eq!(conn.serial, OFFLINE_SERIAL);
        assert_eq!(conn.vid, 0, "not a USB device");
        assert_eq!(inner.selected, None, "nothing to reconnect to on USB");
    }
    // The running preset and the device carry the document's name.
    assert_eq!(core.cached_device_name().unwrap(), "Mac reference");
    // Another device replaces it, and the mark goes with the connection.
    core.open_demo().unwrap();
    assert_eq!(parse(&core.offline_document())["offline"], false);
    assert!(matches!(
        core.save_document("t".into(), 0),
        Err(DspiError::InvalidArgument { .. })
    ));
}

#[test]
fn wp_p1of_a_document_without_a_name_is_named_after_its_file() {
    let mut doc = parse(MAC_DOC);
    doc["meta"].as_object_mut().unwrap().remove("name");
    let (core, reply) = opened(&doc.to_string(), "C:\\Jobs\\Club.v2.dspipreset");
    assert_eq!(reply["name"], "Club.v2");
    assert_eq!(parse(&core.offline_document())["name"], "Club.v2");
}

#[test]
fn wp_p1of_undo_works_on_an_offline_session() {
    let (core, _) = opened(MAC_DOC, "m.dspipreset");
    assert!(!core.can_undo(), "opening is not an undoable change");
    let before = core.get_state().filters[9][4];
    core.set_filter(9, 4, peaking(800.0, 4.0)).unwrap();
    assert_eq!(core.get_state().filters[9][4].freq, 800.0);
    assert!(core.can_undo());
    let moved = core.undo().unwrap();
    assert!(moved.entry.is_some());
    assert_eq!(core.get_state().filters[9][4], before);
    core.redo().unwrap();
    assert_eq!(core.get_state().filters[9][4].freq, 800.0);
}

#[test]
fn wp_p1of_a_document_for_another_platform_opens_that_platforms_demo() {
    let mut doc = parse(MAC_DOC);
    let meta = doc["meta"].as_object_mut().unwrap();
    meta.insert("platform".into(), "RP2040".into());
    meta.insert("inputChannelCount".into(), 2.into());
    meta.insert("outputChannelCount".into(), 5.into());
    let (core, reply) = opened(&doc.to_string(), "small.dspipreset");
    assert_eq!(reply["platform"], "RP2040");
    assert_eq!(reply["inputs"], 2);
    assert_eq!(reply["outputs"], 5);
    let s = core.get_state();
    assert_eq!(s.platform_id, 0);
    assert_eq!(
        (s.num_input_channels, s.num_output_channels, s.num_channels),
        (2, 5, 7)
    );
    // The channels the RP2040 lacks are named, as an import names them.
    let notes = reply["notes"].as_array().unwrap();
    assert!(!notes.is_empty(), "{reply:#}");
    // The document's own values for what it does have: output 1's name.
    let doc_out1 = doc["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["isOutput"] == true && c["outputIndex"] == 0)
        .unwrap()["name"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(s.channel_names[2], doc_out1);
    // Saved, it is an RP2040 document.
    let saved = parse(&core.save_document("t".into(), 0).unwrap());
    assert_eq!(saved["meta"]["platform"], "RP2040");
    assert_eq!(saved["meta"]["outputChannelCount"], 5);
}

#[test]
fn wp_p1of_text_that_is_no_document_is_refused_and_nothing_opens() {
    let core = DspiCore::new();
    for bad in [
        "",
        "not json",
        "{\"schemaVersion\": 1}",
        "{\"schemaVersion\": 9, \"channels\": [{}]}",
    ] {
        assert!(
            matches!(
                core.open_document(bad.into(), "x.dspipreset".into()),
                Err(DspiError::InvalidArgument { .. })
            ),
            "{bad}"
        );
        assert!(!core.is_connected());
    }
    // A document with only outputs describes no device.
    let mut doc = parse(MAC_DOC);
    let meta = doc["meta"].as_object_mut().unwrap();
    meta.insert("inputChannelCount".into(), 0.into());
    doc["channels"]
        .as_array_mut()
        .unwrap()
        .retain(|c| c["isOutput"] == true);
    assert!(matches!(
        core.open_document(doc.to_string(), "x.dspipreset".into()),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert!(!core.is_connected());
}

// ── Templates ─────────────────────────────────────────────────────────

#[test]
fn wp_p1of_save_as_template_writes_kind_template_from_a_device_or_a_document() {
    let core = DspiCore::new();
    assert!(matches!(
        core.save_as_template("t".into(), "v".into(), 0),
        Err(DspiError::Disconnected)
    ));
    core.open_demo().unwrap();
    core.fetch_all().unwrap();
    let t = core.save_as_template("Club".into(), "v".into(), 0).unwrap();
    let info = parse(&files::config_inspect(t.clone()));
    assert_eq!(info["kind"], "template");
    assert_eq!(info["name"], "Club");
    // A template opened is a template; saved, it stays one.
    let (core, reply) = opened(&t, "club.dspipreset");
    assert_eq!(reply["kind"], "template");
    let again = core.save_document("v".into(), 0).unwrap();
    assert_eq!(parse(&again)["meta"]["kind"], "template");
    // The demo's settings came through: nothing differs.
    assert_eq!(parse(&files::config_diff(t, again))["count"], 0);
}

#[test]
fn wp_p1of_k340_template_opens_with_4_outputs_and_the_k340_pair_map() {
    let text = template_text("builtin:k340".into(), String::new()).unwrap();
    let doc = parse(&text);
    assert_eq!(doc["meta"]["kind"], "template");
    assert!(
        doc["meta"]["status"]
            .as_str()
            .unwrap()
            .contains("Approved by the owner"),
        "OD-P1h: the owner approved the file"
    );
    let (core, reply) = opened(&text, "k340.dspipreset");
    assert_eq!(reply["kind"], "template");
    assert_eq!(reply["platform"], "RP2350");
    let s = core.get_state();
    // Four outputs play, the rest are off.
    let on: Vec<usize> = (0..usize::from(s.num_output_channels))
        .filter(|&o| s.output_enabled[o])
        .collect();
    assert_eq!(on, [0, 1, 2, 3]);
    // The K340 pair map, in the image and over the extension channel.
    assert_eq!(s.crossfeed_pair_map, [2, 19, 0, 0]);
    assert_eq!(core.fetch_crossfeed_pair_map().unwrap(), [2, 19, 0, 0]);
    // Routing: USB 1 feeds outputs 3-4, USB 2 outputs 1-2.
    assert_eq!(&s.matrix_routing[0][..4], [false, false, true, true]);
    assert_eq!(&s.matrix_routing[1][..4], [true, true, false, false]);
    // The crossover at 3.5 kHz; no room EQ anywhere.
    let ins = usize::from(s.num_input_channels);
    for o in 0..4 {
        let band = s.filters[ins + o][0];
        let want = if o % 2 == 0 {
            FilterType::LowPass
        } else {
            FilterType::HighPass
        };
        assert_eq!(
            FilterType::from_raw(band.filter_type),
            want,
            "output {}",
            o + 1
        );
        assert_eq!(band.freq, 3500.0);
    }
    for bank in &s.filters {
        for b in bank {
            assert!(!is_room_eq(FilterType::from_raw(b.filter_type)), "{b:?}");
        }
    }
    // Default slot names, so BUG1's labels are derived from the routing.
    let labels = core.output_labels();
    let shown: Vec<(&str, &str, bool)> = labels
        .iter()
        .take(4)
        .map(|l| (l.channel.as_str(), l.band.as_str(), l.user_named))
        .collect();
    assert_eq!(
        shown,
        [
            ("R", "Low", false),
            ("R", "High", false),
            ("L", "Low", false),
            ("L", "High", false)
        ]
    );
    // Listed, with its status.
    let list = parse(&template_list(String::new()));
    let k340 = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "builtin:k340")
        .unwrap();
    assert_eq!(k340["builtin"], true);
    assert_eq!(k340["outputs"], 9);
    assert_eq!(k340["status"], doc["meta"]["status"]);
}

/// The PEQ types a room EQ uses: what the K340 template must not carry.
fn is_room_eq(t: FilterType) -> bool {
    matches!(
        t,
        FilterType::Peaking
            | FilterType::LowShelf
            | FilterType::HighShelf
            | FilterType::LowShelf1
            | FilterType::HighShelf1
            | FilterType::Notch
    )
}

#[test]
fn wp_p1of_templates_from_the_users_folder() {
    let Some(dir) = scratch("templates") else {
        return;
    };
    let core = DspiCore::new();
    core.open_demo().unwrap();
    core.fetch_all().unwrap();
    let t = core
        .save_as_template("Ünïcødé hall".into(), "v".into(), 0)
        .unwrap();
    let preset = core
        .config_export("Just a preset".into(), "v".into(), 0)
        .unwrap();
    std::fs::write(dir.join("hall.dspipreset"), &t).unwrap();
    std::fs::write(dir.join("preset.dspipreset"), preset).unwrap();
    std::fs::write(dir.join("torn.dspipreset"), &t[..t.len() / 2]).unwrap();
    std::fs::write(dir.join("notes.txt"), &t).unwrap();
    std::fs::create_dir_all(dir.join("sub.dspipreset")).unwrap();
    let user = dir.to_string_lossy().to_string();

    let list = parse(&template_list(user.clone()));
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["builtin:k340", "hall.dspipreset"]);
    let hall = &list[1];
    assert_eq!(hall["name"], "Ünïcødé hall");
    assert_eq!(hall["builtin"], false);
    assert_eq!(hall["status"], "");
    assert_eq!(
        template_text("hall.dspipreset".into(), user.clone()).unwrap(),
        t
    );

    // Only plain file names in the folder.
    for id in [
        "../hall.dspipreset",
        "sub/hall.dspipreset",
        "notes.txt",
        "",
        "builtin:nope",
    ] {
        assert!(
            matches!(
                template_text(id.into(), user.clone()),
                Err(DspiError::InvalidArgument { .. })
            ),
            "{id}"
        );
    }
    assert!(matches!(
        template_text("gone.dspipreset".into(), user.clone()),
        Err(DspiError::Io { .. })
    ));
    // A folder that is not there lists the shipped ones.
    let none = parse(&template_list(format!("{user}/missing")));
    assert_eq!(none.as_array().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn wp_p1of_file_names() {
    assert_eq!(stem("a/b/Club.v2.dspipreset"), "Club.v2");
    assert_eq!(stem("C:\\x\\hall.dspipreset"), "hall");
    assert_eq!(stem(".dspipreset"), ".dspipreset");
    assert_eq!(stem("plain"), "plain");
    assert!(plain_file_name("a.dspipreset"));
    assert!(!plain_file_name("a/b.dspipreset"));
    assert!(!plain_file_name("a.txt"));
}

// ── The K340 template's source ────────────────────────────────────────

/// Writes `assets/templates/k340.dspipreset` from a K340 backup (OD-P1h): the
/// owner's routing, pair map, crossover, names and hardware I/O, without room
/// EQ, as a template awaiting the owner's approval. Run in the container:
///
/// ```text
/// DSPI_P1OF_K340_BULK=/work/k340/bulk_params.bin \
/// DSPI_P1OF_K340_SAVED=2026-11-14T03:49:56Z \
/// DSPI_P1OF_K340_OUT=/src/assets/templates/k340.dspipreset \
///   cargo test -p dspi-core --lib wp_p1of_write_k340_template -- --ignored
/// ```
#[test]
#[ignore = "writes the K340 template from DSPI_P1OF_K340_BULK"]
fn wp_p1of_write_k340_template() {
    let (Ok(bulk), Ok(out)) = (
        std::env::var("DSPI_P1OF_K340_BULK"),
        std::env::var("DSPI_P1OF_K340_OUT"),
    ) else {
        return;
    };
    let bulk = std::fs::read(bulk).unwrap();
    let dev = FakeDevice::new(
        Capture {
            platform: Capture::demo().platform,
            bulk,
        },
        FakeOptions {
            extensions: true,
            ..FakeOptions::default()
        },
    );
    let core = DspiCore::new();
    core.open_transport(Box::new(dev), "K340".into(), 0)
        .unwrap();
    core.fetch_all().unwrap();
    {
        // No room EQ: the PEQ bank keeps only the crossover's filters.
        let mut inner = core.lock();
        for bank in &mut inner.state.filters {
            for b in bank.iter_mut() {
                if is_room_eq(FilterType::from_raw(b.filter_type)) {
                    *b = FilterParams {
                        filter_type: FilterType::Flat.to_raw(),
                        bypass: false,
                        freq: 1000.0,
                        q: 0.707,
                        gain: 0.0,
                        qp: 0.707,
                    };
                }
            }
        }
        // Owner 2026-11-14: the template starts at a low, safe user volume.
        inner.state.user_volume_db = -20.0;
    }
    let text = core
        .save_as_template(
            "K340".into(),
            format!("dspi-core {}", env!("CARGO_PKG_VERSION")),
            0,
        )
        .unwrap();
    // Edited as text, so the numbers stay as the export wrote them.
    let key = "\"savedUtc\": \"";
    let start = text.find(key).unwrap() + key.len();
    let end = start + text[start..].find('"').unwrap();
    let saved = std::env::var("DSPI_P1OF_K340_SAVED").unwrap_or_else(|_| text[start..end].into());
    let status = "Approved by the owner 2026-11-14 (OD-P1h). From the owner's K340 \
                  (backup after-hour3, 2026-11-14): routing, crossfeed pair map, crossover, \
                  names and hardware I/O; no room EQ; user volume starts at -20 dB.";
    let mut text = format!(
        "{}{saved}\",\n    \"{STATUS_KEY}\": {}{}",
        &text[..start],
        Value::from(status),
        &text[end + 1..]
    );
    text.push('\n');
    assert_eq!(parse(&text)["meta"][STATUS_KEY], status);
    std::fs::write(out, text).unwrap();
}
