//! WP PGMAT: what both compat-matrix test binaries share (PLAN-PG section 4): the
//! axes, section 1.11's cells encoded as outcomes, the checks run on a connected
//! core, and the rendered `matrix.md`. Included by `crates/dspi-core/tests/
//! wp_pgmat_matrix.rs` (`mod common`) and `crates/dspi-hub/tests/wp_pgmat_hub.rs`
//! (`#[path]`).
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dspi_core::consts::{SIGGEN_CTL_START, SIGGEN_CTL_STOP_NOW};
use dspi_core::{
    ConfirmOp, DspiCore, DspiError, FirmwareUpdateRule, ProtocolGeneration, SiggenConfig,
    firmware_update_check,
};
use dspi_proto::paged::cmds;
use dspi_transport::fake::{Capture, FakeBoard, FakeGeneration, FakeOptions};
use dspi_transport::link::LinkNotice;
use dspi_transport::{FakeDevice, FakeRemote};

pub const WAIT: Duration = Duration::from_secs(20);

// ── The axes ────────────────────────────────────────────────────────────────

/// A firmware row (section 1.11's rows, the G1b row split by guard and sender, and
/// the paged Zero of the PG-C review's carried-over cells).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fw {
    G1s,
    G1b,
    G1bGuarded,
    Sender,
    G2s,
    G2b,
    Zero,
    ZeroPaged,
    G2sNewer,
}

pub const ROWS: [Fw; 9] = [
    Fw::G1s,
    Fw::G1b,
    Fw::G1bGuarded,
    Fw::Sender,
    Fw::G2s,
    Fw::G2b,
    Fw::Zero,
    Fw::ZeroPaged,
    Fw::G2sNewer,
];

impl Fw {
    pub fn key(self) -> &'static str {
        match self {
            Fw::G1s => "g1s",
            Fw::G1b => "g1b",
            Fw::G1bGuarded => "g1b-guarded",
            Fw::Sender => "sender",
            Fw::G2s => "g2s",
            Fw::G2b => "g2b",
            Fw::Zero => "zero",
            Fw::ZeroPaged => "zero-paged",
            Fw::G2sNewer => "g2s-newer",
        }
    }

    pub fn is_paged(self) -> bool {
        matches!(self, Fw::G2s | Fw::G2b | Fw::ZeroPaged | Fw::G2sNewer)
    }

    pub fn is_zero(self) -> bool {
        matches!(self, Fw::Zero | Fw::ZeroPaged)
    }

    /// A fresh fake of this row; `allow`: let a violation through (counted).
    pub fn device(self, allow: bool) -> FakeDevice {
        let opts = |generation: Option<FakeGeneration>, board: FakeBoard| FakeOptions {
            generation,
            board,
            surfaces: true,
            analysis: true,
            allow_violation: allow,
            ..FakeOptions::default()
        };
        let g =
            |g: FakeGeneration| FakeDevice::new(Capture::demo(), opts(Some(g), FakeBoard::Pico));
        match self {
            Fw::G1s => g(FakeGeneration::Stock116),
            Fw::G1b => g(FakeGeneration::Bofr116 { page_guard: false }),
            Fw::G1bGuarded => g(FakeGeneration::Bofr116 { page_guard: true }),
            Fw::Sender => FakeDevice::sender(false),
            Fw::G2s => g(FakeGeneration::STOCK117),
            Fw::G2b => g(FakeGeneration::Bofr117),
            Fw::G2sNewer => g(FakeGeneration::Stock117 { revision: 200 }),
            Fw::Zero => FakeDevice::new(Capture::demo(), opts(None, FakeBoard::Zero)),
            Fw::ZeroPaged => FakeDevice::new(
                Capture::demo(),
                opts(Some(FakeGeneration::STOCK117), FakeBoard::Zero),
            ),
        }
        .with_serial(&format!("PGMAT-{}", self.key().to_uppercase()))
    }
}

