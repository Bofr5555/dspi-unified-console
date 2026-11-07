//! WP TM3: the whole flow on a fake board and a fake bootloader drive. The drive
//! is a real directory under the target directory, so `flash_to_drive_guarded`
//! does its real checks and its real write; only the board is pretend.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use dspi_proto::generated::opcodes as op;
use dspi_transport::fake::{Capture, FakeOptions, FakeRemote};
use dspi_transport::{DeviceDescriptor, FakeDevice, NotificationSource, TransportCaps};
use dspi_transport::{Result as TResult, TransportError};
use dspi_tui::shell::connect::{LinkTarget, guard_raw};

use super::*;

#[path = "../../tests/support_tm3/mod.rs"]
#[allow(dead_code, clippy::arithmetic_side_effects)]
mod support;
use support::{RP2040, RP2040_INFO, RP2350, RP2350_INFO, mount};

const PHRASE: &str = "I ACCEPT THE CORE VOLTAGE RISK";
const WORD: &str = "OVERVOLT";
/// What the demo device reports.
const DEMO_VERSION: &str = "1.1.6-beta4";
/// `EXT_CORE_VREG` selection codes.
const V1_15: u8 = 0x0C;
const V1_20: u8 = 0x0D;
const V1_25: u8 = 0x0E;
const V1_30: u8 = 0x0F;

fn scratch(name: &str) -> PathBuf {
    // Beside the test binary, in the target directory: never on tmpfs.
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("wp_tm3")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn in_(opcodes: &[(bool, u8)], opcode: u8) -> usize {
    opcodes.iter().filter(|(_, o)| *o == opcode).count()
}

/// A device that logs its transfers, and on the bootloader request does what a
/// real one does: answers nothing more, and its drive arrives.
struct Spy {
    dev: FakeDevice,
    world: Arc<Mutex<Board>>,
}

impl Spy {
    fn note(&self, out: bool, opcode: u8) -> TResult<()> {
        let mut b = self.world.lock().unwrap();
        if b.rebooted {
            return Err(TransportError::Disconnected);
        }
        b.log.lock().unwrap().push((out, opcode));
        if opcode == op::REQ_ENTER_BOOTLOADER {
            b.rebooted = true;
            if let Some(info) = b.arrives {
                mount(&b.drive, info);
            }
            return Err(TransportError::Disconnected);
        }
        Ok(())
    }
}

impl Transport for Spy {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> TResult<Vec<u8>> {
        self.note(false, opcode)?;
        self.dev.control_in(opcode, value, len)
    }
    fn control_in_upto(&mut self, opcode: u8, value: u16, len: u16) -> TResult<Vec<u8>> {
        self.note(false, opcode)?;
        self.dev.control_in_upto(opcode, value, len)
    }
    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> TResult<()> {
        self.note(true, opcode)?;
        self.dev.control_out(opcode, value, data)
    }
    fn descriptor(&self) -> &DeviceDescriptor {
        self.dev.descriptor()
    }
    fn max_transfer(&self) -> usize {
        self.dev.max_transfer()
    }
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        None
    }
    fn caps(&self) -> TransportCaps {
        self.dev.caps()
    }
}

/// What happens to the board while a prompt waits.
type DuringAsk = Box<dyn FnMut(&FakeRemote)>;

/// Each transfer: whether it was an OUT, and its opcode.
type Log = Arc<Mutex<Vec<(bool, u8)>>>;

/// The board and its surroundings.
struct Board {
    drive: PathBuf,
    /// What the drive's identity file says once the board reboots; `None`: no
    /// drive ever arrives.
    arrives: Option<&'static str>,
    log: Log,
    rebooted: bool,
    /// Comes back after the write, reporting this.
    returns: Option<&'static str>,
    capture: Capture,
    vreg_default: Option<u8>,
    /// The running device, for a test to change it during a prompt.
    remote: Option<FakeRemote>,
    opened: u32,
    extra_drive: Option<PathBuf>,
}

