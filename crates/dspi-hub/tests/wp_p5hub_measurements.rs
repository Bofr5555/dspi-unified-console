//! WP P5HUB: the hub's measurement documents (`measure.doc.*`, `hub-api.md` section
//! 22) through the core's calls (`dspi_core::hub_server::measure`) against the real hub
//! in-process: each message by role, a 1 MiB document round trip in parts, a conflict,
//! the event, the corrupt-file rule, the merged list (the hub's copy wins), the client
//! read-only mode, and a hub without the cap (every call `Unsupported`, nothing sent).
//!
//! One test runs over TLS (`TlsMode::On`), one over plaintext (`TlsMode::Off`); the rest
//! take the mode `DSPI_HUB_TEST_TLS` gives (feature `test-tls-mode`), so the suite runs
//! with `on` and `compat` as well. Clients are paired through the hub's store (no PIN
//! over the network), so every mode pairs. No document content reaches a log.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use dspi_core::{DspiCore, DspiError, HubServerEvent, HubServerListener};
use dspi_hub::measurements::HubMeasurements;
use dspi_hub::session::Session;
use dspi_hub::{
    AccessMode, Hub, HubConfig, HubHandle, HubService, NetworkConfig, Role, ServiceConfig, TlsMode,
};
use dspi_measure::doc::{
    CaptureInfo, Curve, DocKind, Measurement, MeasurementDoc, Meta, MicCal, MicDoc,
    OutputMeasurement, RunKind, Stimulus,
};
use dspi_measure::types::{ChannelChain, LogGrid, Placement, ProgrammeFeed, Response, WayRole};
use dspi_transport::FakeDevice;
use serde_json::{Map, Value, json};

const WAIT: Duration = Duration::from_secs(30);
const SERIAL: &str = "P5HUB-1";
/// What a padded document carries: never in a log.
const PAD: &str = "PADP5HUBMARKER";

// ── Logs: no document content in them ──────────────────────────────────────

fn kept() -> &'static Mutex<Vec<String>> {
    static K: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    K.get_or_init(|| Mutex::new(Vec::new()))
}

struct Keep;

impl log::Log for Keep {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        !m.target().starts_with("tungstenite") && !m.target().starts_with("rustls")
    }
    fn log(&self, r: &log::Record<'_>) {
        if self.enabled(r.metadata()) {
            kept().lock().unwrap().push(format!("{}", r.args()));
        }
    }
    fn flush(&self) {}
}

fn keep_logs() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        static KEEP: Keep = Keep;
        let _ = log::set_logger(&KEEP);
        log::set_max_level(log::LevelFilter::Trace);
    });
}

/// No run of 100 or more base64 characters (a curve) and no padding marker in
/// anything logged, the core's lines included.
fn no_document_in_logs() {
    for line in kept().lock().unwrap().iter() {
        assert!(!line.contains(PAD), "a document in the log: {line}");
        let mut run = 0;
        for c in line.chars() {
            if c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' {
                run += 1;
                assert!(run < 100, "a base64 run in the log: {line}");
            } else {
                run = 0;
            }
        }
    }
}

// ── The rig ─────────────────────────────────────────────────────────────────

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // `ci/run.sh` points TMPDIR at ~/.cache/dspi-build/work/..., not tmpfs.
    let dir = std::env::temp_dir().join("wp-p5hub").join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn link_code(e: &DspiError) -> &str {
    match e {
        DspiError::Link { code, .. } => code,
        _ => "",
    }
}

/// How a rig serves the family.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    /// The hub's own service (`ServiceConfig::measure`).
    Hub,
    /// The same store behind [`Spy`], which notes every message first (no events).
    Spied,
    /// Not at all: [`Spy`] notes what a client sent anyway.
    Off,
}

/// Every `measure.*` message a client sent, noted before the store (if any) answers it.
struct Spy {
    seen: Mutex<Vec<String>>,
    store: Option<HubMeasurements>,
}