/// A route of section 1.11: the first table's (our clients), and the second's (our
/// hub with an upstream-style client).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Usb,
    Midi,
    L12Forward,
    L12Drop,
    L13,
    /// Our hub, our client (`DspiCore` over DSPi Link).
    Oh,
    /// Our hub, an L12-style client: hello minor 2 and 1.1.6-shaped requests.
    OhL12Client,
    /// Our hub, an L13 client: DSPi-Link 1.3's fixtures replayed.
    OhL13Client,
}

impl Route {
    pub fn key(self) -> &'static str {
        match self {
            Route::Usb => "usb",
            Route::Midi => "midi",
            Route::L12Forward => "l12-forward",
            Route::L12Drop => "l12-drop",
            Route::L13 => "l13",
            Route::Oh => "oh",
            Route::OhL12Client => "oh-l12-client",
            Route::OhL13Client => "oh-l13-client",
        }
    }
}

// ── Outcomes ────────────────────────────────────────────────────────────────

/// How the extension channel was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ext {
    None,
    Full,
    /// Not for a control-role session, but for admin.
    AdminOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boot {
    Offered,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    Nothing,
    /// One `REQ_GET_PLATFORM` (page 0) and nothing else.
    PlatformOnly,
    /// `REQ_GET_PLATFORM`, and the meter poll's `REQ_GET_STATUS` 9 (page 0).
    PlatformAndMeterPoll,
}

/// A cell's outcome, in the terms section 1.11 uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Not a route the row has.
    NotApplicable,
    /// Works: the generation the client speaks to it, the extension channel, whether
    /// an update is offered; `bulk` and `generator` where the firmware has them.
    Full {
        paged: bool,
        ext: Ext,
        boot: Boot,
        bulk: bool,
        generator: bool,
    },
    /// A notice; what reached the device.
    Notice { notice: LinkNotice, sent: Sent },
    /// The core's connect sequence fails; at the transport the device is of this
    /// generation, with this extension channel.
    NoCore { paged: bool, ext: Ext },
    /// An upstream-style client on our hub: how it was served, whether its
    /// extension request was answered, and (on a Zero) whether its bootloader entry
    /// was refused.
    Up {
        kind: UpKind,
        ext: bool,
        entry: Option<Boot>,
    },
}

/// How our hub serves an upstream-style client (section 1.11's second table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpKind {
    /// A 1.1.6 device, 1.1.6 requests: as before PG.
    AsToday,
    /// A paged device, 1.1.6 requests: through the hub's adapter.
    Translated,
    /// A 1.1.6 device to a 1.3 client: listed without `protocol`, so the client
    /// sends nothing.
    Unlisted,
    /// A paged device to a 1.3 client: by page; a page-0 request answers 0x10.
    ByPage,
}

impl Outcome {
    pub const fn full(paged: bool, ext: Ext, boot: Boot) -> Self {
        Outcome::Full {
            paged,
            ext,
            boot,
            bulk: true,
            generator: true,
        }
    }

    pub fn short(self) -> String {
        match self {
            Outcome::NotApplicable => "n/a".into(),
            Outcome::Full {
                paged, ext, boot, ..
            } => {
                let mut s = String::from(if paged {
                    "full (paged)"
                } else {
                    "full (legacy)"
                });
                s.push_str(match ext {
                    Ext::None => ", no ext",
                    Ext::Full => " + ext",
                    Ext::AdminOnly => " + ext admin-only",
                });
                s.push_str(match boot {
                    Boot::Offered => "",
                    Boot::Blocked => ", no update",
                });
                if let Outcome::Full {
                    bulk: false,
                    generator: false,
                    ..
                } = self
                {
                    s.push_str(" (no bulk image, no generator)");
                }
                s
            }
            Outcome::Notice { notice, sent } => format!(
                "notice `{}`; {}",
                notice.code(),
                match sent {
                    Sent::Nothing => "nothing sent",
                    Sent::PlatformOnly => "only REQ_GET_PLATFORM sent",
                    Sent::PlatformAndMeterPoll =>
                        "REQ_GET_PLATFORM and the meter poll's REQ_GET_STATUS 9 sent",
                }
            ),
            Outcome::Up { kind, ext, entry } => {
                let mut t = String::from(match kind {
                    UpKind::AsToday => "works as today",
                    UpKind::Translated => "works through the hub's translation",
                    UpKind::Unlisted => "listed without `protocol`: the client sends nothing",
                    UpKind::ByPage => "full by page; a page-0 request answers 0x10",
                });
                if ext {
                    t.push_str(match kind {
                        UpKind::Translated => "; legacy ext 0x00 reaches the tunnel",
                        UpKind::ByPage => "; ext through 0xE010",
                        _ => "; ext 0x00 answers",
                    });
                }
                if entry == Some(Boot::Blocked) {
                    t.push_str("; a bootloader entry is DENIED");
                }
                t
            }
            Outcome::NoCore { paged, ext } => format!(
                "the core does not connect; at the transport {} {}",
                if paged { "paged" } else { "legacy" },
                match ext {
                    Ext::None => "with no ext",
                    Ext::Full => "+ ext",
                    Ext::AdminOnly => "+ ext admin-only",
                }
            ),
        }
    }