fn capture_with_version(version: &str) -> Capture {
    let mut capture = Capture::demo();
    let parts: Vec<u8> = version.split('.').map(|p| p.parse().unwrap()).collect();
    capture.platform[1] = parts[0];
    capture.platform[4] = parts[1];
    capture.platform[5] = parts[2];
    capture.platform[6] = 0;
    capture
}

struct Fake {
    board: Arc<Mutex<Board>>,
    answers: VecDeque<String>,
    asked: Vec<String>,
    during_ask: Option<DuringAsk>,
    /// On the bus besides the board, and what shows up once the file is on.
    bus: Vec<&'static str>,
    arrived: Vec<&'static str>,
    opened_as: Vec<Target>,
    pauses: u32,
}

impl Fake {
    fn new(name: &str) -> Self {
        let drive = scratch(name).join("RP2350");
        Self {
            board: Arc::new(Mutex::new(Board {
                drive,
                arrives: Some(RP2350_INFO),
                log: Arc::default(),
                rebooted: false,
                returns: Some(DEMO_VERSION),
                capture: Capture::demo(),
                vreg_default: Some(V1_15),
                remote: None,
                opened: 0,
                extra_drive: None,
            })),
            answers: VecDeque::new(),
            asked: Vec::new(),
            during_ask: None,
            bus: Vec::new(),
            arrived: Vec::new(),
            opened_as: Vec::new(),
            pauses: 0,
        }
    }

    /// A K340-like board: its firmware's default is 1.20 V.
    fn at_risk(self) -> Self {
        self.vreg(V1_20)
    }

    fn vreg(self, code: u8) -> Self {
        self.board.lock().unwrap().vreg_default = Some(code);
        self
    }

    fn answers(mut self, answers: &[&str]) -> Self {
        self.answers = answers.iter().map(|a| (*a).to_owned()).collect();
        self
    }

    fn already_mounted(self) -> Self {
        mount(&self.drive(), RP2350_INFO);
        self
    }

    /// A board of this image, other wire formats included.
    fn image(self, bulk: Vec<u8>) -> Self {
        self.board.lock().unwrap().capture.bulk = bulk;
        self
    }

    fn drive(&self) -> PathBuf {
        self.board.lock().unwrap().drive.clone()
    }

    fn written(&self) -> Option<Vec<u8>> {
        std::fs::read(self.drive().join("firmware.uf2")).ok()
    }

    fn log(&self) -> Vec<(bool, u8)> {
        self.board.lock().unwrap().log.lock().unwrap().clone()
    }

    fn entered(&self) -> usize {
        in_(&self.log(), op::REQ_ENTER_BOOTLOADER)
    }

    fn writes(&self) -> Vec<(bool, u8)> {
        self.log().into_iter().filter(|(out, _)| *out).collect()
    }
}

impl World for Fake {
    fn drives(&mut self) -> Vec<BootloaderDrive> {
        let b = self.board.lock().unwrap();
        std::iter::once(&b.drive)
            .chain(b.extra_drive.iter())
            .filter_map(|path| drive_at(path))
            .collect()
    }

    fn open(&mut self, target: &Target) -> Result<Box<dyn Transport>, OpenError> {
        self.opened_as.push(target.clone());
        let mut b = self.board.lock().unwrap();
        let capture = if b.rebooted {
            // Off the bus until the file has gone on, and then what it runs.
            let wrote = b.drive.join("firmware.uf2").exists();
            match b.returns.filter(|_| wrote) {
                Some(DEMO_VERSION) => Capture::demo(),
                Some(other) => capture_with_version(other),
                None => return Err(TransportError::NotFound.into()),
            }
        } else {
            b.capture.clone()
        };
        b.opened += 1;
        let options = FakeOptions {
            extensions: true,
            ..FakeOptions::default()
        };
        let dev = FakeDevice::new(capture, options);
        if let (Some(code), false) = (b.vreg_default, b.rebooted) {
            dev.remote().vreg_set_build_default(code);
        }
        b.remote = Some(dev.remote());
        b.rebooted = false;
        drop(b);
        let spy = Spy {
            dev,
            world: Arc::clone(&self.board),
        };
        // The same door `dspi raw` uses, so a read-only device is guarded here.
        guard_raw(Box::new(spy))
    }