impl HubService for Spy {
    fn name(&self) -> &'static str {
        "p5hub spy"
    }
    fn caps(&self) -> Vec<&'static str> {
        if self.store.is_some() {
            vec![dspi_hub::measurements::CAP]
        } else {
            Vec::new()
        }
    }
    fn handle_message(&self, _: &HubHandle, s: &Arc<Session>, msg: &Value) -> Option<String> {
        let t = msg.get("t")?.as_str()?;
        if t.starts_with("measure.") {
            self.seen.lock().unwrap().push(t.to_owned());
        }
        self.store.as_ref()?.handle_message(s, Instant::now(), msg)
    }
}

impl Spy {
    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

struct Rig {
    hub: Hub,
    dir: PathBuf,
    spy: Arc<Spy>,
}

/// A hub with a data folder and one demo device; `tls` `None` for the mode
/// `DSPI_HUB_TEST_TLS` gives; `family` how the family is served.
fn rig(name: &str, tls: Option<TlsMode>, family: Family) -> Rig {
    keep_logs();
    let dir = scratch(name);
    let spy = Arc::new(Spy {
        seen: Mutex::new(Vec::new()),
        store: (family == Family::Spied).then(|| HubMeasurements::open(&dir.join("store"))),
    });
    let base = NetworkConfig::new(dir.join("store"));
    let hub = Hub::start(HubConfig {
        network: Some(NetworkConfig {
            port: 0,
            access: AccessMode::Pin,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            tls: tls.unwrap_or(base.tls),
            ..base
        }),
        initial_scan: false,
        scan_interval: Duration::from_millis(10),
        services: ServiceConfig {
            measure: family == Family::Hub,
            extra: vec![Arc::clone(&spy) as Arc<dyn HubService>],
            ..ServiceConfig::default()
        },
        ..HubConfig::in_process("p5hub")
    })
    .unwrap();
    hub.attach(Box::new(FakeDevice::demo().with_serial(SERIAL)), 0)
        .unwrap();
    Rig { hub, dir, spy }
}

impl Rig {
    /// A core with `role` on the hub, bound to the device.
    fn core(&self, name: &str, role: Role) -> Arc<DspiCore> {
        let net = self.hub.network().unwrap();
        let window = net.auth().begin_pairing(Instant::now()).unwrap();
        let authed = net
            .auth()
            .pair(
                &window.pin,
                name,
                Some(role.min(Role::Control)),
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                Instant::now(),
            )
            .unwrap();
        if role == Role::Admin {
            assert!(net.auth().set_role(authed.cid, Role::Admin));
        }
        let core = DspiCore::new();
        core.link_connect(
            format!("ws://127.0.0.1:{}/dspi/v1", net.port()),
            name.into(),
            "0".into(),
            authed.token.unwrap(),
            // -- WP LINKFIX -- a bare token goes out only with the hub's id
            self.hub.hub_id().to_owned(),
        )
        .unwrap();
        wait_until("the device list", || !core.link_devices().is_empty());
        let d = core.link_devices().remove(0);
        core.link_select_device(d.handle, d.serial).unwrap();
        core
    }