    pub fn boot(self) -> &'static str {
        match self {
            Outcome::Full {
                boot: Boot::Offered,
                ..
            } => "offered",
            Outcome::Full {
                boot: Boot::Blocked,
                ..
            } => "blocked",
            Outcome::Up {
                entry: Some(Boot::Blocked),
                ..
            } => "blocked",
            _ => "n/a",
        }
    }
}

/// Section 1.11's text for a cell and what it means.
pub struct Expect {
    pub plan: &'static str,
    pub outcome: Outcome,
}

pub const fn ex(plan: &'static str, outcome: Outcome) -> Expect {
    Expect { plan, outcome }
}

/// Section 1.11 (first table), cell by cell. The G1b row is "G1b (guarded or not),
/// sender"; the paged Zero's cells are the PG-C review's (PLAN-PG, "Carried over from
/// the PG-C review", PGMAT and PGZERO): known by its USB descriptor on USB, read as
/// a G2s by a 1.2 hub, unknown behind an upstream 1.3 hub (it reads as a Pico, so an
/// update is offered and `SYS.ENTER_BOOTLOADER` would go out: accepted by the plan).
pub fn expect(fw: Fw, route: Route) -> Expect {
    use Boot::*;
    use Outcome::*;
    let legacy = |ext| Outcome::full(false, ext, Offered);
    let paged = |ext| Outcome::full(true, ext, Offered);
    let no_midi = ex("(no MIDI on stock)", NotApplicable);
    let dash = ex("—", NotApplicable);
    let unserved = |plan| {
        ex(
            plan,
            Notice {
                notice: LinkNotice::HubServesPagedOnly,
                sent: Sent::Nothing,
            },
        )
    };
    let needs13 = |plan| {
        ex(
            plan,
            Notice {
                notice: LinkNotice::Needs13Hub,
                sent: Sent::PlatformOnly,
            },
        )
    };
    // The sender has its vendor subset only: no bulk image, no generator.
    let sender = |ext| Full {
        paged: false,
        ext,
        boot: Offered,
        bulk: false,
        generator: false,
    };
    match (fw, route) {
        (Fw::G1s, Route::Usb) => ex("full, no ext", legacy(Ext::None)),
        (Fw::G1s, Route::Midi) => no_midi,
        (Fw::G1s, Route::L12Forward | Route::L12Drop) => ex("full, no ext", legacy(Ext::None)),
        (Fw::G1s, Route::L13) => unserved("notice: hub serves 1.1.7+ only; nothing sent"),

        (Fw::G1b | Fw::G1bGuarded, Route::Usb | Route::Midi) => ex("full + ext", legacy(Ext::Full)),
        (Fw::G1b | Fw::G1bGuarded, Route::L12Forward | Route::L12Drop) => {
            ex("full; ext admin-only (as today)", legacy(Ext::AdminOnly))
        }
        (Fw::G1b | Fw::G1bGuarded | Fw::Sender, Route::L13) => {
            unserved("notice, nothing sent (a bridge does not list it)")
        }
        (Fw::Sender, Route::Usb | Route::Midi) => ex("full + ext", sender(Ext::Full)),
        (Fw::Sender, Route::L12Forward | Route::L12Drop) => {
            ex("full; ext admin-only (as today)", sender(Ext::AdminOnly))
        }

        (Fw::G2s, Route::Usb) => ex("full (adapter), no ext", paged(Ext::None)),
        (Fw::G2s, Route::Midi) => no_midi,
        (Fw::G2s, Route::L12Forward | Route::L12Drop) => {
            needs13("notice: does not answer 1.1.6, needs a 1.3 hub; only `REQ_GET_PLATFORM` sent")
        }
        (Fw::G2s, Route::L13) => ex("full (adapter), no ext", paged(Ext::None)),

        (Fw::G2b, Route::Usb) => ex("full (adapter) + ext (tunnel)", paged(Ext::Full)),
        (Fw::G2b, Route::Midi) => ex("full + ext (paged over MIDI)", paged(Ext::Full)),
        (Fw::G2b, Route::L12Forward | Route::L12Drop) => ex(
            "full as a legacy device (its legacy stack) + ext admin-only",
            legacy(Ext::AdminOnly),
        ),
        (Fw::G2b, Route::L13) => ex(
            "full + ext admin-only (0xE0 is `config` there)",
            paged(Ext::AdminOnly),
        ),

        (Fw::Zero, Route::Usb) => ex(
            "full as G1s, named \"DSPi Zero\", no update",
            Outcome::full(false, Ext::None, Blocked),
        ),
        (Fw::Zero, Route::Midi) => dash,
        (Fw::Zero, Route::L12Forward | Route::L12Drop) => {
            ex("full, no update", Outcome::full(false, Ext::None, Blocked))
        }
        (Fw::Zero, Route::L13) => unserved("notice"),

        (Fw::ZeroPaged, Route::Usb) => ex(
            "(PG-C review) known by its USB descriptor: \"DSPi Zero\", no update",
            Outcome::full(true, Ext::None, Blocked),
        ),
        (Fw::ZeroPaged, Route::Midi) => dash,
        (Fw::ZeroPaged, Route::L12Forward | Route::L12Drop) => {
            needs13("(PG-C review) as G2s: notice, only `REQ_GET_PLATFORM` sent")
        }
        (Fw::ZeroPaged, Route::L13) => ex(
            "(PG-C review) unknown: reads as a Pico, `SYS.ENTER_BOOTLOADER` would go out",
            Outcome::full(true, Ext::None, Offered),
        ),

        (Fw::G2sNewer, Route::Usb) => {
            ex("full; SET lengths judged by the device", paged(Ext::None))
        }
        (Fw::G2sNewer, Route::Midi) => dash,
        (Fw::G2sNewer, Route::L12Forward | Route::L12Drop) => needs13("as G2s"),
        (Fw::G2sNewer, Route::L13) => ex("full", paged(Ext::None)),

        // Our hub, our client.
        (Fw::G1s, Route::Oh) => ex("full, no ext", legacy(Ext::None)),
        // -- WP PGFIX -- (F3) the plan's wording: the extension channel is
        // `config` on our hub, so admin-only for a control session.
        (Fw::G1b | Fw::G1bGuarded, Route::Oh) => ex("full + ext (admin)", legacy(Ext::AdminOnly)),
        (Fw::Sender, Route::Oh) => ex("full + ext (admin)", sender(Ext::AdminOnly)),
        (Fw::G2s, Route::Oh) => ex("full (adapter), no ext", paged(Ext::None)),
        (Fw::G2b, Route::Oh) => ex("full + ext (admin)", paged(Ext::AdminOnly)),
        (Fw::Zero, Route::Oh) => ex(
            "full, no update or rollout; a tunnelled bootloader entry `DENIED`",
            Outcome::full(false, Ext::None, Blocked),
        ),
        (Fw::ZeroPaged, Route::Oh) => ex(
            "(PG-C review) known through `DeviceDescriptor::hub_board` (the app's local path, \
             a principal's child, our Link client): no update",
            Outcome::full(true, Ext::None, Blocked),
        ),
        (Fw::G2sNewer, Route::Oh) => ex("full", paged(Ext::None)),

        // Our hub, upstream clients (section 1.11's second table).
        (Fw::G1s | Fw::G1b | Fw::G1bGuarded | Fw::Sender | Fw::Zero, Route::OhL12Client) => ex(
            "as today",
            Up {
                kind: UpKind::AsToday,
                ext: matches!(fw, Fw::G1b | Fw::G1bGuarded | Fw::Sender),
                entry: (fw == Fw::Zero).then_some(Blocked),
            },
        ),
        (Fw::G2s | Fw::G2b | Fw::ZeroPaged | Fw::G2sNewer, Route::OhL12Client) => ex(
            "works through the hub's translation; legacy ext 0x00 reaches G2b's tunnel",
            Up {
                kind: UpKind::Translated,
                ext: fw == Fw::G2b,
                entry: (fw == Fw::ZeroPaged).then_some(Blocked),
            },
        ),
        (Fw::G1s | Fw::G1b | Fw::G1bGuarded | Fw::Sender | Fw::Zero, Route::OhL13Client) => ex(
            "listed without `protocol`: the client sends nothing (spec-conformant)",
            Up {
                kind: UpKind::Unlisted,
                ext: false,
                entry: None,
            },
        ),
        (Fw::G2s | Fw::G2b | Fw::ZeroPaged | Fw::G2sNewer, Route::OhL13Client) => ex(
            "full by page; a page-0 request answers `0x10`; ext invisible to it unless it uses \
             0xE010 (`config`)",
            Up {
                kind: UpKind::ByPage,
                ext: fw == Fw::G2b,
                entry: (fw == Fw::ZeroPaged).then_some(Blocked),
            },
        ),
    }
}