    fn serials(&mut self) -> Vec<String> {
        let mut on_bus = self.bus.clone();
        if self.written().is_some() {
            on_bus.extend(&self.arrived);
        }
        on_bus.into_iter().map(str::to_owned).collect()
    }

    fn ask(&mut self, prompt: &str) -> Option<String> {
        self.asked.push(prompt.to_owned());
        let remote = self.board.lock().unwrap().remote.clone();
        if let (Some(f), Some(r)) = (self.during_ask.as_mut(), remote.as_ref()) {
            f(r);
        }
        self.answers.pop_front()
    }

    fn pause(&mut self) {
        self.pauses += 1;
    }
}

fn file(bytes: Vec<u8>) -> Image {
    let info = uf2_inspect(bytes.clone());
    assert!(info.valid, "{}", info.problem);
    Image {
        bytes,
        info,
        name: "test.uf2".into(),
    }
}

/// An unstamped image, as every image is today.
fn plain() -> Image {
    file(support::image(RP2350, DEMO_VERSION, None))
}

fn stamped(mv: &str) -> Image {
    file(support::image(RP2350, DEMO_VERSION, Some(mv)))
}

fn usb() -> Target {
    Target::Usb { serial: None }
}

fn req(args: &[&str]) -> Request {
    let mut all = vec!["test.uf2"];
    all.extend_from_slice(args);
    Request::parse(&all).unwrap()
}

fn flash(world: &mut Fake, image: &Image, args: &[&str]) -> u8 {
    run(&usb(), &req(args), Some(image), world)
}

fn nothing_done(w: &Fake) {
    assert_eq!(w.entered(), 0, "the board was rebooted");
    assert_eq!(w.written(), None, "something was written");
}

#[test]
fn wp_tm3_a_stock_board_takes_an_image_and_comes_back_confirmed() {
    let mut w = Fake::new("stock");
    let image = plain();
    assert_eq!(flash(&mut w, &image, &["--force"]), exit::OK);
    assert_eq!(w.written().as_deref(), Some(image.bytes.as_slice()));
    assert_eq!(w.entered(), 1);
    assert!(
        w.asked.is_empty(),
        "--force skips the go-ahead: {:?}",
        w.asked
    );
    assert_eq!(w.writes(), vec![], "flashing changes no setting");
}

#[test]
fn wp_tm3_without_force_it_asks_and_no_means_nothing_happens() {
    let mut w = Fake::new("go_ahead_y").answers(&["y"]);
    assert_eq!(flash(&mut w, &plain(), &[]), exit::OK);
    assert_eq!(w.asked.len(), 1);
    assert_eq!(w.entered(), 1);

    for answer in [&["n"][..], &[""], &[]] {
        let mut w = Fake::new("go_ahead_n").answers(answer);
        assert_eq!(flash(&mut w, &plain(), &[]), exit::REJECTED, "{answer:?}");
        nothing_done(&w);
    }
}

#[test]
fn wp_tm3_force_alone_is_refused_on_an_at_risk_board() {
    let mut w = Fake::new("force_alone").at_risk().answers(&[PHRASE]);
    assert_eq!(flash(&mut w, &plain(), &["--force"]), exit::REJECTED);
    nothing_done(&w);
    assert!(
        w.asked.is_empty(),
        "the flag has to come first: {:?}",
        w.asked
    );
    assert_eq!(w.answers.len(), 1, "nothing was read");

    // Nor without --force: a yes is not the answer either.
    let mut w = Fake::new("no_flags").at_risk().answers(&["y"]);
    assert_eq!(flash(&mut w, &plain(), &[]), exit::REJECTED);
    nothing_done(&w);
    assert!(w.asked.is_empty());
}

#[test]
fn wp_tm3_the_flag_only_lets_the_question_be_asked() {
    for answer in [
        &[][..],
        &[""],
        &["y"],
        &[WORD],
        &["i accept the core voltage risk"],
        &["I ACCEPT THE CORE VOLTAGE RISK "],
        &[" I ACCEPT THE CORE VOLTAGE RISK"],
    ] {
        let mut w = Fake::new("wrong_answer").at_risk().answers(answer);
        let args = ["--force", "--accept-core-voltage-risk"];
        assert_eq!(flash(&mut w, &plain(), &args), exit::REJECTED, "{answer:?}");
        nothing_done(&w);
        assert_eq!(w.asked.len(), 1, "one go, no retries");
    }
}