    fn store(&self) -> PathBuf {
        self.dir.join("store").join("measurements")
    }
}

#[derive(Default)]
struct Heard(Mutex<Vec<HubServerEvent>>);

impl HubServerListener for Heard {
    fn on_hub_server_event(&self, event: HubServerEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Heard {
    fn wait_for(&self, what: &str, f: impl Fn(&HubServerEvent) -> bool) -> HubServerEvent {
        let mut found = None;
        wait_until(what, || {
            found = self.0.lock().unwrap().iter().find(|e| f(e)).cloned();
            found.is_some()
        });
        found.unwrap()
    }
}

// ── Documents ───────────────────────────────────────────────────────────────

fn curve(seed: f64) -> Curve {
    let n = LogGrid::STANDARD.freqs().unwrap().len();
    Curve::from_response(&Response {
        grid: LogGrid::STANDARD,
        mag_db: (0..n)
            .map(|i| 75.0 + seed + (i as f64 * 0.013).sin() * 4.0)
            .collect(),
        phase_deg: Some((0..n).map(|i| ((i as f64) * 1.7) % 360.0 - 180.0).collect()),
        valid: Some((0..n).map(|i| i % 7 != 0).collect()),
        spl: true,
    })
}

/// A measurement of `serial` ("" none) with `outputs` outputs with phase, id `id`
/// ("" none), padded with a key the reader keeps to about `pad_to` bytes filed.
fn measurement(serial: &str, id: &str, name: &str, outputs: u8, pad_to: usize) -> String {
    let mut doc = MeasurementDoc {
        schema_version: 1,
        meta: Meta::new(DocKind::Measurement, name, "2026-11-17T10:00:00Z", "test"),
        measurement: Measurement {
            id: id.into(),
            serial: serial.into(),
            platform: "RP2350".into(),
            saved_ms: 1_763_373_600_000,
            placement: Placement::ListeningPosition,
            mic: None,
            stimulus: Stimulus {
                kind: RunKind::Walked,
                f1_hz: 20.0,
                f2_hz: 20_000.0,
                duration_ms: 5_500,
                repeat: 3,
                gap_ms: 1_000,
                level_db: -30.0,
                device_rate_hz: 48_000,
                extra: Map::new(),
            },
            capture: CaptureInfo {
                rate_hz: 48_000,
                clock_ratio: 1.0,
                timing_reliable: true,
                max_residual_us: Some(2.0),
                extra: Map::new(),
            },
            outputs: (0..outputs)
                .map(|o| OutputMeasurement {
                    output: o,
                    role: WayRole::Full,
                    speaker: "L".into(),
                    chain: ChannelChain {
                        peq: vec![],
                        crossover: vec![],
                        gain_db: 0.0,
                        delay_ms: 0.0,
                        feed: ProgrammeFeed {
                            gain_db: 0.0,
                            inverted: false,
                        },
                    }
                    .into(),
                    arrival_ms: Some(3.0),
                    response: curve(f64::from(o)),
                    extra: Map::new(),
                })
                .collect(),
            positions: 1,
            note: String::new(),
            tunes: vec![],
            extra: Map::new(),
        },
        extra: Map::new(),
    };
    let bare = doc.to_text().unwrap().len();
    if pad_to > bare + 64 {
        let n = (pad_to - bare - 64) / PAD.len();
        doc.measurement
            .extra
            .insert("padding".into(), json!(PAD.repeat(n)));
    }
    doc.to_text().unwrap()
}

fn mic(id: &str, name: &str) -> String {
    let umik = include_str!("../../dspi-measure/testdata/p5/cal/umik1_7000001.txt");
    let cal = dspi_measure::cal::parse_calibration(umik, "umik1_7000001.txt").unwrap();
    let mut m = MicCal::from_calibration(name, &cal, None, &LogGrid::STANDARD).unwrap();
    m.id = id.into();
    MicDoc {
        schema_version: 1,
        meta: Meta::new(DocKind::Mic, name, "2026-11-17T10:00:00Z", "test"),
        mic: m,
        extra: Map::new(),
    }
    .to_text()
    .unwrap()
}

fn same_document(a: &str, b: &str) {
    let a: Value = serde_json::from_str(a).unwrap();
    let b: Value = serde_json::from_str(b).unwrap();
    assert!(a == b, "the documents differ");
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[test]
fn wp_p5hub_every_message_by_role_over_tls() {
    let rig = rig("roles", Some(TlsMode::On), Family::Hub);
    let viewer = rig.core("p5hub-viewer", Role::Viewer);
    let control = rig.core("p5hub-control", Role::Control);
    let admin = rig.core("p5hub-admin", Role::Admin);
    assert!(control.link_security().encrypted);
    for c in [&viewer, &control, &admin] {
        assert!(c.hub_caps().hub_measure, "{:?}", c.hub_caps());
    }
    let heard = Arc::new(Heard::default());
    viewer.set_hub_server_listener(Some(heard.clone()));

    // Control puts a measurement with neither a serial nor an id: the open device's
    // serial and an id from its time.
    let text = measurement("", "", "Living room", 2, 0);
    assert_eq!(
        viewer.hub_measure_doc_put(text.clone()).unwrap_err(),
        DspiError::Denied
    );
    let put = control.hub_measure_doc_put(text.clone()).unwrap();
    assert!(put.created);
    assert_eq!(put.doc.kind, "measurement");
    assert_eq!(put.doc.serial, SERIAL);
    assert_eq!(put.doc.id, "1763373600000");
    assert!(put.doc.bytes > 10_000, "{:?}", put.doc);
    let again = control.hub_measure_doc_put(text).unwrap();
    assert!(!again.created);
    let id = put.doc.id.clone();
    let ev = heard.wait_for("the put", |e| {
        e.kind == "measure.doc.changed" && e.change == "put"
    });
    assert_eq!(ev.measure_doc, id);
    assert_eq!(ev.measure_kind, "measurement");
    assert_eq!(ev.serial, SERIAL);

    // A microphone; every role reads.
    let m = control.hub_measure_doc_put(mic("77-0", "UMIK-1")).unwrap();
    assert_eq!(m.doc.kind, "mic");
    assert!(m.doc.serial.is_empty());
    let all = viewer
        .hub_measure_docs(String::new(), String::new())
        .unwrap();
    let ids: Vec<&str> = all.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, [id.as_str(), "77-0"]);
    assert_eq!(
        viewer
            .hub_measure_docs("mic".into(), String::new())
            .unwrap()
            .len(),
        1
    );
    assert!(
        viewer
            .hub_measure_docs("measurement".into(), "OTHER".into())
            .unwrap()
            .is_empty()
    );
    let got = viewer.hub_measure_doc_get(id.clone()).unwrap();
    assert_eq!(got.doc, put.doc);
    let doc: Value = serde_json::from_str(&got.document).unwrap();
    assert_eq!(doc["measurement"]["serial"], SERIAL);
    assert_eq!(doc["measurement"]["id"], id.as_str());

    // Rename and note: Control and Admin.
    assert_eq!(
        viewer
            .hub_measure_doc_rename(id.clone(), "x".into())
            .unwrap_err(),
        DspiError::Denied
    );
    let r = control
        .hub_measure_doc_rename(id.clone(), "Wohnzimmer ♫".into())
        .unwrap();
    assert_eq!(r.name, "Wohnzimmer ♫");
    let n = admin
        .hub_measure_doc_note(id.clone(), "after the rug".into())
        .unwrap();
    assert_eq!(n.note, "after the rug");
    assert!(matches!(
        control.hub_measure_doc_rename(id.clone(), "x".repeat(257)),
        Err(DspiError::InvalidArgument { .. })
    ));
    heard.wait_for("the note", |e| e.change == "note" && e.measure_doc == id);

    // Delete: Admin only.
    assert_eq!(
        control.hub_measure_doc_delete(id.clone()).unwrap_err(),
        DspiError::Denied
    );
    admin.hub_measure_doc_delete(id.clone()).unwrap();
    let ev = heard.wait_for("the delete", |e| e.change == "delete");
    assert_eq!(ev.measure_doc, id);
    let e = viewer.hub_measure_doc_get(id).unwrap_err();
    assert_eq!(link_code(&e), "not_found", "{e:?}");
    assert_eq!(e.notice_key(), "dlink.not_found");

    // Another kind with a taken id, and documents of no kind.
    let e = control
        .hub_measure_doc_put(measurement(SERIAL, "77-0", "m", 1, 0))
        .unwrap_err();
    assert_eq!(link_code(&e), "conflict", "{e:?}");
    assert!(matches!(
        control.hub_measure_doc_put("{\"schemaVersion\":1}".into()),
        Err(DspiError::InvalidArgument { .. })
    ));
    no_document_in_logs();
}

#[test]
fn wp_p5hub_a_1_mib_document_round_trips_in_parts() {
    let rig = rig("big", None, Family::Spied);
    let control = rig.core("p5hub-big-control", Role::Control);
    let viewer = rig.core("p5hub-big-viewer", Role::Viewer);
    // As large as a measurement may be: 16 outputs with phase, padded close to 1 MiB.
    let text = measurement(SERIAL, "2-0", "Sixteen outputs", 16, (1 << 20) - 2048);
    assert!(
        text.len() > (1 << 20) - 4096 && text.len() <= 1 << 20,
        "{}",
        text.len()
    );
    let compact: Value = serde_json::from_str(&text).unwrap();
    assert!(
        compact.to_string().len() > 15 * 65_536,
        "past fifteen frames"
    );
    let put = control.hub_measure_doc_put(text.clone()).unwrap();
    assert!(put.created);
    assert_eq!(put.doc.bytes, text.len() as u64);
    let got = viewer.hub_measure_doc_get("2-0".into()).unwrap();
    same_document(&got.document, &text);
    assert_eq!(got.doc, put.doc);
    // Again: the same document, not a new one.
    assert!(!control.hub_measure_doc_put(text).unwrap().created);
    // One byte past the bound is refused here, with nothing sent.
    let before = rig.spy.count();
    let mut over: Value = compact;
    over["measurement"]["padding"] = json!(PAD.repeat(80_000));
    assert!(matches!(
        control.hub_measure_doc_put(over.to_string()),
        Err(DspiError::InvalidArgument { .. })
    ));
    assert_eq!(rig.spy.count(), before);
    assert!(
        viewer
            .hub_measure_docs("measurement".into(), SERIAL.into())
            .unwrap()
            .iter()
            .any(|d| d.id == "2-0")
    );
    no_document_in_logs();
}

#[test]
fn wp_p5hub_conflict_merge_and_the_corrupt_file_rule_over_plaintext() {
    let rig = rig("merge", Some(TlsMode::Off), Family::Hub);
    let control = rig.core("p5hub-merge", Role::Control);
    assert!(!control.link_security().encrypted);
    let local_dir = rig.dir.join("client");
    control
        .measure_set_dir(local_dir.to_string_lossy().into_owned())
        .unwrap();
    // Three documents locally: one also on the hub as it is, one renamed on the hub,
    // one only here; and one only on the hub.
    let a = measurement(SERIAL, "10-0", "A", 1, 0);
    let b = measurement(SERIAL, "11-0", "B", 1, 0);
    let c = measurement(SERIAL, "12-0", "C", 1, 0);
    for t in [&a, &b, &c] {
        control.measure_doc_put(t.clone()).unwrap();
    }
    control.hub_measure_doc_put(a.clone()).unwrap();
    control.hub_measure_doc_put(b.clone()).unwrap();
    control
        .hub_measure_doc_rename("11-0".into(), "B on the hub".into())
        .unwrap();
    control
        .hub_measure_doc_put(measurement(SERIAL, "13-0", "D", 1, 0))
        .unwrap();
    // The local B is now different from the hub's: conflict, the hub's stays.
    let e = control.hub_measure_doc_put(b).unwrap_err();
    assert_eq!(link_code(&e), "conflict", "{e:?}");
    assert_eq!(e.notice_key(), "dlink.conflict");

    let m = control
        .measure_docs_merged("measurement".into(), SERIAL.into())
        .unwrap();
    assert!(m.hub_listed && m.hub_notice_key.is_empty(), "{m:?}");
    let rows: Vec<(&str, &str, &str, bool)> = m
        .entries
        .iter()
        .map(|r| {
            (
                r.doc.id.as_str(),
                r.doc.name.as_str(),
                r.location.as_str(),
                r.local_copy,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("10-0", "A", "both", false),
            ("11-0", "B on the hub", "hub", false),
            ("11-0", "B", "local", true),
            ("12-0", "C", "local", false),
            ("13-0", "D", "hub", false),
        ]
    );

    // A filed document that no longer reads is set aside, and is not found from then.
    let filed = rig
        .store()
        .join(dspi_session::measure_store::dir_name(SERIAL))
        .join("13-0.dspipreset");
    std::fs::write(&filed, "{ torn").unwrap();
    let e = control.hub_measure_doc_get("13-0".into()).unwrap_err();
    assert_eq!(link_code(&e), "not_found", "{e:?}");
    assert!(!filed.exists());
    assert!(
        std::fs::read_dir(filed.parent().unwrap())
            .unwrap()
            .flatten()
            .any(|e| e
                .file_name()
                .to_string_lossy()
                .starts_with("13-0.dspipreset.bad-"))
    );
    // The hub's files are its own (0600).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let p = filed.with_file_name("10-0.dspipreset");
        assert_eq!(
            std::fs::metadata(p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    no_document_in_logs();
}

#[test]
fn wp_p5hub_the_read_only_mode_lets_out_the_reads_only() {
    let rig = rig("readonly", None, Family::Spied);
    let control = rig.core("p5hub-ro", Role::Control);
    control.hub_measure_doc_put(mic("20-0", "Mic")).unwrap();
    control.set_client_read_only(true).unwrap();
    let before = rig.spy.count();
    assert_eq!(
        control
            .hub_measure_doc_put(mic("21-0", "Mic 2"))
            .unwrap_err(),
        DspiError::ReadOnly
    );
    assert_eq!(
        control
            .hub_measure_doc_rename("20-0".into(), "x".into())
            .unwrap_err(),
        DspiError::ReadOnly
    );
    assert_eq!(
        control.hub_measure_doc_delete("20-0".into()).unwrap_err(),
        DspiError::ReadOnly
    );
    assert_eq!(rig.spy.count(), before);
    assert_eq!(
        control
            .hub_measure_docs("mic".into(), String::new())
            .unwrap()
            .len(),
        1
    );
    control.hub_measure_doc_get("20-0".into()).unwrap();
}

#[test]
fn wp_p5hub_without_the_cap_every_call_is_unsupported_and_nothing_is_sent() {
    let rig = rig("nocap", None, Family::Off);
    let admin = rig.core("p5hub-nocap", Role::Admin);
    assert!(!admin.hub_caps().hub_measure);
    assert!(admin.hub_caps().on_hub);
    let local_dir = rig.dir.join("client");
    admin
        .measure_set_dir(local_dir.to_string_lossy().into_owned())
        .unwrap();
    admin.measure_doc_put(mic("30-0", "Mine")).unwrap();
    let u = DspiError::Unsupported;
    assert_eq!(
        admin
            .hub_measure_docs(String::new(), String::new())
            .unwrap_err(),
        u
    );
    assert_eq!(admin.hub_measure_doc_get("30-0".into()).unwrap_err(), u);
    assert_eq!(admin.hub_measure_doc_put(mic("31-0", "x")).unwrap_err(), u);
    assert_eq!(
        admin
            .hub_measure_doc_rename("30-0".into(), "x".into())
            .unwrap_err(),
        u
    );
    assert_eq!(
        admin
            .hub_measure_doc_note("30-0".into(), "x".into())
            .unwrap_err(),
        u
    );
    assert_eq!(admin.hub_measure_doc_delete("30-0".into()).unwrap_err(), u);
    // The documents stay local: the merged list is this console's.
    let m = admin
        .measure_docs_merged(String::new(), String::new())
        .unwrap();
    assert!(!m.hub_listed && m.hub_notice_key.is_empty(), "{m:?}");
    assert_eq!(m.entries.len(), 1);
    assert_eq!(m.entries[0].location, "local");
    assert_eq!(rig.spy.count(), 0, "something was sent");
    assert!(!rig.store().exists());
}

#[test]
fn wp_p5hub_the_store_folder_is_the_hubs_data_folder() {
    // A hub with no data folder keeps nothing and announces nothing.
    let hub = Hub::start(HubConfig {
        initial_scan: false,
        ..HubConfig::in_process("p5hub no data")
    })
    .unwrap();
    let console = hub
        .handle()
        .open_principal(Role::Admin, None, "p5hub console")
        .unwrap();
    let answer = hub.handle().service_request(
        &console,
        &json!({"t": "measure.doc.list", "id": 1, "kind": "mic"}),
    );
    assert!(answer.is_none(), "{answer:?}");
    hub.handle().close_session(console.id());
}