/// Cells known to differ from section 1.11, each with the reason. Reported by WP
/// PGMAT as findings; the plan is not changed to match. A listed cell must still
/// differ (the test fails once it matches, so the entry goes when the cause does).
// -- WP PGFIX -- PGMAT's F1 (the sender: the core connects with its own
// commands), F2 (no meter poll before the generation is known) and F3 (the plan's
// wording "full + ext (admin)" on our hub) are fixed: no cell differs.
pub const FINDINGS: &[(Fw, Route, &str)] = &[];

// ── One cell ────────────────────────────────────────────────────────────────

/// What a cell showed, besides its outcome.
#[derive(Default)]
pub struct Notes {
    pub connect: String,
    pub write: String,
    pub ext: String,
    pub bulk: String,
    pub boot: String,
    pub generator: String,
    pub extra: Vec<String>,
}

pub struct Cell {
    pub fw: Fw,
    pub route: Route,
    pub expected: Expect,
    pub observed: Outcome,
    pub notes: Notes,
}

pub fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn no_violation(fw: Fw, route: Route, remote: &FakeRemote, want: u32) {
    assert_eq!(
        remote.violations(),
        want,
        "{fw:?} {route:?}: {:?}",
        remote.last_violation()
    );
}

pub fn sine() -> SiggenConfig {
    SiggenConfig {
        signal_type: 0,
        channel_mask: 0x0001,
        invert_mask: 0,
        flags: 0,
        level_db: -30.0,
        duration_ms: 0,
        repeat: 1,
        gap_ms: 0,
        p: vec![1000.0, 0.0, 0.0, 0.0],
    }
}