#[test]
fn wp_tm3_the_flag_and_the_phrase_get_past_the_guard() {
    let mut w = Fake::new("phrase").at_risk().answers(&[PHRASE]);
    let image = plain();
    let code = flash(&mut w, &image, &["--accept-core-voltage-risk"]);
    assert_eq!(code, exit::OK);
    assert_eq!(w.written().as_deref(), Some(image.bytes.as_slice()));
    assert_eq!(w.asked.len(), 1, "the phrase is also the go-ahead");
    assert!(w.asked[0].contains(PHRASE));
}

#[test]
fn wp_tm3_an_image_stamped_high_enough_is_clear() {
    let mut w = Fake::new("high_enough").at_risk();
    assert_eq!(flash(&mut w, &stamped("1200"), &["--force"]), exit::OK);
    assert!(w.asked.is_empty());
}

#[test]
fn wp_tm3_a_raising_image_takes_the_word_and_the_phrase_does_not_stand_in() {
    let accept = ["--force", "--accept-core-voltage-risk"];

    // A stock board asks too: 1.25 V from 1.15 V.
    let mut w = Fake::new("raise_force").answers(&[WORD]);
    assert_eq!(
        flash(&mut w, &stamped("1250"), &["--force"]),
        exit::REJECTED
    );
    nothing_done(&w);

    let mut w = Fake::new("raise_phrase").answers(&[PHRASE]);
    assert_eq!(flash(&mut w, &stamped("1250"), &accept), exit::REJECTED);
    nothing_done(&w);

    let mut w = Fake::new("raise_word").answers(&[WORD]);
    assert_eq!(flash(&mut w, &stamped("1250"), &accept), exit::OK);
    assert!(w.asked[0].contains(WORD), "{:?}", w.asked);

    // The owner's example: a unit on 1.25 V, going to 1.30 V.
    let mut w = Fake::new("raise_at_risk").vreg(V1_25).answers(&[]);
    assert_eq!(
        flash(&mut w, &stamped("1300"), &["--force"]),
        exit::REJECTED
    );
    nothing_done(&w);

    // And the word does not stand in for the phrase.
    let mut w = Fake::new("lower_word").at_risk().answers(&[WORD]);
    assert_eq!(flash(&mut w, &stamped("1150"), &accept), exit::REJECTED);
    nothing_done(&w);

    // 1.20 V on a stock board is below the line.
    let mut w = Fake::new("raise_small");
    assert_eq!(flash(&mut w, &stamped("1200"), &["--force"]), exit::OK);
}

#[test]
fn wp_tm3_the_board_is_read_again_after_the_prompt() {
    // Clear when asked, at risk by the time the answer comes: the old reading
    // would have let this through.
    let mut w = Fake::new("changed_at_prompt").answers(&["y"]);
    w.during_ask = Some(Box::new(|r| r.vreg_set_build_default(V1_20)));
    assert_eq!(flash(&mut w, &plain(), &[]), exit::REJECTED);
    nothing_done(&w);
}

#[test]
fn wp_tm3_each_risk_takes_its_own_answer() {
    // 1.25 V on a 1.30 V board is too low: the phrase. Then the board reports
    // 1.20 V, so the same image is a raise, and that phrase answers nothing.
    let mut w = Fake::new("two_risks").vreg(V1_30).answers(&[PHRASE, WORD]);
    w.during_ask = Some(Box::new(|r| r.vreg_set_build_default(V1_20)));
    let image = stamped("1250");
    let args = ["--accept-core-voltage-risk"];
    assert_eq!(flash(&mut w, &image, &args), exit::OK);
    assert_eq!(w.asked.len(), 2, "{:?}", w.asked);
    assert!(w.asked[0].contains(PHRASE));
    assert!(w.asked[1].contains(WORD));

    // Answered once, not twice.
    let mut w = Fake::new("two_risks_one").vreg(V1_30).answers(&[PHRASE]);
    w.during_ask = Some(Box::new(|r| r.vreg_set_build_default(V1_20)));
    assert_eq!(flash(&mut w, &image, &args), exit::REJECTED);
    nothing_done(&w);
}

#[test]
fn wp_tm3_a_mounted_drive_counts_as_a_board_nobody_can_ask() {
    let accept = ["--force", "--accept-core-voltage-risk"];
    let opened = |w: &Fake| w.board.lock().unwrap().opened;

    // --force alone: blocked, even though a stock board would be clear.
    let mut w = Fake::new("mounted_force").already_mounted();
    assert_eq!(flash(&mut w, &plain(), &["--force"]), exit::REJECTED);
    nothing_done(&w);
    assert_eq!(opened(&w), 0, "any DSPi on the bus says nothing about it");

    // The phrase, for the plain image.
    let mut w = Fake::new("mounted_phrase")
        .already_mounted()
        .answers(&[PHRASE]);
    let image = plain();
    w.board.lock().unwrap().returns = None;
    assert_eq!(flash(&mut w, &image, &accept), exit::TRANSPORT);
    assert_eq!(w.written().as_deref(), Some(image.bytes.as_slice()));
    assert_eq!(w.entered(), 0, "it is in the bootloader already");

    let mut w = Fake::new("mounted_word").already_mounted().answers(&[WORD]);
    assert_eq!(flash(&mut w, &plain(), &accept), exit::REJECTED);
    assert_eq!(w.written(), None);

    // A raising image: the word takes the phrase's place, one answer per image.
    let mut w = Fake::new("mounted_raise")
        .already_mounted()
        .answers(&[PHRASE]);
    assert_eq!(flash(&mut w, &stamped("1250"), &accept), exit::REJECTED);
    assert_eq!(w.written(), None);
    let mut w = Fake::new("mounted_raise_word")
        .already_mounted()
        .answers(&[WORD]);
    w.board.lock().unwrap().returns = None;
    assert_eq!(flash(&mut w, &stamped("1250"), &accept), exit::TRANSPORT);
    assert!(w.written().is_some());

    // --device names a unit; the drive may be any board's.
    let mut w = Fake::new("mounted_device")
        .already_mounted()
        .answers(&[PHRASE]);
    let target = Target::Usb {
        serial: Some("ABC".into()),
    };
    let code = run(&target, &req(&accept), Some(&plain()), &mut w);
    assert_eq!(code, exit::USAGE);
    assert_eq!(w.written(), None);
    assert!(w.asked.is_empty());
}

/// A device image of another wire format: what `dspi_proto::wire_compat` maps,
/// as TM4's fixtures build them.
fn image_as(wire: u8) -> Vec<u8> {
    let v32 = FakeDevice::demo().remote().image();
    let mut d = if wire > v32[0] {
        // A newer firmware appends: the V32 prefix, then what it added.
        let mut d = v32;
        d.extend_from_slice(&[0xEE; 16]);
        d
    } else if wire >= 16 {
        v32[..dspi_proto::wire_compat::bulk_size(wire).unwrap()].to_vec()
    } else {
        // Older than anything that can be mapped.
        v32[..512].to_vec()
    };
    d[0] = wire;
    let len = d.len() as u16;
    d[6..8].copy_from_slice(&len.to_le_bytes());
    d
}

#[test]
fn wp_tm3_a_read_only_device_can_update() {
    // V17, V31 and V33 map onto this build; V15 cannot be read at all. Each is
    // when somebody needs this command, and none of them may be written to.
    for wire in [33, 31, 17, 15] {
        let name = format!("read_only_{wire}");
        let mut w = Fake::new(&name).image(image_as(wire));
        let image = plain();
        assert_eq!(flash(&mut w, &image, &["--force"]), exit::OK, "V{wire}");
        assert_eq!(w.entered(), 1, "V{wire}: the request got through the guard");
        assert_eq!(w.written().as_deref(), Some(image.bytes.as_slice()));
        assert_eq!(w.writes(), vec![], "V{wire}: nothing but the reboot");

        // And the guard still reads through the read-only one.
        let mut w = Fake::new(&format!("{name}_risk"))
            .image(image_as(wire))
            .at_risk();
        assert_eq!(
            flash(&mut w, &plain(), &["--force"]),
            exit::REJECTED,
            "V{wire}"
        );
        nothing_done(&w);
    }
}