/// The checks on a connected core. `role_ext`: the extension channel as this
/// session sees it is taken from `ext_status`.
pub fn exercise(
    fw: Fw,
    route: Route,
    core: &DspiCore,
    remote: &FakeRemote,
    notes: &mut Notes,
) -> (bool, bool, Boot, bool, bool) {
    let what = format!("{fw:?} {route:?}");
    let p = core
        .device_protocol()
        .unwrap_or_else(|e| panic!("{what}: {e}"));
    let is_paged = p.generation == ProtocolGeneration::Paged;
    notes.connect = format!(
        "{:?}, board {} \"{}\"",
        p.generation, p.board_id, p.board_name
    );
    if fw.is_zero() && route != Route::L13 {
        assert_eq!(p.board_name, "DSPi Zero", "{what}");
    }
    let ext = !core.ext_status().unwrap().supported.is_empty();
    notes.ext = format!("ext_status: {}", if ext { "listed" } else { "empty" });

    // One control write, read back.
    let full_dsp = fw != Fw::Sender;
    if full_dsp {
        core.set_master_volume(-13.0)
            .unwrap_or_else(|e| panic!("{what}: write: {e}"));
        core.fetch_all().unwrap();
        assert_eq!(core.get_state().master_volume_db, -13.0, "{what}");
        notes.write = "master volume -13 dB read back".into();
    } else {
        // The sender's control: its alink block through the extension channel.
        let info = core.alink_info();
        notes.write = format!(
            "alink_info: {}",
            if matches!(info, Ok(Some(_))) {
                "read"
            } else {
                "none"
            }
        );
    }

    // A bulk round trip.
    let bulk = full_dsp && {
        core.fetch_all().unwrap();
        let before = core.get_state().bulk_raw;
        core.apply_all_params()
            .unwrap_or_else(|e| panic!("{what}: bulk: {e}"));
        core.fetch_all().unwrap();
        let after = core.get_state().bulk_raw;
        assert_eq!(after, before, "{what}: the image came back");
        if route == Route::Usb || route == Route::Midi {
            assert_eq!(remote.image(), after, "{what}: the device holds it");
        }
        notes.bulk = format!("{} bytes out and back", after.len());
        true
    };

    // The generator.
    let generator = full_dsp && {
        core.siggen_set_config(sine())
            .unwrap_or_else(|e| panic!("{what}: siggen config: {e}"));
        core.siggen_control(SIGGEN_CTL_START)
            .unwrap_or_else(|e| panic!("{what}: START: {e}"));
        core.siggen_control(SIGGEN_CTL_STOP_NOW).unwrap();
        notes.generator = "START, STOP_NOW".into();
        true
    };

    // A board that reads as newer than the vendored page revision takes a
    // longer SET: the adapter leaves the length to the device (R8b).
    if fw == Fw::G2sNewer {
        let mut longer = (-9.0f32).to_le_bytes().to_vec();
        longer.push(0);
        core.raw_set_id(cmds::ROUTE_MASTER_VOLUME.0, 0, longer)
            .unwrap_or_else(|e| panic!("{what}: a longer SET: {e}"));
        notes
            .extra
            .push("a 5-byte ROUTE.MASTER_VOLUME SET went to the device".into());
    }

    // The bootloader: offered or blocked.
    let target = core.firmware_target();
    let verdict = firmware_update_check(Vec::new(), &target, "");
    let boot = if verdict.rule == FirmwareUpdateRule::NoBootsel {
        let entry =
            core.enter_bootloader_confirmed(Some(core.confirm_token(ConfirmOp::EnterBootloader)));
        assert_eq!(entry, Err(DspiError::Unsupported), "{what}");
        assert_eq!(remote.rebooted_to(), None, "{what}");
        notes.boot = format!(
            "blocked (board {:?}); the gated entry refused, nothing sent",
            target.board_id
        );
        Boot::Blocked
    } else {
        notes.boot = format!(
            "offered (board {:?}, rule {:?})",
            target.board_id, verdict.rule
        );
        Boot::Offered
    };
    (is_paged, ext, boot, bulk, generator)
}