#[test]
fn wp_tm3_the_wrong_chip_never_reaches_the_reboot() {
    let mut w = Fake::new("wrong_chip");
    let image = file(support::image(RP2040, DEMO_VERSION, None));
    assert_eq!(flash(&mut w, &image, &["--force"]), exit::USAGE);
    nothing_done(&w);

    // The drive said one thing and the file another.
    let mut w = Fake::new("wrong_chip_drive");
    mount(&w.drive(), RP2040_INFO);
    assert_eq!(flash(&mut w, &plain(), &["--force"]), exit::USAGE);
    assert_eq!(w.written(), None);

    // A drive that turns out to be another chip after the reboot.
    let mut w = Fake::new("wrong_chip_after");
    w.board.lock().unwrap().arrives = Some(RP2040_INFO);
    assert_ne!(flash(&mut w, &plain(), &["--force"]), exit::OK);
    assert_eq!(w.written(), None);
}

#[test]
fn wp_tm3_a_chip_without_a_uf2_bootloader_is_refused() {
    let mut w = Fake::new("stm32");
    {
        let mut b = w.board.lock().unwrap();
        // Both places a device says it: the platform reply and the image header.
        b.capture.platform[0] = 2;
        b.capture.bulk[1] = 2;
    }
    assert_eq!(flash(&mut w, &plain(), &["--force"]), exit::USAGE);
    nothing_done(&w);
}

#[test]
fn wp_tm3_only_usb_can_flash() {
    let link = Target::Link(LinkTarget::parse("hub.local").unwrap());
    for target in [link, Target::Midi { port: None }] {
        let mut w = Fake::new("not_usb");
        let code = run(&target, &req(&["--force"]), Some(&plain()), &mut w);
        assert_eq!(code, exit::USAGE, "{target:?}");
        nothing_done(&w);
        assert_eq!(w.board.lock().unwrap().opened, 0);
    }
    // The demo device never goes looking for a real drive.
    let mut w = Fake::new("demo_no_drive");
    let code = run(&Target::Demo, &req(&["--force"]), Some(&plain()), &mut w);
    assert_eq!(code, exit::USAGE);
    let world = &mut RealWorld {
        chosen: None,
        demo: true,
    };
    assert_eq!(world.drives(), vec![]);
}

#[test]
fn wp_tm3_no_drive_or_two_drives_write_nothing() {
    let mut w = Fake::new("no_drive");
    w.board.lock().unwrap().arrives = None;
    assert_eq!(flash(&mut w, &plain(), &["--force"]), exit::TRANSPORT);
    assert_eq!(w.entered(), 1);
    assert_eq!(w.written(), None);
    assert_eq!(w.pauses, POLLS);

    // Two appear: not a guess.
    let mut w = Fake::new("two_drives");
    let other = w.drive().with_file_name("RP2350 1");
    mount(&other, RP2350_INFO);
    w.board.lock().unwrap().extra_drive = Some(other.clone());
    let image = plain();
    // One was there first: that is a mounted drive, so it asks.
    let code = flash(&mut w, &image, &["--force"]);
    assert_eq!(code, exit::REJECTED);
    assert_eq!(w.written(), None);
    // Both there: names are needed.
    mount(&w.drive(), RP2350_INFO);
    assert_eq!(flash(&mut w, &image, &["--force"]), exit::USAGE);
    assert_eq!(w.written(), None);
    assert!(!other.join("firmware.uf2").exists());
}

#[test]
fn wp_tm3_writing_is_not_success_until_the_board_says_so() {
    // Never comes back.
    let mut w = Fake::new("never_back");
    w.board.lock().unwrap().returns = None;
    assert_eq!(flash(&mut w, &plain(), &["--force"]), exit::TRANSPORT);
    assert!(w.written().is_some());
    assert_eq!(w.pauses, POLLS);

    // Comes back on something else.
    let mut w = Fake::new("other_version");
    w.board.lock().unwrap().returns = Some("1.1.5");
    assert_eq!(flash(&mut w, &plain(), &["--force"]), exit::REJECTED);

    // An image that does not say cannot be confirmed, but the board is back.
    let mut w = Fake::new("no_version");
    let bare = file(support::image(RP2350, "", None));
    assert_eq!(bare.info.firmware_version, "");
    assert_eq!(flash(&mut w, &bare, &["--force"]), exit::OK);
}

#[test]
fn wp_tm3_after_a_mounted_drive_only_the_new_device_is_the_board() {
    let accept = ["--accept-core-voltage-risk"];
    let image = plain();

    // Another DSPi was on the bus all along, and must not be taken for it.
    let mut w = Fake::new("new_device").already_mounted().answers(&[PHRASE]);
    w.bus = vec!["OTHER"];
    w.arrived = vec!["BOARD"];
    assert_eq!(flash(&mut w, &image, &accept), exit::OK);
    let pinned = Target::Usb {
        serial: Some("BOARD".into()),
    };
    assert_eq!(w.opened_as, vec![pinned]);

    // Only that other one: not confirmed.
    let mut w = Fake::new("no_new_device")
        .already_mounted()
        .answers(&[PHRASE]);
    w.bus = vec!["OTHER"];
    assert_eq!(flash(&mut w, &image, &accept), exit::TRANSPORT);
    assert_eq!(w.opened_as, vec![]);

    // Two new ones: not a guess either.
    let mut w = Fake::new("two_new_devices")
        .already_mounted()
        .answers(&[PHRASE]);
    w.arrived = vec!["ONE", "TWO"];
    assert_eq!(flash(&mut w, &image, &accept), exit::TRANSPORT);
    assert_eq!(w.opened_as, vec![]);
}

#[test]
fn wp_tm3_dry_run_does_nothing_and_says_what_would_happen() {
    let mut w = Fake::new("dry_clear").answers(&["y"]);
    assert_eq!(flash(&mut w, &plain(), &["--dry-run"]), exit::OK);
    nothing_done(&w);
    assert!(w.asked.is_empty());

    let mut w = Fake::new("dry_risk").at_risk().answers(&[PHRASE]);
    let args = ["--dry-run", "--accept-core-voltage-risk", "--force"];
    assert_eq!(flash(&mut w, &plain(), &args), exit::REJECTED);
    nothing_done(&w);
    assert!(w.asked.is_empty());

    let mut w = Fake::new("dry_no_file");
    let dry = Request {
        dry_run: true,
        ..Request::default()
    };
    assert_eq!(run(&usb(), &dry, None, &mut w), exit::OK);
    nothing_done(&w);
}

#[test]
fn wp_tm3_no_file_only_reboots_into_the_bootloader() {
    let force = Request {
        force: true,
        ..Request::default()
    };
    let mut w = Fake::new("reboot");
    assert_eq!(run(&usb(), &force, None, &mut w), exit::OK);
    assert_eq!(w.entered(), 1);
    assert_eq!(w.written(), None);

    let mut w = Fake::new("reboot_ro").image(image_as(33));
    assert_eq!(run(&usb(), &force, None, &mut w), exit::OK);
    assert_eq!(w.entered(), 1, "read-only too");

    // It asks without --force, and an at-risk board is no reason to refuse:
    // no firmware changes here.
    let plain_req = Request::default();
    let mut w = Fake::new("reboot_no").answers(&["n"]);
    assert_eq!(run(&usb(), &plain_req, None, &mut w), exit::REJECTED);
    nothing_done(&w);
    let mut w = Fake::new("reboot_risk").at_risk();
    assert_eq!(run(&usb(), &force, None, &mut w), exit::OK);
    assert_eq!(w.entered(), 1);

    // Already there.
    let mut w = Fake::new("reboot_mounted").already_mounted();
    assert_eq!(run(&usb(), &force, None, &mut w), exit::OK);
    nothing_done(&w);
}