pub fn ext_of(control: bool, admin: bool) -> Ext {
    match (control, admin) {
        (true, _) => Ext::Full,
        (false, true) => Ext::AdminOnly,
        (false, false) => Ext::None,
    }
}

pub fn line(c: &Cell) -> String {
    let finding = finding(c.fw, c.route).unwrap_or("");
    let verdict = if c.observed == c.expected.outcome {
        "as planned"
    } else {
        "DIFFERS"
    };
    let mut extra = c.notes.extra.join("; ");
    if !c.notes.connect.is_empty() {
        extra = if extra.is_empty() {
            c.notes.connect.clone()
        } else {
            format!("{}; {extra}", c.notes.connect)
        };
    }
    [
        c.fw.key(),
        c.route.key(),
        c.expected.plan,
        &c.expected.outcome.short(),
        &c.observed.short(),
        c.observed.boot(),
        &c.notes.write,
        &c.notes.ext,
        &c.notes.bulk,
        &c.notes.boot,
        &c.notes.generator,
        &extra,
        finding,
        verdict,
    ]
    .iter()
    .map(|s| s.replace(['\t', '\n'], " "))
    .collect::<Vec<_>>()
    .join("\t")
}

/// Record `cells` as `name`'s and hold them against the plan: no Zero cell offers
/// an update except the one the plan accepts, and every cell is as planned unless
/// it is a listed finding (which must still differ).
pub fn check(name: &str, cells: &[Cell]) {
    let lines: Vec<String> = cells.iter().map(line).collect();
    record(name, &lines);
    for c in cells {
        if c.fw.is_zero() && !(c.fw == Fw::ZeroPaged && c.route == Route::L13) {
            assert_ne!(c.observed.boot(), "offered", "{:?} {:?}", c.fw, c.route);
        }
    }
    let mut differ = Vec::new();
    for c in cells {
        let known = finding(c.fw, c.route).is_some();
        let same = c.observed == c.expected.outcome;
        if same == known {
            differ.push(format!(
                "{:?} {:?}: plan {:?} ({}), observed {:?}{}",
                c.fw,
                c.route,
                c.expected.outcome,
                c.expected.plan,
                c.observed,
                if known {
                    " (listed as a finding but matches)"
                } else {
                    ""
                }
            ));
        }
    }
    assert!(
        differ.is_empty(),
        "cells that differ from section 1.11:\n{}",
        differ.join("\n")
    );
}

pub fn finding(fw: Fw, route: Route) -> Option<&'static str> {
    FINDINGS
        .iter()
        .find(|(f, r, _)| *f == fw && *r == route)
        .map(|(_, _, why)| *why)
}

// ── The rendered table ──────────────────────────────────────────────────────

pub static RENDER: Mutex<()> = Mutex::new(());

pub fn matrix_dir() -> PathBuf {
    std::env::var_os("PGMAT_MATRIX_DIR")
        .map_or_else(|| std::env::temp_dir().join("pgmat"), PathBuf::from)
}

/// Write this test's cells and render `matrix.md` from every cell file present.
pub fn record(name: &str, lines: &[String]) {
    let _g = RENDER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = matrix_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("cells-{name}.tsv")),
        lines.join("\n") + "\n",
    )
    .unwrap();
    let md = render(&dir);
    let tmp = dir.join("matrix.md.tmp");
    std::fs::write(&tmp, md).unwrap();
    std::fs::rename(tmp, dir.join("matrix.md")).unwrap();
}

pub const ROW_NAMES: [(&str, &str); 9] = [
    ("g1s", "G1s"),
    ("g1b", "G1b (no PAGEGUARD)"),
    ("g1b-guarded", "G1b (PAGEGUARD)"),
    ("sender", "sender (guarded)"),
    ("g2s", "G2s"),
    ("g2b", "G2b"),
    ("zero", "Zero (G1s today)"),
    ("zero-paged", "Zero (paged, PG-C review)"),
    ("g2s-newer", "G2s at a newer page revision"),
];
pub const COLUMNS: [(&str, &str); 6] = [
    ("usb", "USB"),
    ("midi", "USB MIDI"),
    ("l12-forward", "L12 hub (page forwarded)"),
    ("l12-drop", "L12 hub (page dropped)"),
    ("l13", "L13 upstream hub"),
    ("oh", "OH (our hub)"),
];
pub const UPSTREAM_COLUMNS: [(&str, &str); 2] = [
    ("oh-l12-client", "L12 client (minor 2)"),
    ("oh-l13-client", "L13 client (1.3 fixtures)"),
];