#[test]
fn wp_tm3_a_flag_it_does_not_know_stops_it() {
    for args in [
        &["a.uf2", "--acept-core-voltage-risk"][..],
        &["a.uf2", "--word", "OVERVOLT"],
        &[
            "a.uf2",
            "--accept-core-voltage-risk=I ACCEPT THE CORE VOLTAGE RISK",
        ],
        &["a.uf2", "-y"],
        &["a.uf2", "b.uf2"],
        &["--drive"],
    ] {
        assert!(Request::parse(args).is_err(), "{args:?}");
    }
    let parsed = Request::parse(&["--drive=/x", "a.uf2", "--force", "--json"]).unwrap();
    assert_eq!(parsed.drive, Some(PathBuf::from("/x")));
    assert_eq!(parsed.file, Some(PathBuf::from("a.uf2")));
    assert!(parsed.force && parsed.json && !parsed.accept_risk);
}

#[test]
fn wp_tm3_the_demo_device_does_not_stand_in_for_the_board() {
    let demo = |w: &mut Fake, image: &Image, args: &[&str]| {
        let mut all = vec!["--drive", "fake"];
        all.extend_from_slice(args);
        run(&Target::Demo, &req(&all), Some(image), w)
    };
    let accept = ["--force", "--accept-core-voltage-risk"];

    // An image that would be clear on the demo's own 1.20 V: asked all the same.
    let mut w = Fake::new("demo_force").at_risk().already_mounted();
    assert_eq!(demo(&mut w, &stamped("1200"), &["--force"]), exit::REJECTED);
    assert_eq!(w.written(), None);
    assert!(w.asked.is_empty());

    let mut w = Fake::new("demo_phrase")
        .already_mounted()
        .answers(&[PHRASE]);
    let image = stamped("1200");
    assert_eq!(demo(&mut w, &image, &accept), exit::OK);
    assert_eq!(w.written().as_deref(), Some(image.bytes.as_slice()));

    // A raising image still takes the word.
    let mut w = Fake::new("demo_word").already_mounted().answers(&[PHRASE]);
    assert_eq!(demo(&mut w, &stamped("1250"), &accept), exit::REJECTED);
    assert_eq!(w.written(), None);

    // No device is opened, and with no drive there is nothing to wait for.
    let mut w = Fake::new("demo_nothing");
    assert_eq!(demo(&mut w, &plain(), &accept), exit::USAGE);
    assert_eq!(w.board.lock().unwrap().opened, 0);
    nothing_done(&w);
    assert_eq!(w.pauses, 0);
}

#[test]
fn wp_tm3_a_mounted_drive_of_the_wrong_chip_is_refused_before_any_question() {
    let rp2040 = file(support::image(RP2040, DEMO_VERSION, None));
    for args in [&["--accept-core-voltage-risk"][..], &["--dry-run"]] {
        let mut w = Fake::new("chip_first").already_mounted().answers(&[PHRASE]);
        assert_eq!(flash(&mut w, &rp2040, args), exit::USAGE, "{args:?}");
        assert!(w.asked.is_empty(), "{args:?}: {:?}", w.asked);
        assert_eq!(w.written(), None);
    }
}

#[test]
fn wp_tm3_a_file_that_names_no_boot_rom_is_not_a_drive_on_its_way() {
    let mut w = Fake::new("not_boot_rom").answers(&[PHRASE]);
    mount(&w.drive(), "UF2 Bootloader v1.0\nModel: Some Other Board\n");
    let args = ["--force", "--accept-core-voltage-risk"];
    assert_eq!(flash(&mut w, &plain(), &args), exit::USAGE);
    nothing_done(&w);
    assert!(w.asked.is_empty());
}

#[test]
fn wp_tm3_only_the_line_ending_comes_off_an_answer() {
    assert_eq!(without_line_ending("x\n"), "x");
    assert_eq!(without_line_ending("x\r\n"), "x");
    assert_eq!(without_line_ending("x"), "x");
    assert_eq!(without_line_ending(" x \n"), " x ");
    assert_eq!(without_line_ending("x\n\n"), "x\n");
}