pub fn render(dir: &std::path::Path) -> String {
    let mut cells: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("cells-") && n.ends_with(".tsv"))
        })
        .collect();
    files.sort();
    for f in &files {
        for l in std::fs::read_to_string(f).unwrap_or_default().lines() {
            let v: Vec<String> = l.split('\t').map(str::to_owned).collect();
            if v.len() >= 14 {
                cells.insert((v[0].clone(), v[1].clone()), v);
            }
        }
    }
    // "DIFFERS: F<n>" with the finding's number where one is listed.
    let verdict = |v: &Vec<String>| match v[12].split(':').next() {
        Some(id) if v[13] != "as planned" && id.starts_with('F') => format!("{}: {id}", v[13]),
        _ => v[13].clone(),
    };
    let mut out = String::from(
        "# PLAN-PG compat matrix (generated by WP PGMAT's tests)\n\n\
         Generated from the cells of `crates/dspi-core/tests/wp_pgmat_matrix.rs` and \
         `crates/dspi-hub/tests/wp_pgmat_hub.rs`. Each cell: what was observed, and \
         whether it is what PLAN-PG section 1.11 says (\"as planned\") or not (\"DIFFERS\", a \
         finding). \"full (legacy)\" / \"full (paged)\" is the generation our client spoke. \
         Through a hub the session is control; \"ext admin-only\" means a control session \
         did not get the extension channel and an admin did.\n\n\
         ## Our clients (every client, through the core)\n\n| Firmware \\ route |",
    );
    for (_, name) in COLUMNS {
        out.push_str(&format!(" {name} |"));
    }
    out.push_str("\n|---|");
    out.push_str(&"---|".repeat(COLUMNS.len()));
    out.push('\n');
    for (rk, rn) in ROW_NAMES {
        out.push_str(&format!("| {rn} |"));
        for (ck, _) in COLUMNS {
            match cells.get(&(rk.to_owned(), ck.to_owned())) {
                Some(v) if v[4] == "n/a" => out.push_str(&format!(" {} |", v[2])),
                Some(v) => out.push_str(&format!(
                    " {}; bootloader {} ({}) |",
                    v[4],
                    v[5],
                    verdict(v)
                )),
                None => out.push_str(" (not run) |"),
            }
        }
        out.push('\n');
    }
    out.push_str("\n## OH with upstream clients (best effort)\n\n| Device on OH |");
    for (_, name) in UPSTREAM_COLUMNS {
        out.push_str(&format!(" {name} |"));
    }
    out.push_str("\n|---|---|---|\n");
    for (rk, rn) in ROW_NAMES {
        if !UPSTREAM_COLUMNS
            .iter()
            .any(|(ck, _)| cells.contains_key(&(rk.to_owned(), (*ck).to_owned())))
        {
            continue;
        }
        out.push_str(&format!("| {rn} |"));
        for (ck, _) in UPSTREAM_COLUMNS {
            match cells.get(&(rk.to_owned(), ck.to_owned())) {
                Some(v) => out.push_str(&format!(" {} ({}) |", v[4], verdict(v))),
                None => out.push_str(" (not run) |"),
            }
        }
        out.push('\n');
    }
    out.push_str(
        "\n## Every cell\n\n| Firmware | Route | Plan (section 1.11) | Plan, encoded | Observed | \
         Bootloader | Write | Ext | Bulk | Bootloader check | Generator | Notes | Finding | Verdict |\n\
         |---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n",
    );
    for (rk, _) in ROW_NAMES {
        for (ck, _) in COLUMNS.iter().chain(UPSTREAM_COLUMNS.iter()) {
            if let Some(v) = cells.get(&(rk.to_owned(), (*ck).to_owned())) {
                out.push('|');
                for f in v.iter().take(14) {
                    out.push_str(&format!(" {} |", f.replace('|', "\\|")));
                }
                out.push('\n');
            }
        }
    }
    let mut findings: Vec<&str> = cells
        .values()
        .map(|v| v[12].as_str())
        .filter(|f| !f.is_empty())
        .collect();
    findings.sort_unstable();
    findings.dedup();
    if !findings.is_empty() {
        out.push_str("\n## Findings (cells that differ from section 1.11)\n\n");
        for f in &findings {
            out.push_str(&format!("- {f}\n"));
        }
    }
    let differ: Vec<_> = cells.values().filter(|v| v[13] != "as planned").collect();
    out.push_str(&format!(
        "\n{} cells, {} as planned, {} differ.\n",
        cells.len(),
        cells.len() - differ.len(),
        differ.len()
    ));
    out
}
