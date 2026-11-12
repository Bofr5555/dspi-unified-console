//! A simulated DSPi: the demo device, and a stateful fixture for tests.
//!
//! Unlike [`crate::MockTransport`] (scripted replies, for the awkward paths),
//! this behaves like a device: it holds a V32 bulk image, answers the bulk
//! reads (single-shot and chunked) and the platform/version reads from it,
//! applies writes to the image, queues the `PARAM_CHANGED` notifications a
//! real device would send, and answers the extension channel (`EXT_LIST`,
//! the crossfeed pair map, the core-voltage status).
//!
//! What it answers, and where it differs from the firmware
//! (Bofr5555/DSPi `vendor_commands.c`):
//!
//! - **Identity:** `GET_PLATFORM` (the capture's reply), `GET_SERIAL`,
//!   `GET_BUILD_INFO`, `GET_STATUS` (sub-query 9 is the meter block; the
//!   others answer a `u32`, as the firmware does).
//! - **Output rings (WP CG12):** `GET_BUFFER_STATS` in the firmware's layout,
//!   with every live ring steady at the current latency target, which is a
//!   demo's picture and not measured behaviour. Sub-query 28 (lowest fill)
//!   stays at the firmware's start value, no write seen. Nothing streams.
//! - **Bulk:** `0xA0`/`0xA1` single-shot and `0xA2`/`0xA3` chunked, with the
//!   chunking spec's session rules (`bulk_params_chunking.md`): offset 0
//!   starts a session, chunks are sequential, any other request tears the
//!   session down, and an out-of-order or failed chunk stalls and closes
//!   every chunk session (vendor_commands.c:4669-4672). A bulk write is
//!   applied when it passes [`BulkPacket::decode`] and notifies
//!   `BULK_INVALIDATED` with source `BULK_SET` (`HOST_SET` with
//!   [`FakeOptions::bulk_source`]). The device keeps its own
//!   header. A wrong-version or wrong-size bulk write **stalls** here
//!   (unless [`FakeOptions::ack_unknown_sets`]); the firmware ACKs the
//!   transfer and refuses it later on the main loop, so a host sees no
//!   error there, only no change. A chunk out of sequence always stalls.
//! - **Scalars:** the GET/SET pairs in `SCALARS`, plus the matrix route,
//!   EQ and crossover bands, and channel names. A SET patches the image at
//!   the field's bulk offset and, when the bytes changed, queues
//!   `PARAM_CHANGED` with source `HOST_SET`. Offsets come from the generated
//!   section table plus the per-section layouts below (`bulk_params.h`),
//!   which the tests check against the generated section lengths. As on the
//!   firmware, a SET with an out-of-range index or a short payload is
//!   acknowledged and changes nothing; a GET with a bad index stalls. Range
//!   clamps are not modelled: a delay below 0, loudness and crossfeed
//!   limits, and the master volume's -128..0 range (a NaN or +6 dB is stored
//!   as sent). A master-volume SET also queues the legacy v1 packet
//!   `[0x01, 0, 0, 0, f32]` ahead of the `PARAM_CHANGED`, every time, as
//!   `update_master_volume` does (usb_audio.c:369-375).
//! - **Features** (`fake/features.rs`, the demo coverage): the leveller,
//!   psychoacoustic bass, subharmonic synthesizer, tube preamp, output
//!   limiter and upmixer parameters, input selection and configuration
//!   (source, I2S rate/channels/clock mode, S/PDIF and ADAT inputs), the I2S
//!   output config (output types, BCK, MCK), ADAT output, DAC hardware mute,
//!   LG Sound Sync, plus non-bulk models of the upmixer status, input-source
//!   and board names (with their `0x0F`/`0x10` notifications), the output
//!   latency option and the UART/I2C control-interface configs. With these,
//!   `dspi_session::probe` finds every feature but the signal generator and
//!   the spectrum analyser, which the next item adds.
//! - **Analysis** (with [`FakeOptions::analysis`], `fake/analysis.rs`, WP CG22):
//!   the spectrum analyser (protocol V3: caps, config, band frames one at a
//!   time and packed, the bin frame, status, control; auto-start on a read and
//!   auto-off) and the signal generator (caps, config, control, status and its
//!   notifications), with made-up but plausible levels that move with
//!   [`crate::clock`]. The generator plays only when a host starts it. Off,
//!   they stall.
//! - **Presets:** directory, names, startup, active slot, save/load/delete
//!   (in memory), the two persistence modes. A load sends `PRESET_LOADED`
//!   and `BULK_INVALIDATED` (source `PRESET`) and keeps the live master
//!   volume in the independent mode.
//! - **Extensions** (with [`FakeOptions::extensions`]): `EXT_LIST`, the pair
//!   map (validated and normalised as `crossfeed_map.h`; a write lands in
//!   the bulk image's `crossfeed.pair_map` and notifies) and the core-voltage
//!   status and requests (`core_vreg.h`). WP CG11: a voltage is what
//!   `core_vreg.c` would program, a `SET` ends a running trial, and
//!   [`VregSim`] plays hangs, reboots and power cycles as
//!   `core_vreg_boot_apply` treats them. It can also hold requests in the
//!   firmware's one staging slot until its main loop runs
//!   ([`FakeRemote::vreg_stage_requests`]).
//! - **Control surfaces** (with [`FakeOptions::surfaces`], `fake/surfaces.rs`,
//!   WP CG16): the record store, the firmware's refusal rules, save and revert,
//!   aux outputs, macros, IR learning and the display status. Off, they stall.
//! - **Everything else stalls**, in both directions, by default: a fake
//!   that stalls makes a missing model visible. The firmware ACKs every
//!   unhandled OUT request on USB except `REQ_EXTENSION`;
//!   [`FakeOptions::ack_unknown_sets`] does the same. Not modelled at all
//!   (GETs stall): the subharm headroom, pin SETs (output pins,
//!   BCK, MCK, S/PDIF/I2S/ADAT RX pins) and output-pin GETs, the I2S
//!   clock-pin mode, the S/PDIF RX, ADAT and I2S-slave status reads, the
//!   control-interface status, the Core 1 mode read, clock diagnostics, the
//!   DAC mute test pulse and the bootloader. The `SCALARS`/`FEATURE_SCALARS` tables and the per-opcode
//!   matches in `State::get_other`/`set_other`/`get_feature`/`set_feature`
//!   are the authoritative set.
//!
//! A read never returns `Ok` with fewer bytes than [`Transport::control_in`]
//! asked for: that is [`TransportError::ShortRead`]. Only
//! [`Transport::control_in_upto`] returns a short reply. A request longer
//! than [`FakeOptions::max_transfer`] fails before reaching the device, as
//! WinUSB refuses one over 4096 bytes.
//!
//! **Closing.** Dropping a [`FakeDevice`] (or [`FakeRemote::disconnect`])
//! ends its notification sources with [`TransportError::Disconnected`], as a
//! closed USB handle does. A reader that is stopped on purpose sees the same
//! error as an unplug; telling them apart is the owner's job.
//!
//! Owned by WP T4 (docs/core-merge/PLAN.md).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::clock::Instant;

use dspi_proto::ext::PairMap;
use dspi_proto::generated::{
    self, ext as gext, filter, notify as nfy, opcodes as op, status as st, wire as w,
};
use dspi_proto::packets::BuildInfo;
use dspi_proto::wire::{BulkPacket, WireHeader};

mod features;
// -- WP FWX --
mod diag;
pub use diag::{EXT_FAKE_CLIP, FAKE_HW_REV};
// -- WP CG22 --
mod analysis;
// -- WP CG16 --
mod surfaces;
pub use surfaces::{SurfaceSeed, surface_seed};
// -- WP QD6 --
mod wire_images;
// -- WP BULKFIX --
mod bulk_db;
pub use bulk_db::{BulkDbDevice, BulkDbOptions, GainProbe};

use crate::{
    DeviceDescriptor, NotificationSource, Result, Transport, TransportCaps, TransportError,
    TransportKind,
};

/// The bundled RP2350 capture (`tests/fixtures/link/device/`, see its
/// `capture.json`): `REQ_GET_PLATFORM`, `REQ_GET_ALL_PARAMS` and
/// `REQ_GET_STATUS` sub-query 9 as a v1.1.6 beta5 device answered them.
const DEMO_PLATFORM: &[u8] = include_bytes!("../../../tests/fixtures/link/device/platform.bin");
const DEMO_BULK: &[u8] = include_bytes!("../../../tests/fixtures/link/device/bulk_v32.bin");
const DEMO_STATUS: &[u8] = include_bytes!("../../../tests/fixtures/link/device/status_wvalue9.bin");

/// The beta the demo says it runs, in `GET_PLATFORM`'s seventh byte: 0, the
/// final 1.1.6 the consoles target and bundle (`setApplicationVersion` in the
/// Qt app, the `DSPi-*-v1.1.6.uf2` images), so a demo never shows a firmware
/// banner (WP CG27). The capture is beta 5, but this is no fiction about the
/// image: v1.1.6 has `FW_VERSION_BETA` 0 and `WIRE_FORMAT_VERSION` 32 with the
/// same 6136 bytes. Beta 5's one opcode (`REQ_GET_CLOCK_DIAG`) is not modelled
/// here anyway.
const DEMO_FW_BETA: u8 = 0;

/// The serial every fake reports (`GET_SERIAL` and the descriptor).
const SERIAL: &str = "DEMO0000DSPI0001";
/// `FLASH_OK`: the reply to the deferred flash commands.
const FLASH_OK: u8 = 0;

/// A captured device to start from: a `REQ_GET_PLATFORM` reply and a V32
/// bulk image. `tests/fixtures/link/device/` holds a real RP2350 capture.
#[derive(Debug, Clone)]
pub struct Capture {
    /// The `REQ_GET_PLATFORM` reply, 4 to 7 bytes.
    pub platform: Vec<u8>,
    /// The `REQ_GET_ALL_PARAMS` reply: a whole `WireBulkParams`.
    pub bulk: Vec<u8>,
}

impl Capture {
    /// The capture bundled with this crate, reporting [`DEMO_FW_BETA`]: an
    /// RP2350 on v1.1.6, 8 inputs and 9 outputs, master volume -20 dB.
    pub fn demo() -> Self {
        let mut platform = DEMO_PLATFORM.to_vec();
        if let Some(beta) = platform.get_mut(6) {
            *beta = DEMO_FW_BETA;
        }
        Self {
            platform,
            bulk: DEMO_BULK.to_vec(),
        }
    }
}

/// Which optional firmware features the fake answers for, and how strictly
/// it behaves. `Default` is a stock-firmware device that stalls what it does
/// not model, with no transfer-size cap below `wLength`'s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FakeOptions {
    /// Answer the extension channel (the Bofr5555/DSPi fork's K340 build).
    pub extensions: bool,
    /// ACK an OUT request the fake does not model or refuses, as the
    /// firmware does on USB (`REQ_EXTENSION` still stalls). Off by default,
    /// so a test sees a write the fake would silently drop. Known gaps even
    /// with it on:
    /// - with `extensions: false` the fake still stalls an OUT
    ///   `REQ_EXTENSION`, whereas firmware without the extension channel
    ///   ACKs it as an unknown plain opcode;
    /// - a `0xA1` of the wrong length over 64 bytes stalls on the firmware
    ///   (it does not fit `vendor_rx_buf[64]`) but is ACKed here;
    /// - the bulk-owner lock (a STALL while another transport owns the bulk
    ///   buffer) is not modelled.
    pub ack_unknown_sets: bool,
    /// The largest control transfer, reported by
    /// [`Transport::max_transfer`] and enforced on every request. `None` is
    /// 65535 (`wLength`'s limit, as libusb on Linux and macOS); `Some(4096)`
    /// emulates WinUSB.
    pub max_transfer: Option<usize>,
    /// Tag a bulk SET's `BULK_INVALIDATED` with its path (`HOST_SET`) and
    /// report `NOTIFY_FEAT_BULK_SOURCE` through `EXT_NOTIFY_FEATURES`, as
    /// firmware from the `bulk-source` commits does
    /// (`notification_protocol_v2_spec.md` 3.5.1). Off: every bulk apply is
    /// `BULK_SET` and the sub-command is not listed. The flag is only
    /// discoverable with [`FakeOptions::extensions`] on.
    pub bulk_source: bool,
    // -- WP CG16 --
    /// Answer the control-surface requests from a model holding the demo's
    /// configuration (`fake/surfaces.rs`). Off is firmware without the feature:
    /// they stall, and a host finds no surfaces. [`FakeDevice::demo`] has it on.
    pub surfaces: bool,
    // -- WP CG22 --
    /// Answer the spectrum analyser and the signal generator from a model
    /// (`fake/analysis.rs`). Off is firmware without them: they stall.
    /// [`FakeDevice::demo`] has it on.
    pub analysis: bool,
    // -- WP FWX --
    /// Answer `EXT_IDENTIFY` and `EXT_RESET_INFO`, report
    /// `NOTIFY_FEAT_CLIP_EVENT` and send the clip event on
    /// [`EXT_FAKE_CLIP`] (`fake/diag.rs`), as firmware from wp/FWX does. Off:
    /// bofr-main 04122d3, which has none of them. Only discoverable with
    /// [`FakeOptions::extensions`] on.
    pub diag_ext: bool,
}

/// `wLength` is 16 bits.
const MAX_W_LENGTH: usize = 65535;

/// A simulated device behind the [`Transport`] trait. See the module docs
/// for what it answers.
pub struct FakeDevice {
    descriptor: DeviceDescriptor,
    shared: Arc<Shared>,
    max_transfer: usize,
}

/// A handle on a [`FakeDevice`]'s state that stays usable after the device
/// was moved into a session or a core: a test plays the device's side with
/// it (a knob turned at the device, an unplug) and inspects the image.
#[derive(Clone)]
pub struct FakeRemote {
    shared: Arc<Shared>,
}

/// State shared by the device, its notification sources and its remotes.
struct Shared {
    state: Mutex<State>,
    /// Signalled when a packet is queued or the device goes away.
    wake: Condvar,
}

/// Packets held for the notification endpoint; the oldest goes first when
/// nobody reads.
const MAX_QUEUE: usize = 1024;

/// Live PEQ bands per channel: `MAX_BANDS` (12) is the storage depth and
/// "only bands 0..9 are active today" (config.h:880). Not a define, so not
/// generated; `probe_band_count` measures it.
const LIVE_BANDS: usize = 10;

/// The demo's build default core voltage, 1.20 V (the K340 build,
/// `DSPI_VREG_MV=1200`).
const DEMO_VREG: u8 = 0x0D;

/// `MASTER_VOLUME_MODE_INDEPENDENT` (config.h:517), not generated.
const MV_INDEPENDENT: u8 = 0;

/// `OUTPUT_CONFIG_MODE_WITH_PRESET` (config.h:531), the firmware default.
const OC_WITH_PRESET: u8 = 1;

impl FakeDevice {
    /// A device that answers from `capture`. Nothing in the capture is
    /// trusted: a short or odd image gives a device with that image (and no
    /// channels where the header is missing), never a panic.
    pub fn new(capture: Capture, options: FakeOptions) -> Self {
        let max_transfer = options.max_transfer.unwrap_or(MAX_W_LENGTH);
        Self {
            max_transfer,
            descriptor: DeviceDescriptor {
                serial: SERIAL.into(),
                bus_id: "demo".into(),
                address: 0,
            },
            shared: Arc::new(Shared {
                state: Mutex::new(State::new(capture, options)),
                wake: Condvar::new(),
            }),
        }
    }

    /// The built-in demo device: the RP2350 capture bundled with this crate,
    /// with the extension channel and control surfaces on.
    pub fn demo() -> Self {
        Self::demo_with(true)
    }

    // -- WP CG16 --
    /// [`FakeDevice::demo`] on firmware without control surfaces.
    pub fn demo_without_surfaces() -> Self {
        Self::demo_with(false)
    }

    // -- WP QD6 --
    /// The demo device as firmware of wire format `wire`
    /// ([`Capture::demo_as_wire`]), or `None` for a format without a fixture.
    pub fn demo_as_wire(wire: u8) -> Option<Self> {
        let options = FakeOptions {
            extensions: true,
            surfaces: true,
            ..FakeOptions::default()
        };
        Some(Self::new(Capture::demo_as_wire(wire)?, options))
    }

    fn demo_with(surfaces: bool) -> Self {
        let options = FakeOptions {
            extensions: true,
            surfaces,
            // -- WP CG22 --
            analysis: true,
            ..FakeOptions::default()
        };
        let dev = Self::new(Capture::demo(), options);
        {
            let mut s = dev.shared.lock();
            // The capture's own meter block; `new` synthesises a silent one.
            if DEMO_STATUS.len() == s.status_len() {
                s.status = DEMO_STATUS.to_vec();
            }
        }
        dev
    }

    /// A handle for a test to act as the device (see [`FakeRemote`]).
    pub fn remote(&self) -> FakeRemote {
        FakeRemote {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Refuse a transfer the host stack would refuse before the wire.
    fn check_size(&self, opcode: u8, len: usize) -> Result<()> {
        if len > self.max_transfer {
            return Err(TransportError::Usb(format!(
                "a {len}-byte transfer for opcode 0x{opcode:02X} exceeds the {}-byte limit",
                self.max_transfer
            )));
        }
        Ok(())
    }

    fn run<T>(&self, f: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        let mut s = self.shared.lock();
        if s.gone {
            return Err(TransportError::Disconnected);
        }
        let r = f(&mut s);
        let queued = !s.queue.is_empty();
        drop(s);
        if queued {
            self.shared.wake.notify_all();
        }
        r
    }
}

impl Drop for FakeDevice {
    /// Closing the device ends its notification stream, as a closed USB
    /// handle does.
    fn drop(&mut self) {
        self.shared.lock().gone = true;
        self.shared.wake.notify_all();
    }
}

impl FakeRemote {
    /// A change made at the device (a knob, a UART host): patch the image at
    /// `offset` and, if the bytes changed, queue `PARAM_CHANGED` with
    /// `source` (a `PARAM_SRC_*`). False, with nothing changed, when the
    /// bytes would touch the header or run off the image.
    pub fn poke(&self, offset: usize, bytes: &[u8], source: u8) -> bool {
        let ok = offset >= WireHeader::LEN && self.shared.lock().write(offset, bytes, source);
        self.shared.wake.notify_all();
        ok
    }

    /// The device's bulk image as it stands.
    pub fn image(&self) -> Vec<u8> {
        self.shared.lock().image.clone()
    }

    /// Queue a raw packet on the notification endpoint (any event, even a
    /// malformed one). A v2 packet (`[2, evt, flags, seq, ..]`) gets the
    /// device's next sequence number in byte 3, so a reader sees no loss;
    /// anything else is queued as is.
    pub fn push(&self, packet: Vec<u8>) {
        self.shared.lock().push_raw(packet);
        self.shared.wake.notify_all();
    }

    /// Replace the `GET_STATUS` sub-query 9 meter block (`peaks[n] u16,
    /// cpu0, cpu1, clip_flags u32, active_inputs`). Not checked: a test may
    /// set a short block to exercise a host's length check.
    pub fn set_status(&self, bytes: Vec<u8>) {
        self.shared.lock().status = bytes;
    }

    /// A whole-image change at the device (a preset recalled from a knob, a
    /// UART bulk write): the image becomes `bytes` exactly, header included,
    /// and `BULK_INVALIDATED` goes out with `source` (a `PARAM_SRC_*`).
    pub fn load_image(&self, bytes: Vec<u8>, source: u8) {
        {
            let mut s = self.shared.lock();
            s.image = bytes;
            s.event(nfy::NOTIFY_EVT_BULK_INVALIDATED, source);
        }
        self.shared.wake.notify_all();
    }

    /// Unplug: every later request, and the notification stream, fail with
    /// [`TransportError::Disconnected`].
    pub fn disconnect(&self) {
        self.shared.lock().gone = true;
        self.shared.wake.notify_all();
    }

    // -- WP CG11 --

    /// Play a hang, reboot, power cycle or passing time on the core-voltage
    /// state.
    pub fn vreg_sim(&self, sim: VregSim) {
        self.shared.lock().vreg_sim(sim);
    }

    /// Set the build default (`DSPI_VREG_MV`) as a boot with it would: no
    /// trial, nothing stored.
    pub fn vreg_set_build_default(&self, code: u8) {
        let mut s = self.shared.lock();
        s.vreg.default_vreg = code;
        s.vreg.stored_sel = 0;
        s.vreg.trial = None;
        s.vreg.live = code;
    }

    /// Hold core-voltage requests in the firmware's one staging slot until
    /// [`FakeRemote::vreg_run_main_loop`], as the firmware does: the status
    /// read right after a write shows the old state, and a second write
    /// replaces a first that was not applied yet.
    pub fn vreg_stage_requests(&self, on: bool) {
        self.shared.lock().vreg.stage_requests = on;
    }

    /// The main loop applies the staged core-voltage request, if any.
    pub fn vreg_run_main_loop(&self) {
        self.shared.lock().vreg_run_main_loop();
    }

    // -- WP UPW2 -- Error injection.

    /// Make every later request with `opcode`, either direction, fail with
    /// [`TransportError::Stalled`], whatever the fake would answer.
    pub fn stall(&self, opcode: u8) {
        let mut s = self.shared.lock();
        if !s.stalled.contains(&opcode) {
            s.stalled.push(opcode);
        }
    }

    /// Make every later SET with `opcode` succeed without changing anything,
    /// as the firmware ACKs a SET it drops over USB (vendor_commands.c:4761-4766),
    /// for an opcode the fake would otherwise apply. Only a read-back shows it.
    pub fn ignore(&self, opcode: u8) {
        let mut s = self.shared.lock();
        if !s.ignored.contains(&opcode) {
            s.ignored.push(opcode);
        }
    }

    /// Make the next `reads` notification reads fail at once with
    /// [`TransportError::Usb`], as a handle the host stack still holds but can
    /// no longer use does (`u32::MAX`: every read from now on). Control
    /// requests still work. Zero ends it.
    pub fn fail_notifications(&self, reads: u32) {
        self.shared.lock().notify_failures = reads;
        self.shared.wake.notify_all();
    }
}

// -- WP CG16 --
// Fake-only `REQ_EXTENSION` sub-commands. Every one is defined here and listed in
// `FAKE_ONLY_EXTENSIONS`, and `wp_cg16_fake_only_extension_codes_are_unique` holds
// them apart: the next one takes the next code.

/// `REQ_EXTENSION` OUT sub-command only a [`FakeDevice`] answers, from the
/// experimental range: a [`VregSim::encode`] payload (WP CG11). It lets a client's
/// demo device mode play hangs without a [`FakeRemote`] (QD28, AN23, SW23). A host
/// checks for [`TransportKind::Fake`] before sending it.
pub const EXT_FAKE_VREG_SIM: u16 = 0xFFF0;

/// `REQ_EXTENSION` OUT sub-command only a [`FakeDevice`] answers, as
/// [`EXT_FAKE_VREG_SIM`] (WP QD4): `[target, name..]`, stored and announced as a
/// rename by someone else (AN6, QD4, SW, TM17). `target` is an input source, or
/// [`RENAME_ELSEWHERE_DEVICE`] for the device.
pub const EXT_FAKE_RENAME_ELSEWHERE: u16 = 0xFFF1;
/// The device itself as the target of [`EXT_FAKE_RENAME_ELSEWHERE`].
pub const RENAME_ELSEWHERE_DEVICE: u8 = 0xFF;

/// `REQ_EXTENSION` sub-command only a [`FakeDevice`] answers, as
/// [`EXT_FAKE_VREG_SIM`] (WP QD5), so a client can reach the firmware's deferred
/// output-latency apply without a [`FakeRemote`]. OUT `[mode, reads, blackout]`
/// sets how the next SET lands: mode 0 at once (a SET held so far applies on the
/// next status read), 1 after `reads` status reads, 2 never, 3 dropped after
/// `reads` status reads. `blackout` status reads stall first while a SET is
/// pending. IN: the SETs received so far, a little-endian `u32`.
pub const EXT_FAKE_LATENCY_TIMING: u16 = 0xFFF2;

/// `REQ_EXTENSION` OUT sub-command only a [`FakeDevice`] answers, as
/// [`EXT_FAKE_VREG_SIM`] (WP CG16): one byte, nonzero to hold the device in a
/// running revert (`control_surfaces_busy`), zero to let it finish. While held, a
/// deferred control-surface SET settles `CS_STATUS_BUSY`, an aux SET changes
/// nothing and leaves the status alone, and the save, the revert, a macro fire and
/// a learn arm or cancel stall. It lets a client's demo show the refusals the
/// firmware has no other way to give.
pub const EXT_FAKE_CS_SIM: u16 = 0xFFF3;

// -- WP QD31 --
/// `REQ_EXTENSION` OUT sub-command only a [`FakeDevice`] answers, as
/// [`EXT_FAKE_VREG_SIM`] (WP QD31): one byte, how `REQ_SAVE_OUTPUT_CONFIG` answers
/// from now on. 0 is `PRESET_OK`, as the firmware always does; [`OUTPUT_SAVE_STALL`]
/// stalls; another byte is that status. It lets a client's demo show a failed save,
/// which the firmware's deferred flash write doesn't report on this request.
pub const EXT_FAKE_OUTPUT_SAVE_SIM: u16 = 0xFFF4;
/// [`EXT_FAKE_OUTPUT_SAVE_SIM`]'s byte for a stall: a device that doesn't answer.
pub const OUTPUT_SAVE_STALL: u8 = 0xFF;

// -- WP CG20 --
/// `REQ_EXTENSION` OUT sub-command only a [`FakeDevice`] answers, as
/// [`EXT_FAKE_VREG_SIM`] (WP CG20): `[protocol, code as u32 LE]`, the remote button an
/// armed learn was waiting for ([`FakeRemote::cs_learned`], for a client's demo
/// device). Stalls with nothing armed.
pub const EXT_FAKE_CS_LEARN: u16 = 0xFFF5;

/// Every fake-only `REQ_EXTENSION` sub-command.
pub const FAKE_ONLY_EXTENSIONS: &[u16] = &[
    EXT_FAKE_VREG_SIM,
    EXT_FAKE_RENAME_ELSEWHERE,
    EXT_FAKE_LATENCY_TIMING,
    EXT_FAKE_CS_SIM,
    // -- WP QD31 --
    EXT_FAKE_OUTPUT_SAVE_SIM,
    // -- WP CG20 --
    EXT_FAKE_CS_LEARN,
    // -- WP FWX --
    EXT_FAKE_CLIP,
];

/// What can happen to a simulated device's core-voltage state (WP CG11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VregSim {
    /// The 8 s watchdog fires. A running trial is reported as failed, with
    /// this run's whole uptime, as `core_vreg.c` does; otherwise the hang
    /// counts, and two raise the stored voltage. Uptime restarts.
    Hang,
    /// A requested reboot (`watchdog_reboot`: USB reset, BOOTSEL). Back on
    /// the persisted voltage with no failure recorded. The failed-trial code
    /// survives; its uptime, kept in RAM, does not.
    Reboot,
    /// Power lost: the scratch registers go too, so every record is gone.
    PowerCycle,
    /// This many seconds pass on the device's clock.
    Advance(u32),
}

impl VregSim {
    /// The [`EXT_FAKE_VREG_SIM`] payload.
    pub fn encode(self) -> [u8; 5] {
        let (kind, secs) = match self {
            VregSim::Hang => (0, 0),
            VregSim::Reboot => (1, 0),
            VregSim::PowerCycle => (2, 0),
            VregSim::Advance(secs) => (3, secs),
        };
        let [a, b, c, d] = secs.to_le_bytes();
        [kind, a, b, c, d]
    }

    fn decode(data: &[u8]) -> Option<Self> {
        let [kind, a, b, c, d] = *data else {
            return None;
        };
        match kind {
            0 => Some(VregSim::Hang),
            1 => Some(VregSim::Reboot),
            2 => Some(VregSim::PowerCycle),
            3 => Some(VregSim::Advance(u32::from_le_bytes([a, b, c, d]))),
            _ => None,
        }
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Every update to the state is a plain assignment between checks, so
        // a test thread that panicked while holding the lock leaves nothing
        // half-written that matters.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Transport for FakeDevice {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        let want = usize::from(len);
        self.check_size(opcode, want)?;
        let mut reply = self.run(|s| s.get(opcode, value, want))?;
        if reply.len() < want {
            return Err(TransportError::ShortRead {
                opcode,
                wanted: want,
                got: reply.len(),
            });
        }
        reply.truncate(want);
        Ok(reply)
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        self.check_size(opcode, usize::from(max_len))?;
        let mut reply = self.run(|s| s.get(opcode, value, usize::from(max_len)))?;
        reply.truncate(usize::from(max_len));
        Ok(reply)
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        self.check_size(opcode, data.len())?;
        self.run(|s| s.set(opcode, value, data))
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    fn max_transfer(&self) -> usize {
        self.max_transfer
    }

    /// The device's notification stream. Every source shares one queue, so
    /// two readers split the packets between them, as two readers of one
    /// USB endpoint would. Ends with `Disconnected` once the device is
    /// dropped or unplugged.
    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        Some(Box::new(FakeNotifications {
            shared: Arc::clone(&self.shared),
        }))
    }

    fn caps(&self) -> TransportCaps {
        TransportCaps {
            kind: TransportKind::Fake,
            chunked_bulk: true,
            remote: false,
        }
    }
}

/// The fake's notification endpoint.
struct FakeNotifications {
    shared: Arc<Shared>,
}

impl NotificationSource for FakeNotifications {
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let start = Instant::now();
        let mut s = self.shared.lock();
        loop {
            if s.gone {
                return Err(TransportError::Disconnected);
            }
            // -- WP UPW2 --
            if s.notify_failures > 0 {
                s.notify_failures = s.notify_failures.saturating_sub(1);
                return Err(TransportError::Usb("injected notification fault".into()));
            }
            if let Some(p) = s.queue.pop_front() {
                return Ok(p);
            }
            let left = timeout.saturating_sub(start.elapsed());
            if left.is_zero() {
                // Silence: the endpoint NAKs until the timeout.
                return Ok(Vec::new());
            }
            s = match self.shared.wake.wait_timeout(s, left) {
                Ok((g, _)) => g,
                Err(p) => p.into_inner().0,
            };
        }
    }
}

// ------------------------------------------------------------------ layouts

/// `(name, offset from the start of its struct, length)`, as the
/// `*_FIELDS` tables in `dspi_proto::wire`. The tests check each struct's
/// fields are contiguous and fill the generated section.
type Field = (&'static str, usize, usize);

// `WireGlobalParams` (bulk_params.h:60-67).
const G_PREAMP: Field = ("preamp_gain_db", 0, 4);
const G_BYPASS: Field = ("bypass", 4, 1);
const G_LOUDNESS: Field = ("loudness_enabled", 5, 1);
const G_LOUDNESS_MASK: Field = ("loudness_output_mask", 6, 2);
const G_LOUDNESS_REF: Field = ("loudness_ref_spl", 8, 4);
const G_LOUDNESS_INT: Field = ("loudness_intensity_pct", 12, 4);

// `WireCrossfeedParams` (bulk_params.h:72-82).
const X_ENABLED: Field = ("enabled", 0, 1);
const X_PRESET: Field = ("preset", 1, 1);
const X_ITD: Field = ("itd_enabled", 2, 1);
const X_PAIR_MASK: Field = ("output_pair_mask", 3, 1);
const X_FC: Field = ("custom_fc", 4, 4);
const X_FEED: Field = ("custom_feed_db", 8, 4);
const X_PAIR_MAP: Field = ("pair_map", 12, 4);

// `WireOutputChannel` (bulk_params.h:115-121), one per output.
const O_ENABLED: Field = ("enabled", 0, 1);
const O_MUTE: Field = ("mute", 1, 1);
// Layout only: checked by the tests, never written.
#[cfg(test)]
const O_RESERVED: Field = ("reserved", 2, 2);
const O_GAIN: Field = ("gain_db", 4, 4);
const O_DELAY: Field = ("delay_ms", 8, 4);
const OUTPUT_LEN: usize = 12;

// `WireCrosspoint` (bulk_params.h:105-110), `[input][WIRE_MAX_OUTPUT_CHANNELS]`.
const C_ENABLED: Field = ("enabled", 0, 1);
const C_PHASE: Field = ("phase_invert", 1, 1);
// Layout only: checked by the tests, never written.
#[cfg(test)]
const C_RESERVED: Field = ("reserved", 2, 2);
const C_GAIN: Field = ("gain_db", 4, 4);
const CROSSPOINT_LEN: usize = 8;

// `WireMasterVolume` (bulk_params.h:198-201).
const MV_DB: Field = ("master_volume_db", 0, 4);
// Layout only: checked by the tests, never written.
#[cfg(test)]
const MV_RESERVED: Field = ("reserved", 4, 12);

// `WireUserVolume` (bulk_params.h:274-278).
const UV_DB: Field = ("user_volume_db", 0, 4);
const UV_MUTE: Field = ("user_mute", 4, 1);
// Layout only: checked by the tests, never written.
#[cfg(test)]
const UV_RESERVED: Field = ("reserved", 5, 11);

// `WireBandParams` (bulk_params.h:136-145), `eq[ch][WIRE_MAX_BANDS]` and
// `crossovers[ch][WIRE_MAX_XOVER_BANDS]`.
const B_TYPE: Field = ("type", 0, 1);
const B_BYPASS: Field = ("bypass", 1, 1);
const B_QP: Field = ("qp", 2, 2);
const B_FREQ: Field = ("freq", 4, 4);
const B_Q: Field = ("q", 8, 4);
const B_GAIN: Field = ("gain_db", 12, 4);
const BAND_LEN: usize = 16;

/// One `float` per channel (`WireChannelDelays`) or per input
/// (`WirePreampConfig`).
const F32: Field = ("value", 0, 4);

/// `(offset, len)` of a section in the generated table.
fn section(name: &str) -> Option<(usize, usize)> {
    generated::SECTIONS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, off, len)| (*off, *len))
}

/// Field `f` of a record: `rec[f.1 .. f.1 + f.2]`.
fn field_of(rec: &[u8], f: Field) -> Option<&[u8]> {
    rec.get(f.1..f.1.checked_add(f.2)?)
}

/// What a scalar's `wValue` indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Index {
    None,
    /// `0 .. num_input_channels`.
    Input,
    /// `0 .. num_output_channels`.
    Output,
    /// `0 .. num_channels`.
    Channel,
    /// An S/PDIF/I2S output slot (a stereo pair), `0 .. num_outputs / 2`.
    Spdif,
}

/// How the firmware stores a SET's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Store {
    /// As sent.
    Raw,
    /// `!= 0` stored as 0/1.
    Bool,
    /// One byte; a value above the maximum is ignored (no change).
    Max(u8),
    /// One byte, clamped to the maximum.
    Clamp(u8),
}

impl Store {
    /// The bytes to store for a SET's `bytes`; `None` ignores the SET.
    fn apply(self, bytes: &[u8]) -> Option<Vec<u8>> {
        let first = bytes.first().copied().unwrap_or(0);
        match self {
            Store::Raw => Some(bytes.to_vec()),
            Store::Bool => Some(vec![u8::from(bytes.iter().any(|b| *b != 0))]),
            Store::Max(m) => (first <= m).then(|| vec![first]),
            Store::Clamp(m) => Some(vec![first.min(m)]),
        }
    }
}

/// Every plain GET/SET pair: the base table and the demo coverage's.
fn all_scalars() -> impl Iterator<Item = &'static Scalar> {
    SCALARS.iter().chain(features::FEATURE_SCALARS)
}

/// A parameter with a plain GET/SET pair whose value is one bulk field.
struct Scalar {
    set: u8,
    get: u8,
    section: &'static str,
    field: Field,
    /// Record size when `index` is not `None`.
    stride: usize,
    index: Index,
    store: Store,
}

const fn scalar(set: u8, get: u8, section: &'static str, field: Field, store: Store) -> Scalar {
    Scalar {
        set,
        get,
        section,
        field,
        stride: 0,
        index: Index::None,
        store,
    }
}

const fn indexed(
    (set, get): (u8, u8),
    section: &'static str,
    field: Field,
    stride: usize,
    index: Index,
    store: Store,
) -> Scalar {
    Scalar {
        set,
        get,
        section,
        field,
        stride,
        index,
        store,
    }
}

/// The scalar parameters, with the bulk field each one lives in (the
/// firmware's `notify_param_write` offsets in `vendor_commands.c`).
const SCALARS: &[Scalar] = &[
    scalar(
        op::REQ_SET_MASTER_VOLUME,
        op::REQ_GET_MASTER_VOLUME,
        "master_volume",
        MV_DB,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_USER_VOLUME,
        op::REQ_GET_USER_VOLUME,
        "user_volume",
        UV_DB,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_USER_MUTE,
        op::REQ_GET_USER_MUTE,
        "user_volume",
        UV_MUTE,
        Store::Bool,
    ),
    scalar(
        op::REQ_SET_BYPASS,
        op::REQ_GET_BYPASS,
        "global",
        G_BYPASS,
        Store::Bool,
    ),
    scalar(
        op::REQ_SET_LOUDNESS,
        op::REQ_GET_LOUDNESS,
        "global",
        G_LOUDNESS,
        Store::Bool,
    ),
    scalar(
        op::REQ_SET_LOUDNESS_MASK,
        op::REQ_GET_LOUDNESS_MASK,
        "global",
        G_LOUDNESS_MASK,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_LOUDNESS_REF,
        op::REQ_GET_LOUDNESS_REF,
        "global",
        G_LOUDNESS_REF,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_LOUDNESS_INTENSITY,
        op::REQ_GET_LOUDNESS_INTENSITY,
        "global",
        G_LOUDNESS_INT,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_CROSSFEED,
        op::REQ_GET_CROSSFEED,
        "crossfeed",
        X_ENABLED,
        Store::Bool,
    ),
    scalar(
        op::REQ_SET_CROSSFEED_PRESET,
        op::REQ_GET_CROSSFEED_PRESET,
        "crossfeed",
        X_PRESET,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_CROSSFEED_ITD,
        op::REQ_GET_CROSSFEED_ITD,
        "crossfeed",
        X_ITD,
        Store::Bool,
    ),
    scalar(
        op::REQ_SET_CROSSFEED_OUTPUTS,
        op::REQ_GET_CROSSFEED_OUTPUTS,
        "crossfeed",
        X_PAIR_MASK,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_CROSSFEED_FREQ,
        op::REQ_GET_CROSSFEED_FREQ,
        "crossfeed",
        X_FC,
        Store::Raw,
    ),
    scalar(
        op::REQ_SET_CROSSFEED_FEED,
        op::REQ_GET_CROSSFEED_FEED,
        "crossfeed",
        X_FEED,
        Store::Raw,
    ),
    indexed(
        (op::REQ_SET_PREAMP_CH, op::REQ_GET_PREAMP_CH),
        "preamp",
        F32,
        4,
        Index::Input,
        Store::Raw,
    ),
    indexed(
        (op::REQ_SET_DELAY, op::REQ_GET_DELAY),
        "delays",
        F32,
        4,
        Index::Channel,
        Store::Raw,
    ),
    indexed(
        (op::REQ_SET_OUTPUT_ENABLE, op::REQ_GET_OUTPUT_ENABLE),
        "outputs",
        O_ENABLED,
        OUTPUT_LEN,
        Index::Output,
        Store::Bool,
    ),
    indexed(
        (op::REQ_SET_OUTPUT_MUTE, op::REQ_GET_OUTPUT_MUTE),
        "outputs",
        O_MUTE,
        OUTPUT_LEN,
        Index::Output,
        Store::Raw,
    ),
    indexed(
        (op::REQ_SET_OUTPUT_GAIN, op::REQ_GET_OUTPUT_GAIN),
        "outputs",
        O_GAIN,
        OUTPUT_LEN,
        Index::Output,
        Store::Raw,
    ),
    indexed(
        (op::REQ_SET_OUTPUT_DELAY, op::REQ_GET_OUTPUT_DELAY),
        "outputs",
        O_DELAY,
        OUTPUT_LEN,
        Index::Output,
        Store::Raw,
    ),
];

// -------------------------------------------------------------------- state

/// An open chunked bulk transfer (`bulk_params_chunking.md`).
enum Chunked {
    None,
    /// A read: the image as it was at offset 0, and the next offset.
    Get {
        snapshot: Vec<u8>,
        next: usize,
    },
    /// A write: what arrived so far, from offset 0.
    Set {
        buf: Vec<u8>,
    },
}

/// The core-voltage state behind `EXT_CORE_VREG` (core_vreg.h).
struct Vreg {
    live: u8,
    stored_sel: u8,
    default_vreg: u8,
    flags: u8,
    hang_count: u8,
    trial: Option<u8>,
    failed_trial: u8,
    failed_trial_uptime_s: u32,
    // -- WP CG11 --
    /// Seconds added to the real clock since the last boot ([`VregSim::Advance`]).
    uptime_offset_s: u32,
    /// Hold requests for the "main loop", as `core_vreg_req_*` does.
    stage_requests: bool,
    /// The one staging slot: a later request overwrites an unapplied one.
    staged: Option<(u8, u8)>,
}

const NAME_LEN: usize = st::PRESET_NAME_LEN as usize;
const SLOTS: usize = st::PRESET_SLOTS as usize;

/// The preset store (flash_storage.c's directory, in memory).
struct Presets {
    names: Vec<[u8; NAME_LEN]>,
    /// The bulk image saved in each slot.
    slots: Vec<Option<Vec<u8>>>,
    active: u8,
    startup_mode: u8,
    default_slot: u8,
    output_config_mode: u8,
    master_volume_mode: u8,
    // -- WP UPW2 --
    /// The directory's independent master volume, in dB
    /// (`REQ_SAVE_MASTER_VOLUME`, `REQ_GET_SAVED_MASTER_VOLUME`).
    saved_master_volume: [u8; 4],
}

struct State {
    platform: Vec<u8>,
    image: Vec<u8>,
    /// What `REQ_FACTORY_RESET` restores.
    factory: Vec<u8>,
    /// `GET_STATUS` sub-query 9.
    status: Vec<u8>,
    options: FakeOptions,
    chunked: Chunked,
    presets: Presets,
    vreg: Vreg,
    /// The demo coverage's non-bulk state (names, latency, control
    /// interfaces).
    extras: features::Extras,
    // -- WP CG12 --
    /// `BufferStatsPacket.sequence`: counts the reads, wrapping.
    buffer_stats_sequence: u16,
    // -- WP CG16 --
    /// `None`: this firmware has no control surfaces.
    surfaces: Option<surfaces::Surfaces>,
    // -- WP CG22 --
    /// `None`: this firmware has no analyser or generator.
    analysis: Option<analysis::Analysis>,
    // -- WP QD31 --
    /// [`EXT_FAKE_OUTPUT_SAVE_SIM`]'s byte.
    output_save_sim: u8,
    // -- WP UPW2 --
    /// `subharm_config.solo`: monitoring only, never in the image.
    subharm_solo: bool,
    // -- WP FWX --
    /// Identify's blink and last tone (`fake/diag.rs`).
    diag: diag::Diag,
    /// Opcodes [`FakeRemote::stall`] made fail.
    stalled: Vec<u8>,
    /// SETs [`FakeRemote::ignore`] made ACK and drop.
    ignored: Vec<u8>,
    /// Notification reads [`FakeRemote::fail_notifications`] still fails.
    notify_failures: u32,
    queue: VecDeque<Vec<u8>>,
    seq: u8,
    started: Instant,
    /// Unplugged or closed.
    gone: bool,
}

impl State {
    fn new(capture: Capture, options: FakeOptions) -> Self {
        let mut presets = Presets {
            names: vec![[0; NAME_LEN]; SLOTS],
            slots: vec![None; SLOTS],
            active: 0,
            startup_mode: st::PRESET_STARTUP_SPECIFIED as u8,
            default_slot: 0,
            output_config_mode: OC_WITH_PRESET,
            master_volume_mode: MV_INDEPENDENT,
            // -- WP UPW2 -- Set from the image below.
            saved_master_volume: [0; 4],
        };
        // Slot 0 holds what the device is running.
        if let (Some(name), Some(slot)) = (presets.names.first_mut(), presets.slots.first_mut()) {
            put_name(name, b"Demo");
            *slot = Some(capture.bulk.clone());
        }
        let mut s = Self {
            // -- WP CG16 --
            surfaces: options.surfaces.then(surfaces::Surfaces::new),
            // -- WP CG22 --
            analysis: options.analysis.then(analysis::Analysis::new),
            platform: capture.platform,
            factory: capture.bulk.clone(),
            image: capture.bulk,
            status: Vec::new(),
            options,
            chunked: Chunked::None,
            presets,
            extras: features::Extras::new(),
            // -- WP CG12 --
            buffer_stats_sequence: 0,
            vreg: Vreg {
                live: DEMO_VREG,
                stored_sel: 0,
                default_vreg: DEMO_VREG,
                flags: 0,
                hang_count: 0,
                trial: None,
                failed_trial: 0,
                failed_trial_uptime_s: 0,
                // -- WP CG11 --
                uptime_offset_s: 0,
                stage_requests: false,
                staged: None,
            },
            // -- WP QD31 --
            output_save_sim: st::PRESET_OK as u8,
            // -- WP UPW2 --
            subharm_solo: false,
            // -- WP FWX --
            diag: diag::Diag::default(),
            stalled: Vec::new(),
            ignored: Vec::new(),
            notify_failures: 0,
            queue: VecDeque::new(),
            seq: 0,
            started: Instant::now(),
            gone: false,
        };
        // A silent meter block of the right size, with two live inputs.
        let mut status = vec![0; s.status_len()];
        if let Some(last) = status.last_mut() {
            *last = s.num_inputs().min(2);
        }
        s.status = status;
        // -- WP UPW2 --
        if let Some(live) = s.live_master_volume() {
            s.presets.saved_master_volume = live;
        }
        s
    }

    // -- WP UPW2 --
    /// The master volume in the image, as its four wire bytes.
    fn live_master_volume(&self) -> Option<[u8; 4]> {
        self.locate("master_volume", 0, 0, MV_DB)
            .and_then(|o| self.bytes(o, MV_DB.2))
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
    }

    fn header_byte(&self, i: usize) -> u8 {
        self.image.get(i).copied().unwrap_or(0)
    }

    fn num_channels(&self) -> u8 {
        self.header_byte(2)
    }

    fn num_outputs(&self) -> u8 {
        self.header_byte(3)
    }

    fn num_inputs(&self) -> u8 {
        self.header_byte(4)
    }

    /// `peaks[num_channels] u16, cpu0, cpu1, clip_flags u32, active_inputs`.
    fn status_len(&self) -> usize {
        usize::from(self.num_channels())
            .saturating_mul(2)
            .saturating_add(7)
    }

    fn bound(&self, index: Index) -> usize {
        usize::from(match index {
            Index::None => 1,
            Index::Input => self.num_inputs(),
            Index::Output => self.num_outputs(),
            Index::Channel => self.num_channels(),
            Index::Spdif => self.num_outputs() / 2,
        })
    }

    /// The absolute offset of field `field` in record `i` of a section with
    /// `stride`-byte records. `None` when the field would leave the section
    /// or the image.
    fn locate(&self, section_name: &str, stride: usize, i: usize, field: Field) -> Option<usize> {
        let (base, len) = section(section_name)?;
        let (_, rel, flen) = field;
        let in_section = i.checked_mul(stride)?.checked_add(rel)?;
        if in_section.checked_add(flen)? > len {
            return None;
        }
        let off = base.checked_add(in_section)?;
        (off.checked_add(flen)? <= self.image.len()).then_some(off)
    }

    fn scalar_at(&self, s: &Scalar, value: u16) -> Option<usize> {
        let i = match s.index {
            Index::None => 0,
            // The firmware takes the index from the low byte.
            _ => usize::from(value & 0xFF),
        };
        if i >= self.bound(s.index) {
            return None;
        }
        self.locate(s.section, s.stride, i, s.field)
    }

    fn bytes(&self, off: usize, len: usize) -> Option<Vec<u8>> {
        self.image
            .get(off..off.checked_add(len)?)
            .map(<[u8]>::to_vec)
    }

    /// Patch the image and, if anything changed, queue `PARAM_CHANGED`.
    /// False, with nothing changed, when the bytes do not fit the image.
    fn write(&mut self, off: usize, bytes: &[u8], source: u8) -> bool {
        let (Ok(off16), Ok(len16)) = (u16::try_from(off), u16::try_from(bytes.len())) else {
            return false;
        };
        let Some(dst) = off
            .checked_add(bytes.len())
            .and_then(|end| self.image.get_mut(off..end))
        else {
            return false;
        };
        if dst == bytes {
            // The firmware notifies against its shadow: no change, no packet.
            return true;
        }
        dst.copy_from_slice(bytes);
        let mut p = vec![
            nfy::NOTIFY_V2_VERSION as u8,
            nfy::NOTIFY_EVT_PARAM_CHANGED as u8,
            0,
            0,
        ];
        p.extend_from_slice(&off16.to_le_bytes());
        p.extend_from_slice(&len16.to_le_bytes());
        p.extend_from_slice(&[source, 0, 0, 0]);
        p.extend_from_slice(bytes);
        self.push(p);
        true
    }

    /// Queue a v2 packet, stamping its sequence byte. The oldest packet goes
    /// when the queue is full.
    fn push(&mut self, mut packet: Vec<u8>) {
        if let Some(seq) = packet.get_mut(3) {
            *seq = self.seq;
        }
        self.seq = self.seq.wrapping_add(1);
        self.enqueue(packet);
    }

    /// [`State::push`] for a v2 packet; anything else (the v1 volume packet,
    /// a malformed one) goes out as is and takes no sequence number.
    fn push_raw(&mut self, packet: Vec<u8>) {
        let v2 = packet.len() >= 4 && packet.first() == Some(&(nfy::NOTIFY_V2_VERSION as u8));
        if v2 {
            self.push(packet);
        } else {
            self.enqueue(packet);
        }
    }

    fn enqueue(&mut self, packet: Vec<u8>) {
        if self.queue.len() >= MAX_QUEUE {
            self.queue.pop_front();
        }
        self.queue.push_back(packet);
    }

    /// An 8-byte v2 event with one argument byte.
    fn event(&mut self, event: u16, arg: u8) {
        self.push(vec![
            nfy::NOTIFY_V2_VERSION as u8,
            event as u8,
            0,
            0,
            arg,
            0,
            0,
            0,
        ]);
    }

    /// Take `body` as the new image, except the header (the device's own
    /// shape) and the `keep` sections, which keep their live bytes.
    fn replace_body(&mut self, body: &[u8], keep: &[&str]) {
        let mut next = body.to_vec();
        let n = next.len().min(self.image.len());
        for name in keep.iter().copied().chain(["header"]) {
            if let Some((off, len)) = section(name) {
                let end = off.saturating_add(len).min(n);
                if let (Some(dst), Some(src)) = (next.get_mut(off..end), self.image.get(off..end)) {
                    dst.copy_from_slice(src);
                }
            }
        }
        if let (Some(dst), Some(src)) = (self.image.get_mut(..n), next.get(..n)) {
            dst.copy_from_slice(src);
        }
    }

    /// `global.preamp_gain_db` reads as input 0's preamp in the bulk image
    /// (bulk_params.c:129); the firmware sends no notification for it.
    fn sync_legacy_preamp(&mut self) {
        let db = self
            .locate("preamp", 4, 0, F32)
            .and_then(|o| self.bytes(o, F32.2));
        if let (Some(db), Some(off)) = (db, self.locate("global", 0, 0, G_PREAMP))
            && let Some(dst) = self.image.get_mut(off..off.saturating_add(G_PREAMP.2))
        {
            dst.copy_from_slice(&db);
        }
    }

    /// An output's delay is also its channel's delay, and the firmware
    /// notifies both (vendor_commands.c, `REQ_SET_OUTPUT_DELAY`).
    fn mirror_output_delay(&mut self, value: u16, bytes: &[u8]) {
        let channel = usize::from(self.num_inputs()).saturating_add(usize::from(value & 0xFF));
        if let Some(off) = self.locate("delays", 4, channel, F32) {
            self.write(off, bytes, nfy::PARAM_SRC_HOST_SET as u8);
        }
    }

    /// `REQ_SAVE_PARAMS`: the live state goes to the active slot. A status
    /// byte back; the flash write is deferred on the device.
    fn save_params(&mut self) -> u8 {
        let image = self.image.clone();
        if let Some(s) = self.presets.slots.get_mut(usize::from(self.presets.active)) {
            *s = Some(image);
        }
        FLASH_OK
    }

    /// `REQ_FACTORY_RESET`: the live state becomes the factory image (here,
    /// the capture the device started from); presets stay. Answers
    /// `FLASH_OK`, then invalidates with source `FACTORY`.
    fn factory_reset(&mut self) -> u8 {
        let factory = self.factory.clone();
        self.replace_body(&factory, &[]);
        // -- WP CG22 --
        self.analysis_on_preset();
        self.event(
            nfy::NOTIFY_EVT_BULK_INVALIDATED,
            nfy::PARAM_SRC_FACTORY as u8,
        );
        FLASH_OK
    }

    /// `bulk_params_apply`: exactly this wire version and size, or refused.
    fn apply_bulk(&mut self, payload: Vec<u8>) -> bool {
        match BulkPacket::decode(payload) {
            Ok(p) => {
                self.replace_body(p.as_bytes(), &[]);
                let source = if self.options.bulk_source {
                    nfy::PARAM_SRC_HOST_SET
                } else {
                    nfy::PARAM_SRC_BULK_SET
                };
                self.event(nfy::NOTIFY_EVT_BULK_INVALIDATED, source as u8);
                true
            }
            Err(_) => false,
        }
    }

    // ------------------------------------------------------------- IN

    /// The full reply to an IN request; the caller truncates to `wLength`.
    fn get(&mut self, opcode: u8, value: u16, want: usize) -> Result<Vec<u8>> {
        // -- WP UPW2 --
        if self.stalled.contains(&opcode) {
            return Err(TransportError::Stalled { opcode });
        }
        if opcode != op::REQ_GET_ALL_PARAMS_CHUNK {
            self.chunked = Chunked::None;
        }
        // -- WP CG16 --
        if let Some(r) = self.get_surface(opcode, value) {
            return r.ok_or(TransportError::Stalled { opcode });
        }
        // -- WP CG22 --
        if let Some(r) = self.get_analysis(opcode, value) {
            return r.ok_or(TransportError::Stalled { opcode });
        }
        if let Some(r) = self.get_feature(opcode, value) {
            return r.ok_or(TransportError::Stalled { opcode });
        }
        let reply = if let Some(s) = all_scalars().find(|s| s.get == opcode) {
            self.scalar_at(s, value)
                .and_then(|o| self.bytes(o, s.field.2))
        } else {
            self.get_other(opcode, value, want)
        };
        reply.ok_or(TransportError::Stalled { opcode })
    }

    fn get_other(&mut self, opcode: u8, value: u16, want: usize) -> Option<Vec<u8>> {
        match opcode {
            op::REQ_GET_PLATFORM => Some(self.platform.clone()),
            op::REQ_GET_SERIAL => Some(SERIAL.as_bytes().to_vec()),
            op::REQ_GET_BUILD_INFO => Some(
                BuildInfo {
                    describe: "dspi-fake".into(),
                    date: "2026-11-06".into(),
                }
                .encode()
                .to_vec(),
            ),
            op::REQ_GET_STATUS => Some(self.get_status(value)),
            // -- WP QD6 -- The firmware answers with the flags it cleared; the capture
            // has none latched that this models
            op::REQ_CLEAR_CLIPS => Some(vec![0; 4]),
            // -- WP CG12 --
            op::REQ_GET_BUFFER_STATS => self.get_buffer_stats(),
            // -- WP TM16 --
            // `REQ_RESET_BUFFER_STATS` (vendor_commands.c:3422-3430): an ACK.
            // The fake's watermarks sit at the fill, and its rings have never
            // been written to, so there is nothing for it to clear.
            op::REQ_RESET_BUFFER_STATS => Some(vec![1]),
            op::REQ_GET_ALL_PARAMS => Some(self.image.clone()),
            op::REQ_GET_ALL_PARAMS_CHUNK => self.get_chunk(usize::from(value), want),
            // Legacy: input 0's preamp.
            op::REQ_GET_PREAMP => self
                .locate("preamp", 4, 0, F32)
                .and_then(|o| self.bytes(o, F32.2)),
            op::REQ_GET_MATRIX_ROUTE => self.get_route(value),
            op::REQ_GET_EQ_PARAM => self.get_eq(value),
            op::REQ_GET_CHANNEL_NAME => self
                .channel_name_at(value)
                .and_then(|o| self.bytes(o, usize::from(w::WIRE_NAME_LEN))),
            op::REQ_PRESET_GET_DIR => Some(self.preset_dir()),
            op::REQ_PRESET_GET_NAME => self
                .presets
                .names
                .get(usize::from(value & 0xFF))
                .map(|n| n.to_vec()),
            op::REQ_PRESET_GET_ACTIVE => Some(vec![self.presets.active]),
            op::REQ_PRESET_GET_STARTUP => Some(vec![
                self.presets.startup_mode,
                self.presets.default_slot,
                self.presets.active,
            ]),
            op::REQ_GET_OUTPUT_CONFIG_MODE => Some(vec![self.presets.output_config_mode]),
            op::REQ_GET_MASTER_VOLUME_MODE => Some(vec![self.presets.master_volume_mode]),
            // Write-as-read: the slot in wValue, a status byte back.
            op::REQ_PRESET_SAVE => Some(vec![self.preset_save(value)]),
            op::REQ_PRESET_LOAD => Some(vec![self.preset_load(value)]),
            op::REQ_PRESET_DELETE => Some(vec![self.preset_delete(value)]),
            op::REQ_SAVE_PARAMS => Some(vec![self.save_params()]),
            // -- WP QD31 --
            // Write-as-read: PRESET_OK at once, the flash write deferred to the main
            // loop (vendor_commands.c, `case REQ_SAVE_OUTPUT_CONFIG`). There is no
            // flash behind it here: nothing a client reads changes.
            op::REQ_SAVE_OUTPUT_CONFIG => match self.output_save_sim {
                OUTPUT_SAVE_STALL => None,
                status => Some(vec![status]),
            },
            op::REQ_FACTORY_RESET => Some(vec![self.factory_reset()]),
            // No PDM/EQ-worker interlock is modelled: never a conflict.
            op::REQ_GET_CORE1_CONFLICT => Some(vec![0]),
            // -- WP UPW2 --
            // Write-as-read: PRESET_OK, the flash write deferred to the main loop
            // (vendor_commands.c, `case REQ_SAVE_MASTER_VOLUME`). Here the live
            // master volume becomes the saved one at once.
            op::REQ_SAVE_MASTER_VOLUME => {
                if let Some(live) = self.live_master_volume() {
                    self.presets.saved_master_volume = live;
                }
                Some(vec![st::PRESET_OK as u8])
            }
            op::REQ_GET_SAVED_MASTER_VOLUME => Some(self.presets.saved_master_volume.to_vec()),
            op::REQ_GET_SUBHARM_SOLO => Some(vec![u8::from(self.subharm_solo)]),
            // `wValue = (channel << 8) | band`: 1 when the band's bypass byte is
            // exactly 1, as `REQ_SET_BAND_BYPASS` stores it.
            op::REQ_GET_BAND_BYPASS => {
                let [band, ch] = value.to_le_bytes();
                let (off, _) = self.band_at(ch, band)?;
                let b = self.bytes(off.checked_add(B_BYPASS.1)?, 1)?;
                Some(vec![u8::from(b.first() == Some(&1))])
            }
            // Silent meters on the firmware's scale, two bytes per output.
            op::REQ_GET_SUBHARM_METER => {
                Some(vec![0; usize::from(self.num_outputs()).saturating_mul(2)])
            }
            // TinyUSB keeps no error counters: the firmware answers six zero
            // `u32`s, and an ACK to the reset (vendor_commands.c,
            // `case REQ_GET_USB_ERROR_STATS`).
            op::REQ_GET_USB_ERROR_STATS => Some(vec![0; 24]),
            op::REQ_RESET_USB_ERROR_STATS => Some(vec![1]),
            op::REQ_EXTENSION if self.options.extensions => self.get_ext(value),
            _ => None,
        }
    }

    /// `REQ_GET_STATUS`: sub-query 9 is the meter block; the others are the
    /// firmware's `u32` counters and readings, here quiet and plausible.
    fn get_status(&self, value: u16) -> Vec<u8> {
        if value == 9 {
            return self.status.clone();
        }
        let v: u32 = match value {
            13 => 307_200_000,
            14 => dspi_proto::ext::vreg_mv(self.vreg.live).map_or(0, u32::from),
            15 => 48_000,
            16 => 3_500,
            23 => u32::from(self.status.last().copied().unwrap_or(0)),
            // -- WP CG12 --
            // `output_ring_min_prewrite` before its first write
            // (audio_pipeline.c:1856, vendor_commands.c:2570).
            28 => u32::MAX,
            _ => 0,
        };
        v.to_le_bytes().to_vec()
    }

    // -- WP CG12 --
    /// `REQ_GET_BUFFER_STATS`: `BufferStatsPacket` (config.h:1165-1194), 44
    /// bytes, filled as vendor_commands.c:3378-3405 does, from a fill at the
    /// current target (`REQ_OUTPUT_LATENCY`). The bucket mapping is
    /// `get_slot_consumer_stats` (audio_pipeline.c:1764-1783). The watermarks
    /// sit at the fill. `dspi_proto`'s packet has no `fill_centi_pct`, so the
    /// bytes are laid out here.
    fn get_buffer_stats(&mut self) -> Option<Vec<u8>> {
        /// `SLOT_FILL_FRAMES_PER_BUCKET`, `SLOT_FILL_MAX_BUCKETS`.
        const BUCKET_FRAMES: u32 = 48;
        const BUCKETS: u32 = 16;
        let status = self.get_feature(op::REQ_OUTPUT_LATENCY, 0)??;
        let fill = u32::from(u16::from_le_bytes([*status.get(4)?, *status.get(5)?]));
        let buckets = (fill / BUCKET_FRAMES).min(BUCKETS);
        let pct = u8::try_from(buckets.saturating_mul(100) / BUCKETS).ok()?;
        let centi = u16::try_from((fill * 10_000 / (BUCKET_FRAMES * BUCKETS)).min(65_535)).ok()?;
        let [centi0, centi1] = centi.to_le_bytes();
        let prepared = u8::try_from(buckets).ok()?;
        let free = u8::try_from(BUCKETS.saturating_sub(buckets)).ok()?;
        let entry = [free, prepared, 1, pct, pct, pct, centi0, centi1];
        // NUM_SPDIF_INSTANCES (config.h:804-807): 4 with 9 outputs, 2 with 5.
        let live = usize::from((self.num_outputs() / 2).min(4));

        let sequence = self.buffer_stats_sequence;
        self.buffer_stats_sequence = sequence.wrapping_add(1);
        // Not streaming, no PDM: flags 0, and the PDM entry stays zero.
        let mut b = vec![u8::try_from(live).ok()?, 0];
        b.extend(sequence.to_le_bytes());
        for slot in 0..4 {
            b.extend(if slot < live { entry } else { [0; 8] });
        }
        b.extend([0; 8]);
        Some(b)
    }

    fn get_chunk(&mut self, off: usize, want: usize) -> Option<Vec<u8>> {
        if off == 0 {
            // Offset 0 snapshots the image, so every chunk is from one state.
            self.chunked = Chunked::Get {
                snapshot: self.image.clone(),
                next: 0,
            };
        }
        let Chunked::Get { snapshot, next } = &mut self.chunked else {
            // A later offset with no read open (or with a write open) fails
            // and closes every chunk session (vendor_commands.c:4669-4672).
            self.chunked = Chunked::None;
            return None;
        };
        if off != *next || off >= snapshot.len() {
            self.chunked = Chunked::None;
            return None;
        }
        // The device clamps a chunk that runs past the end.
        let end = off.saturating_add(want).min(snapshot.len());
        let part = snapshot.get(off..end).map(<[u8]>::to_vec);
        *next = end;
        if end == snapshot.len() {
            self.chunked = Chunked::None;
        }
        part
    }

    /// `wValue = (input << 8) | output`; a `MatrixRoutePacket` back.
    fn get_route(&self, value: u16) -> Option<Vec<u8>> {
        let [output, input] = value.to_le_bytes();
        let rec = self.bytes(self.route_at(input, output)?, CROSSPOINT_LEN)?;
        let mut r = vec![input, output];
        r.extend_from_slice(field_of(&rec, C_ENABLED)?);
        r.extend_from_slice(field_of(&rec, C_PHASE)?);
        r.extend_from_slice(field_of(&rec, C_GAIN)?);
        Some(r)
    }

    fn route_at(&self, input: u8, output: u8) -> Option<usize> {
        if input >= self.num_inputs() || output >= self.num_outputs() {
            return None;
        }
        let i = usize::from(input)
            .checked_mul(usize::from(w::WIRE_MAX_OUTPUT_CHANNELS))?
            .checked_add(usize::from(output))?;
        self.locate(
            "crosspoints",
            CROSSPOINT_LEN,
            i,
            ("record", 0, CROSSPOINT_LEN),
        )
    }

    /// Where band `band` of channel `ch` lives: a PEQ band in `eq` (true),
    /// or a crossover band (`XOVER_BAND_BASE..`, outputs only) in
    /// `crossovers` (false).
    fn band_at(&self, ch: u8, band: u8) -> Option<(usize, bool)> {
        if ch >= self.num_channels() {
            return None;
        }
        let (ch, band) = (usize::from(ch), usize::from(band));
        let rec = ("band", 0, BAND_LEN);
        if band < LIVE_BANDS {
            let i = ch
                .checked_mul(usize::from(w::WIRE_MAX_BANDS))?
                .checked_add(band)?;
            return Some((self.locate("eq", BAND_LEN, i, rec)?, true));
        }
        let x = band.checked_sub(usize::from(filter::XOVER_BAND_BASE))?;
        if x >= usize::from(filter::MAX_XOVER_BANDS) || ch < usize::from(self.num_inputs()) {
            return None;
        }
        let i = ch
            .checked_mul(usize::from(w::WIRE_MAX_XOVER_BANDS))?
            .checked_add(x)?;
        Some((self.locate("crossovers", BAND_LEN, i, rec)?, false))
    }

    /// `wValue = (channel << 8) | (band << 3) | param`; four bytes back.
    fn get_eq(&self, value: u16) -> Option<Vec<u8>> {
        let [lo, ch] = value.to_le_bytes();
        let (band, param) = (lo.checked_shr(3)?, lo & 0x07);
        let (off, peq) = self.band_at(ch, band)?;
        let b = self.bytes(off, BAND_LEN)?;
        let byte = |f: Field| field_of(&b, f).and_then(|v| v.first().copied());
        let v: Vec<u8> = match param {
            0 => u32::from(byte(B_TYPE)?).to_le_bytes().to_vec(),
            1 => field_of(&b, B_FREQ)?.to_vec(),
            2 => field_of(&b, B_Q)?.to_vec(),
            3 => field_of(&b, B_GAIN)?.to_vec(),
            4 => u32::from(byte(B_BYPASS)? == 1).to_le_bytes().to_vec(),
            5 if peq => {
                let mut qp = field_of(&b, B_QP)?.to_vec();
                qp.extend_from_slice(&[0, 0]);
                qp
            }
            _ => vec![0; 4],
        };
        Some(v)
    }

    fn channel_name_at(&self, value: u16) -> Option<usize> {
        let ch = value & 0xFF;
        if ch >= u16::from(self.num_channels()) {
            return None;
        }
        let len = usize::from(w::WIRE_NAME_LEN);
        self.locate("channel_names", len, usize::from(ch), ("name", 0, len))
    }

    /// `REQ_PRESET_GET_DIR`, 7 bytes (`PresetDirectory`).
    fn preset_dir(&self) -> Vec<u8> {
        let occupied = self
            .presets
            .slots
            .iter()
            .zip(0u32..)
            .filter(|(s, _)| s.is_some())
            .fold(0u16, |m, (_, i)| m | 1u16.checked_shl(i).unwrap_or(0));
        let [lo, hi] = occupied.to_le_bytes();
        let p = &self.presets;
        vec![
            lo,
            hi,
            p.startup_mode,
            p.default_slot,
            p.active,
            p.output_config_mode,
            p.master_volume_mode,
        ]
    }

    /// The slot in `wValue`'s low byte, if it exists.
    fn preset_slot(value: u16) -> Option<(usize, u8)> {
        let slot = u8::try_from(value & 0xFF).ok()?;
        (usize::from(slot) < SLOTS).then_some((usize::from(slot), slot))
    }

    fn preset_save(&mut self, value: u16) -> u8 {
        let Some((i, slot)) = Self::preset_slot(value) else {
            return st::PRESET_ERR_INVALID_SLOT as u8;
        };
        if let Some(s) = self.presets.slots.get_mut(i) {
            *s = Some(self.image.clone());
        }
        self.presets.active = slot;
        st::PRESET_OK as u8
    }

    fn preset_load(&mut self, value: u16) -> u8 {
        let Some((i, slot)) = Self::preset_slot(value) else {
            return st::PRESET_ERR_INVALID_SLOT as u8;
        };
        // The reply only says "accepted"; an empty slot changes nothing here.
        if let Some(Some(body)) = self.presets.slots.get(i).cloned() {
            let keep: &[&str] = if self.presets.master_volume_mode == MV_INDEPENDENT {
                &["master_volume"]
            } else {
                &[]
            };
            self.replace_body(&body, keep);
            self.presets.active = slot;
            // -- WP CG22 --
            self.analysis_on_preset();
            self.event(nfy::NOTIFY_EVT_PRESET_LOADED, slot);
            self.event(
                nfy::NOTIFY_EVT_BULK_INVALIDATED,
                nfy::PARAM_SRC_PRESET as u8,
            );
        }
        st::PRESET_OK as u8
    }

    fn preset_delete(&mut self, value: u16) -> u8 {
        let Some((i, _)) = Self::preset_slot(value) else {
            return st::PRESET_ERR_INVALID_SLOT as u8;
        };
        if let Some(s) = self.presets.slots.get_mut(i) {
            *s = None;
        }
        if let Some(n) = self.presets.names.get_mut(i) {
            *n = [0; NAME_LEN];
        }
        st::PRESET_OK as u8
    }

    fn get_ext(&self, value: u16) -> Option<Vec<u8>> {
        match value {
            gext::EXT_LIST => {
                let mut ids = vec![gext::EXT_CROSSFEED_PAIR_MAP, gext::EXT_CORE_VREG];
                // -- WP FWX --
                if self.options.bulk_source || self.options.diag_ext {
                    ids.push(gext::EXT_NOTIFY_FEATURES);
                }
                if self.options.diag_ext {
                    ids.extend([gext::EXT_IDENTIFY, gext::EXT_RESET_INFO]);
                }
                Some(ids.iter().flat_map(|id| id.to_le_bytes()).collect())
            }
            // `NOTIFY_FEAT_BULK_SOURCE` (bit 0), `NOTIFY_FEAT_CLIP_EVENT` (bit 1).
            gext::EXT_NOTIFY_FEATURES if self.options.bulk_source || self.options.diag_ext => {
                Some(self.notify_features().to_le_bytes().to_vec())
            }
            // -- WP FWX --
            gext::EXT_IDENTIFY | gext::EXT_RESET_INFO => self.get_diag(value),
            gext::EXT_CROSSFEED_PAIR_MAP => self
                .locate("crossfeed", 0, 0, X_PAIR_MAP)
                .and_then(|o| self.bytes(o, X_PAIR_MAP.2)),
            gext::EXT_CORE_VREG => Some(self.vreg_status()),
            // -- WP QD5 --
            EXT_FAKE_LATENCY_TIMING => Some(self.latency_sets().to_le_bytes().to_vec()),
            _ => None,
        }
    }

    /// `CoreVregStatus`, 16 bytes (extension_channel_spec.md 2.3).
    fn vreg_status(&self) -> Vec<u8> {
        let v = &self.vreg;
        let uptime = self.vreg_uptime_s();
        let mut b = vec![
            v.live,
            v.stored_sel,
            v.default_vreg,
            v.flags,
            v.hang_count,
            u8::from(v.trial.is_some()),
            v.trial.unwrap_or(0),
            v.failed_trial,
        ];
        b.extend_from_slice(&v.failed_trial_uptime_s.to_le_bytes());
        b.extend_from_slice(&uptime.to_le_bytes());
        b
    }

    // ------------------------------------------------------------ OUT

    fn set(&mut self, opcode: u8, value: u16, data: &[u8]) -> Result<()> {
        // -- WP UPW2 --
        if self.stalled.contains(&opcode) {
            return Err(TransportError::Stalled { opcode });
        }
        if self.ignored.contains(&opcode) {
            return Ok(());
        }
        if opcode != op::REQ_SET_ALL_PARAMS_CHUNK {
            self.chunked = Chunked::None;
        }
        // -- WP CG16 --
        let ok = if let Some(ok) = self.set_surface(opcode, value, data) {
            ok
        // -- WP CG22 --
        } else if let Some(ok) = self.set_analysis(opcode, data) {
            ok
        } else if let Some(ok) = self.set_feature(opcode, value, data) {
            ok
        } else if let Some(s) = all_scalars().find(|s| s.set == opcode) {
            if let (Some(off), Some(bytes)) = (self.scalar_at(s, value), data.get(..s.field.2))
                && let Some(bytes) = s.store.apply(bytes)
            {
                if opcode == op::REQ_SET_MASTER_VOLUME {
                    // `update_master_volume` sends the v1 packet on every
                    // call, then the v2 one if the value changed
                    // (usb_audio.c:369-375).
                    let mut v1 = vec![nfy::NOTIFY_EVT_MASTER_VOLUME as u8, 0, 0, 0];
                    v1.extend_from_slice(&bytes);
                    self.enqueue(v1);
                }
                self.write(off, &bytes, nfy::PARAM_SRC_HOST_SET as u8);
                match opcode {
                    op::REQ_SET_PREAMP_CH => self.sync_legacy_preamp(),
                    op::REQ_SET_OUTPUT_DELAY => self.mirror_output_delay(value, &bytes),
                    _ => {}
                }
            }
            true
        } else {
            match self.set_other(opcode, value, data) {
                Some(ok) => ok,
                // A chunk out of sequence stalls at the setup stage, whatever
                // the options (bulk_params_chunking.md).
                None => return Err(TransportError::Stalled { opcode }),
            }
        };
        // Over USB the firmware ACKs every SET it refuses or does not know,
        // except the extension channel's (vendor_commands.c:4761-4766); only
        // UART, I2C and MIDI answer ERROR. The fake stalls a refusal unless
        // `ack_unknown_sets` asks for the firmware's behaviour.
        if ok || (opcode != op::REQ_EXTENSION && self.options.ack_unknown_sets) {
            Ok(())
        } else {
            Err(TransportError::Stalled { opcode })
        }
    }

    /// `Some(false)` refuses the request (ACKed or stalled: see
    /// [`State::set`]); `None` stalls it in any case.
    fn set_other(&mut self, opcode: u8, value: u16, data: &[u8]) -> Option<bool> {
        let host = nfy::PARAM_SRC_HOST_SET as u8;
        Some(match opcode {
            // Refused: ACKed and dropped on USB, ERROR on UART/I2C/MIDI;
            // stalls here unless ack_unknown_sets.
            op::REQ_SET_ALL_PARAMS => self.apply_bulk(data.to_vec()),
            op::REQ_SET_ALL_PARAMS_CHUNK => return self.set_chunk(usize::from(value), data),
            op::REQ_SET_PREAMP => {
                // Legacy: every input at once.
                if let Some(db) = data.get(..F32.2) {
                    for i in 0..usize::from(self.num_inputs()) {
                        if let Some(off) = self.locate("preamp", 4, i, F32) {
                            self.write(off, db, host);
                        }
                    }
                    self.sync_legacy_preamp();
                }
                true
            }
            op::REQ_SET_MATRIX_ROUTE => {
                // `MatrixRoutePacket`: input, output, enabled, phase, gain.
                if let [input, output, en, ph, g0, g1, g2, g3, ..] = *data
                    && let Some(off) = self.route_at(input, output)
                {
                    let rec = [u8::from(en != 0), u8::from(ph != 0), 0, 0, g0, g1, g2, g3];
                    self.write(off, &rec, host);
                }
                true
            }
            op::REQ_SET_EQ_PARAM => {
                self.set_eq(data);
                true
            }
            op::REQ_SET_BAND_BYPASS => {
                // `wValue = (channel << 8) | band`, one byte. A band that does
                // not exist is ignored, as in `set_eq`.
                let [band, ch] = value.to_le_bytes();
                if let (Some(v), Some((off, _))) = (data.first(), self.band_at(ch, band))
                    && let Some(at) = off.checked_add(B_BYPASS.1)
                {
                    // The firmware's rule: exactly 1 bypasses.
                    self.write(at, &[u8::from(*v == 1)], host);
                }
                true
            }
            op::REQ_SET_CHANNEL_NAME => {
                if !data.is_empty()
                    && let Some(off) = self.channel_name_at(value)
                {
                    let mut name = [0u8; NAME_LEN];
                    put_name(&mut name, data);
                    self.write(off, &name, host);
                }
                true
            }
            op::REQ_PRESET_SET_NAME => {
                if !data.is_empty()
                    && let Some(name) = self.presets.names.get_mut(usize::from(value & 0xFF))
                {
                    put_name(name, data);
                }
                true
            }
            op::REQ_PRESET_SET_STARTUP => {
                if let [mode, slot, ..] = *data {
                    self.presets.startup_mode = mode;
                    self.presets.default_slot = slot;
                }
                true
            }
            op::REQ_SET_OUTPUT_CONFIG_MODE => {
                if let Some(m) = data.first() {
                    self.presets.output_config_mode = *m;
                }
                true
            }
            op::REQ_SET_MASTER_VOLUME_MODE => {
                if let Some(m) = data.first() {
                    self.presets.master_volume_mode = *m;
                }
                true
            }
            op::REQ_EXTENSION if self.options.extensions => self.set_ext(value, data),
            // -- WP UPW2 -- Monitoring only: no wire offset, no notification.
            op::REQ_SET_SUBHARM_SOLO => {
                if let Some(b) = data.first() {
                    self.subharm_solo = *b != 0;
                }
                true
            }
            // Unknown: see `State::set` for ACK versus stall.
            _ => false,
        })
    }

    /// `None`: a chunk out of sequence (always a stall); `Some(false)`: the
    /// complete image was refused.
    fn set_chunk(&mut self, off: usize, data: &[u8]) -> Option<bool> {
        if off == 0 {
            self.chunked = Chunked::Set { buf: Vec::new() };
        }
        let Chunked::Set { buf } = &mut self.chunked else {
            self.chunked = Chunked::None;
            return None;
        };
        let fits = buf
            .len()
            .checked_add(data.len())
            .is_some_and(|end| end <= generated::BULK_SIZE);
        if off != buf.len() || !fits {
            self.chunked = Chunked::None;
            return None;
        }
        buf.extend_from_slice(data);
        if buf.len() < generated::BULK_SIZE {
            return Some(true);
        }
        // The last byte landed: apply, as `0xA1` would.
        match std::mem::replace(&mut self.chunked, Chunked::None) {
            Chunked::Set { buf } => Some(self.apply_bulk(buf)),
            _ => None,
        }
    }

    /// An `EqParamPacket` (channel, band, type, bypass, freq, Q, gain: 16
    /// bytes), plus the optional `qp` (`Q * 512`, 2 bytes).
    fn set_eq(&mut self, data: &[u8]) {
        let Some(&[ch, band, ftype, bypass, ref floats @ ..]) = data.get(..16) else {
            return;
        };
        let Some((off, peq)) = self.band_at(ch, band) else {
            return;
        };
        let Some(old) = self.bytes(off, BAND_LEN) else {
            return;
        };
        let qp: &[u8] = match (data.get(16..18), peq) {
            (Some(q), _) => q,
            // Keep the stored PEQ value; crossover bands carry none.
            (None, true) => field_of(&old, B_QP).unwrap_or(&[0, 0]),
            (None, false) => &[0, 0],
        };
        let mut rec = Vec::with_capacity(BAND_LEN);
        rec.push(ftype);
        // The firmware's rule: exactly 1 bypasses.
        rec.push(u8::from(bypass == 1));
        rec.extend_from_slice(qp);
        rec.extend_from_slice(floats);
        self.write(off, &rec, nfy::PARAM_SRC_HOST_SET as u8);
    }

    /// False stalls the request.
    fn set_ext(&mut self, value: u16, data: &[u8]) -> bool {
        // -- WP FWX --
        if let Some(ok) = self.set_diag(value, data) {
            return ok;
        }
        match value {
            gext::EXT_CROSSFEED_PAIR_MAP => {
                let Ok(map) = <[u8; PairMap::SIZE]>::try_from(data).map(PairMap) else {
                    return false;
                };
                // `NUM_SPDIF_INSTANCES`: the stereo outputs, without the PDM sub.
                let pairs = usize::from(self.num_outputs().checked_div(2).unwrap_or(0));
                // An invalid map is acknowledged and ignored.
                if map.is_valid(pairs.min(PairMap::PAIRS))
                    && let Some(off) = self.locate("crossfeed", 0, 0, X_PAIR_MAP)
                {
                    self.write(off, &map.normalized().0, nfy::PARAM_SRC_HOST_SET as u8);
                }
                true
            }
            gext::EXT_CORE_VREG => match *data {
                [code, v] => self.vreg_request(code, v),
                _ => false,
            },
            // -- WP CG11 --
            EXT_FAKE_VREG_SIM => match VregSim::decode(data) {
                Some(sim) => {
                    self.vreg_sim(sim);
                    true
                }
                None => false,
            },
            // -- WP CG16 --
            EXT_FAKE_CS_SIM => match (data, self.surfaces.as_mut()) {
                ([on], Some(s)) => {
                    s.set_busy(*on != 0);
                    true
                }
                _ => false,
            },
            // -- WP CG20 --
            EXT_FAKE_CS_LEARN => match *data {
                [protocol, a, b, c, d] => {
                    self.cs_learned(protocol, u32::from_le_bytes([a, b, c, d]))
                }
                _ => false,
            },
            // -- WP QD4 --
            EXT_FAKE_RENAME_ELSEWHERE => match data.split_first() {
                Some((&target, name)) => self.rename_elsewhere(target, name),
                None => false,
            },
            // -- WP QD5 --
            EXT_FAKE_LATENCY_TIMING => self.latency_timing(data),
            // -- WP QD31 --
            EXT_FAKE_OUTPUT_SAVE_SIM => match *data {
                [byte] => {
                    self.output_save_sim = byte;
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// `{op, value}` (extension_channel_spec.md 2.3). False refuses it
    /// (`core_vreg_request_valid`); a valid one is applied at once, or staged
    /// for the main loop with [`FakeRemote::vreg_stage_requests`].
    fn vreg_request(&mut self, code: u8, v: u8) -> bool {
        let r = &mut self.vreg;
        let valid = match code {
            0 => v == 0 || (r.default_vreg..=0x0F).contains(&v),
            1 => (0x0C..=0x0F).contains(&v),
            2 | 4 => true,
            3 => v <= 1,
            _ => false,
        };
        if !valid {
            return false;
        }
        if r.stage_requests {
            r.staged = Some((code, v));
        } else {
            self.vreg_apply(code, v);
        }
        true
    }

    /// `core_vreg_apply_request`, for a request already validated.
    fn vreg_apply(&mut self, code: u8, v: u8) {
        use dspi_proto::ext::CoreVregStatus as S;
        let r = &mut self.vreg;
        match code {
            0 => {
                r.stored_sel = v;
                r.flags &= !S::FLAG_AUTO_RAISED;
                r.hang_count = 0;
                // An explicit choice ends a trial (`core_vreg.c`, OP_SET).
                r.trial = None;
                r.live = r.resolved();
            }
            1 => {
                r.trial = Some(v);
                r.live = v;
            }
            2 => {
                r.trial = None;
                r.live = r.resolved();
            }
            3 => {
                if v == 0 {
                    r.flags |= S::FLAG_AUTO_DISABLED;
                } else {
                    r.flags &= !S::FLAG_AUTO_DISABLED;
                }
            }
            4 => {
                r.hang_count = 0;
                r.failed_trial = 0;
                r.failed_trial_uptime_s = 0;
            }
            _ => {}
        }
    }

    // -- WP CG11 --

    fn vreg_uptime_s(&self) -> u32 {
        u32::try_from(self.started.elapsed().as_secs())
            .unwrap_or(u32::MAX)
            .saturating_add(self.vreg.uptime_offset_s)
    }

    /// The firmware's main loop reaches the staged request, if any.
    fn vreg_run_main_loop(&mut self) {
        if let Some((code, v)) = self.vreg.staged.take() {
            self.vreg_apply(code, v);
        }
    }

    fn vreg_sim(&mut self, sim: VregSim) {
        match sim {
            VregSim::Advance(secs) => {
                self.vreg.uptime_offset_s = self.vreg.uptime_offset_s.saturating_add(secs);
            }
            VregSim::Hang => self.vreg_boot(true, false),
            VregSim::Reboot => self.vreg_boot(false, false),
            VregSim::PowerCycle => self.vreg_boot(false, true),
        }
    }

    /// `core_vreg_boot_apply` after a reset: `watchdog` for our own 8 s
    /// watchdog firing, `power_on` when the scratch registers were lost.
    fn vreg_boot(&mut self, watchdog: bool, power_on: bool) {
        use dspi_proto::ext::CoreVregStatus as S;
        let prev_uptime = self.vreg_uptime_s();
        let r = &mut self.vreg;
        let prev_trial = r.trial.take();
        // RAM: lost on every reset. The staged request goes with it.
        r.staged = None;
        r.failed_trial_uptime_s = 0;
        if power_on {
            // SCRATCH0: the count and the failed-trial code live here.
            r.hang_count = 0;
            r.failed_trial = 0;
        }
        if watchdog {
            if let Some(trial) = prev_trial {
                r.failed_trial = trial;
                r.failed_trial_uptime_s = prev_uptime;
            } else {
                r.hang_count = r.hang_count.saturating_add(1).min(0xF);
            }
        }
        let mut v = r.resolved();
        if r.flags & S::FLAG_AUTO_DISABLED == 0 && r.hang_count >= 2 && v < 0x0E {
            v = v.saturating_add(1);
            r.stored_sel = v;
            r.flags |= S::FLAG_AUTO_RAISED;
            // `core_vreg_persist` clears it once the raise is written.
            r.hang_count = 0;
        }
        r.live = v;
        r.uptime_offset_s = 0;
        self.started = Instant::now();
    }
}

impl Vreg {
    /// `resolve()`: the voltage a stored selection runs at.
    fn resolved(&self) -> u8 {
        let v = if self.stored_sel == 0 {
            self.default_vreg
        } else {
            self.stored_sel
        };
        v.max(self.default_vreg).clamp(0x0C, 0x0F)
    }
}

/// Copy a name the firmware's way: at most `NAME_LEN - 1` bytes, NUL padded.
fn put_name(dst: &mut [u8; NAME_LEN], src: &[u8]) {
    *dst = [0; NAME_LEN];
    let n = src.len().min(NAME_LEN.saturating_sub(1));
    if let (Some(d), Some(s)) = (dst.get_mut(..n), src.get(..n)) {
        d.copy_from_slice(s);
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_t4 {
    use super::*;
    use dspi_proto::generated::wire::WIRE_FORMAT_VERSION;

    pub(super) fn f32_at(b: &[u8], o: usize) -> f32 {
        f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    }

    pub(super) fn off_of(name: &str) -> usize {
        section(name).unwrap().0
    }

    /// Every packet queued so far, without waiting.
    pub(super) fn drain(src: &mut Box<dyn NotificationSource>) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let p = src.read(Duration::ZERO).unwrap();
            if p.is_empty() {
                return out;
            }
            out.push(p);
        }
    }

    fn read_chunked(d: &mut FakeDevice, chunk: usize) -> Vec<u8> {
        let mut buf = Vec::new();
        while buf.len() < generated::BULK_SIZE {
            let want = chunk.min(generated::BULK_SIZE - buf.len());
            let part = d
                .control_in(op::REQ_GET_ALL_PARAMS_CHUNK, buf.len() as u16, want as u16)
                .unwrap();
            buf.extend_from_slice(&part);
        }
        buf
    }

    // ---- layouts against the generated sections ------------------------

    pub(super) fn assert_contiguous(what: &str, fields: &[Field], len: usize) {
        let mut at = 0;
        for (name, off, flen) in fields {
            assert_eq!(
                *off, at,
                "{what}.{name} is not where the previous field ends"
            );
            at += flen;
        }
        assert_eq!(at, len, "{what} does not fill its {len} bytes");
    }

    #[test]
    fn wp_t4_layouts_fill_their_generated_sections() {
        let len = |s: &str| section(s).unwrap().1;
        let (ins, outs, chans) = (
            usize::from(w::WIRE_MAX_INPUT_CHANNELS),
            usize::from(w::WIRE_MAX_OUTPUT_CHANNELS),
            usize::from(w::WIRE_MAX_CHANNELS),
        );
        let global = [
            G_PREAMP,
            G_BYPASS,
            G_LOUDNESS,
            G_LOUDNESS_MASK,
            G_LOUDNESS_REF,
            G_LOUDNESS_INT,
        ];
        assert_contiguous("global", &global, len("global"));
        let crossfeed = [
            X_ENABLED,
            X_PRESET,
            X_ITD,
            X_PAIR_MASK,
            X_FC,
            X_FEED,
            X_PAIR_MAP,
        ];
        assert_contiguous("crossfeed", &crossfeed, len("crossfeed"));
        assert_contiguous("master_volume", &[MV_DB, MV_RESERVED], len("master_volume"));
        assert_contiguous(
            "user_volume",
            &[UV_DB, UV_MUTE, UV_RESERVED],
            len("user_volume"),
        );
        assert_contiguous(
            "output",
            &[O_ENABLED, O_MUTE, O_RESERVED, O_GAIN, O_DELAY],
            OUTPUT_LEN,
        );
        assert_eq!(OUTPUT_LEN * outs, len("outputs"));
        assert_contiguous(
            "crosspoint",
            &[C_ENABLED, C_PHASE, C_RESERVED, C_GAIN],
            CROSSPOINT_LEN,
        );
        assert_eq!(CROSSPOINT_LEN * ins * outs, len("crosspoints"));
        assert_contiguous(
            "band",
            &[B_TYPE, B_BYPASS, B_QP, B_FREQ, B_Q, B_GAIN],
            BAND_LEN,
        );
        assert_eq!(BAND_LEN * chans * usize::from(w::WIRE_MAX_BANDS), len("eq"));
        assert_eq!(
            BAND_LEN * chans * usize::from(w::WIRE_MAX_XOVER_BANDS),
            len("crossovers")
        );
        assert_eq!(F32.2 * chans, len("delays"));
        assert_eq!(F32.2 * ins, len("preamp"));
        assert_eq!(usize::from(w::WIRE_NAME_LEN) * chans, len("channel_names"));
        // The pair map's bulk offset, as the extension spec states it.
        assert_eq!(off_of("crossfeed") + X_PAIR_MAP.1, 44);
        for s in SCALARS {
            let (_, slen) = section(s.section).unwrap();
            let room = if s.index == Index::None {
                slen
            } else {
                s.stride
            };
            assert!(s.field.1 + s.field.2 <= room, "{}.{}", s.section, s.field.0);
            assert!(s.index == Index::None || s.stride > 0);
        }
    }

    // ---- identity and bulk ----------------------------------------------

    #[test]
    fn wp_t4_the_demo_reads_back_its_capture() {
        let mut d = FakeDevice::demo();
        let plat = d.control_in_upto(op::REQ_GET_PLATFORM, 0, 7).unwrap();
        assert_eq!(plat.get(..6), DEMO_PLATFORM.get(..6));
        assert_eq!(plat.get(6), Some(&DEMO_FW_BETA));
        assert_eq!(
            d.control_in(op::REQ_GET_ALL_PARAMS, 0, generated::BULK_SIZE as u16)
                .unwrap(),
            DEMO_BULK
        );
        assert_eq!(read_chunked(&mut d, 1024), DEMO_BULK);
        assert_eq!(read_chunked(&mut d, 4096), DEMO_BULK);
        assert_eq!(
            d.control_in(op::REQ_GET_STATUS, 9, 41).unwrap(),
            DEMO_STATUS
        );
        let bi = d.control_in(op::REQ_GET_BUILD_INFO, 0, 64).unwrap();
        assert_eq!(BuildInfo::decode(&bi).unwrap().describe, "dspi-fake");
        assert_eq!(
            d.control_in(op::REQ_GET_SERIAL, 0, 16).unwrap(),
            SERIAL.as_bytes()
        );
        let h = WireHeader::decode(&DEMO_BULK[..16]).unwrap();
        assert_eq!(h.format_version, WIRE_FORMAT_VERSION as u8);
        assert_eq!(d.caps().kind, TransportKind::Fake);
    }

    #[test]
    fn wp_t4_control_in_never_returns_short() {
        let mut d = FakeDevice::demo();
        assert!(matches!(
            d.control_in(op::REQ_GET_PLATFORM, 0, 64),
            Err(TransportError::ShortRead {
                wanted: 64,
                got: 7,
                ..
            })
        ));
        // The device clamps a chunk past the end; `control_in` refuses it.
        d.control_in(op::REQ_GET_ALL_PARAMS_CHUNK, 0, 6000).unwrap();
        assert!(matches!(
            d.control_in(op::REQ_GET_ALL_PARAMS_CHUNK, 6000, 1000),
            Err(TransportError::ShortRead { got: 136, .. })
        ));
        // `control_in_upto` takes it.
        d.control_in(op::REQ_GET_ALL_PARAMS_CHUNK, 0, 6000).unwrap();
        let tail = d
            .control_in_upto(op::REQ_GET_ALL_PARAMS_CHUNK, 6000, 1000)
            .unwrap();
        assert_eq!(tail, &DEMO_BULK[6000..]);
    }

    #[test]
    fn wp_t4_chunked_reads_follow_the_session_rules() {
        let mut d = FakeDevice::demo();
        let chunk =
            |d: &mut FakeDevice, off: u16| d.control_in(op::REQ_GET_ALL_PARAMS_CHUNK, off, 1024);
        // No session: a later offset stalls.
        assert!(matches!(
            chunk(&mut d, 1024),
            Err(TransportError::Stalled { .. })
        ));
        // Out of order stalls and ends the session.
        chunk(&mut d, 0).unwrap();
        assert!(chunk(&mut d, 2048).is_err());
        assert!(chunk(&mut d, 1024).is_err());
        // Another request tears an open session down.
        chunk(&mut d, 0).unwrap();
        d.control_in(op::REQ_GET_MASTER_VOLUME, 0, 4).unwrap();
        assert!(chunk(&mut d, 1024).is_err());
        // The snapshot is taken at offset 0: a change at the device in
        // between is not in it (and, not being a request, keeps the session).
        let mut buf = chunk(&mut d, 0).unwrap();
        let r = d.remote();
        assert!(r.poke(off_of("master_volume"), &(-3.0f32).to_le_bytes(), 5));
        while buf.len() < generated::BULK_SIZE {
            let want = 1024.min(generated::BULK_SIZE - buf.len());
            buf.extend(
                d.control_in(op::REQ_GET_ALL_PARAMS_CHUNK, buf.len() as u16, want as u16)
                    .unwrap(),
            );
        }
        assert_eq!(buf, DEMO_BULK);
        assert_ne!(r.image(), DEMO_BULK);
    }

    #[test]
    fn wp_t4_bulk_writes_apply_and_invalidate() {
        let mut d = FakeDevice::demo();
        let mut src = d.notifications().unwrap();
        let mv = off_of("master_volume");
        let mut img = DEMO_BULK.to_vec();
        img[mv..mv + 4].copy_from_slice(&(-7.5f32).to_le_bytes());

        // Chunked, in order.
        for (i, part) in img.chunks(2048).enumerate() {
            d.control_out(op::REQ_SET_ALL_PARAMS_CHUNK, (i * 2048) as u16, part)
                .unwrap();
        }
        assert_eq!(d.remote().image(), img);
        assert_eq!(
            drain(&mut src),
            vec![vec![2, 3, 0, 0, nfy::PARAM_SRC_BULK_SET as u8, 0, 0, 0]]
        );

        // Out of order stalls.
        assert!(
            d.control_out(op::REQ_SET_ALL_PARAMS_CHUNK, 2048, &img[2048..4096])
                .is_err()
        );
        // Another wire version or size is refused.
        let mut bad = img.clone();
        bad[0] = 31;
        assert!(d.control_out(op::REQ_SET_ALL_PARAMS, 0, &bad).is_err());
        assert!(
            d.control_out(op::REQ_SET_ALL_PARAMS, 0, &img[..100])
                .is_err()
        );
        // Single-shot.
        img[mv..mv + 4].copy_from_slice(&(-1.0f32).to_le_bytes());
        d.control_out(op::REQ_SET_ALL_PARAMS, 0, &img).unwrap();
        assert_eq!(
            f32_at(&d.control_in(op::REQ_GET_MASTER_VOLUME, 0, 4).unwrap(), 0),
            -1.0
        );
        // The header stays the device's own.
        let mut other = img.clone();
        other[1] = 0; // claims RP2040
        d.control_out(op::REQ_SET_ALL_PARAMS, 0, &other).unwrap();
        assert_eq!(d.remote().image()[1], DEMO_BULK[1]);
    }

    // ---- scalar writes ----------------------------------------------------

    #[test]
    fn wp_q2a_band_bypass_flips_the_band_and_notifies() {
        let mut d = FakeDevice::demo();
        let mut src = d.notifications().unwrap();
        let band_off = |ch: u16, b: u16| {
            if b >= 20 {
                off_of("crossovers")
                    + (usize::from(ch) * usize::from(w::WIRE_MAX_XOVER_BANDS) + usize::from(b - 20))
                        * BAND_LEN
            } else {
                off_of("eq")
                    + (usize::from(ch) * usize::from(w::WIRE_MAX_BANDS) + usize::from(b)) * BAND_LEN
            }
        };
        let image = |d: &FakeDevice, at: usize| d.remote().image()[at + 1];

        // A PEQ band: on, and the host-set notification says so.
        d.control_out(op::REQ_SET_BAND_BYPASS, 3, &[1]).unwrap();
        assert_eq!(image(&d, band_off(0, 3)), 1);
        assert!(src.read(Duration::from_millis(100)).is_ok());
        d.control_out(op::REQ_SET_BAND_BYPASS, 3, &[0]).unwrap();
        assert_eq!(image(&d, band_off(0, 3)), 0);

        // A crossover band on an output.
        d.control_out(op::REQ_SET_BAND_BYPASS, (9 << 8) | 21, &[1])
            .unwrap();
        assert_eq!(image(&d, band_off(9, 21)), 1);

        // Bands that do not exist change nothing: a crossover on an input, a
        // gap between the ranges, a channel past the end.
        let before = d.remote().image();
        for v in [21, 15, (200 << 8) | 1] {
            d.control_out(op::REQ_SET_BAND_BYPASS, v, &[1]).unwrap();
        }
        assert_eq!(d.remote().image(), before);
    }

    #[test]
    fn wp_t4_a_write_changes_the_image_and_notifies() {
        let mut d = FakeDevice::demo();
        let mut src = d.notifications().unwrap();
        let remote = d.remote();
        let off = off_of("master_volume");
        assert_eq!(f32_at(DEMO_BULK, off), -20.0, "capture.json");

        d.control_out(op::REQ_SET_MASTER_VOLUME, 0, &(-12.5f32).to_le_bytes())
            .unwrap();
        let got = d.control_in(op::REQ_GET_MASTER_VOLUME, 0, 4).unwrap();
        assert_eq!(f32_at(&got, 0), -12.5);
        assert_eq!(f32_at(&remote.image(), off), -12.5);

        // The legacy v1 packet first, then v2.
        let v1 = src.read(Duration::from_millis(100)).unwrap();
        assert_eq!(v1[..4], [1, 0, 0, 0]);
        assert_eq!(f32_at(&v1, 4), -12.5);
        let p = src.read(Duration::from_millis(100)).unwrap();
        let mut want = vec![2, 2, 0, 0];
        want.extend_from_slice(&(off as u16).to_le_bytes());
        want.extend_from_slice(&4u16.to_le_bytes());
        want.extend_from_slice(&[nfy::PARAM_SRC_HOST_SET as u8, 0, 0, 0]);
        want.extend_from_slice(&(-12.5f32).to_le_bytes());
        assert_eq!(p, want);

        // The same value again: no change, so only the v1 packet, which the
        // firmware sends on every call.
        d.control_out(op::REQ_SET_MASTER_VOLUME, 0, &(-12.5f32).to_le_bytes())
            .unwrap();
        assert_eq!(drain(&mut src).len(), 1);
    }

    #[test]
    fn wp_t4_indexed_scalars_land_in_their_records() {
        let mut d = FakeDevice::demo();
        let mut src = d.notifications().unwrap();
        let img = |d: &FakeDevice| d.remote().image();
        let outs = off_of("outputs");

        d.control_out(op::REQ_SET_OUTPUT_GAIN, 3, &(-4.0f32).to_le_bytes())
            .unwrap();
        assert_eq!(f32_at(&img(&d), outs + 3 * 12 + 4), -4.0);
        d.control_out(op::REQ_SET_OUTPUT_MUTE, 8, &[1]).unwrap();
        assert_eq!(img(&d)[outs + 8 * 12 + 1], 1);
        assert_eq!(d.control_in(op::REQ_GET_OUTPUT_MUTE, 8, 1).unwrap(), [1]);
        // Booleans are stored as 0/1.
        d.control_out(op::REQ_SET_BYPASS, 0, &[7]).unwrap();
        assert_eq!(d.control_in(op::REQ_GET_BYPASS, 0, 1).unwrap(), [1]);
        d.control_out(op::REQ_SET_DELAY, 16, &2.5f32.to_le_bytes())
            .unwrap();
        assert_eq!(f32_at(&img(&d), off_of("delays") + 16 * 4), 2.5);
        d.control_out(op::REQ_SET_PREAMP_CH, 7, &(-3.0f32).to_le_bytes())
            .unwrap();
        assert_eq!(
            f32_at(&d.control_in(op::REQ_GET_PREAMP_CH, 7, 4).unwrap(), 0),
            -3.0
        );
        let sources: Vec<u8> = drain(&mut src).iter().map(|p| p[8]).collect();
        assert_eq!(sources.len(), 5);
        assert!(sources.iter().all(|s| *s == nfy::PARAM_SRC_HOST_SET as u8));

        // Past the device's count: a GET stalls, a SET is a silent no-op, as
        // is a short payload.
        let before = img(&d);
        assert!(matches!(
            d.control_in(op::REQ_GET_OUTPUT_GAIN, 9, 4),
            Err(TransportError::Stalled { .. })
        ));
        d.control_out(op::REQ_SET_OUTPUT_GAIN, 9, &1.0f32.to_le_bytes())
            .unwrap();
        d.control_out(op::REQ_SET_PREAMP_CH, 8, &1.0f32.to_le_bytes())
            .unwrap();
        d.control_out(op::REQ_SET_MASTER_VOLUME, 0, &[1, 2])
            .unwrap();
        assert_eq!(img(&d), before);
        assert!(drain(&mut src).is_empty());
    }

    #[test]
    fn wp_t4_matrix_eq_and_names() {
        let mut d = FakeDevice::demo();
        // A crosspoint, packet in and out.
        let mut pkt = vec![1, 4, 1, 5];
        pkt.extend_from_slice(&(-6.0f32).to_le_bytes());
        d.control_out(op::REQ_SET_MATRIX_ROUTE, 0, &pkt).unwrap();
        let got = d
            .control_in(op::REQ_GET_MATRIX_ROUTE, (1 << 8) | 4, 8)
            .unwrap();
        assert_eq!(got[..4], [1, 4, 1, 1]);
        assert_eq!(f32_at(&got, 4), -6.0);
        let at = off_of("crosspoints") + (9 + 4) * 8;
        assert_eq!(d.remote().image()[at..at + 2], [1, 1]);

        // A PEQ band, read back one parameter at a time.
        let mut band = vec![10, 3, 2, 1];
        for v in [1000.0f32, 0.7, -3.0] {
            band.extend_from_slice(&v.to_le_bytes());
        }
        d.control_out(op::REQ_SET_EQ_PARAM, 0, &band).unwrap();
        let get = |d: &mut FakeDevice, ch: u16, b: u16, p: u16| {
            d.control_in(op::REQ_GET_EQ_PARAM, (ch << 8) | (b << 3) | p, 4)
        };
        assert_eq!(get(&mut d, 10, 3, 0).unwrap(), 2u32.to_le_bytes());
        assert_eq!(f32_at(&get(&mut d, 10, 3, 1).unwrap(), 0), 1000.0);
        assert_eq!(f32_at(&get(&mut d, 10, 3, 2).unwrap(), 0), 0.7);
        assert_eq!(f32_at(&get(&mut d, 10, 3, 3).unwrap(), 0), -3.0);
        assert_eq!(get(&mut d, 10, 3, 4).unwrap(), 1u32.to_le_bytes());
        // Reserved bands stall, which is how a host counts the live ones.
        assert!(get(&mut d, 0, 9, 0).is_ok());
        assert!(get(&mut d, 0, 10, 0).is_err());
        // Crossover bands exist on outputs only.
        assert!(get(&mut d, 8, 20, 0).is_ok());
        assert!(get(&mut d, 0, 20, 0).is_err());
        assert!(get(&mut d, 17, 0, 0).is_err());

        // A channel name, at most 31 bytes, NUL padded.
        d.control_out(op::REQ_SET_CHANNEL_NAME, 2, b"Tweeter L")
            .unwrap();
        let name = d.control_in(op::REQ_GET_CHANNEL_NAME, 2, 32).unwrap();
        assert_eq!(&name[..10], b"Tweeter L\0");
        d.control_out(op::REQ_SET_CHANNEL_NAME, 2, &[b'x'; 40])
            .unwrap();
        let name = d.control_in(op::REQ_GET_CHANNEL_NAME, 2, 32).unwrap();
        assert_eq!((name[30], name[31]), (b'x', 0));
    }

    // ---- presets ----------------------------------------------------------

    #[test]
    fn wp_t4_presets_save_load_and_name() {
        let mut d = FakeDevice::demo();
        let mut src = d.notifications().unwrap();
        let dir = d.control_in(op::REQ_PRESET_GET_DIR, 0, 7).unwrap();
        assert_eq!(dir, [1, 0, 0, 0, 0, OC_WITH_PRESET, MV_INDEPENDENT]);
        let name = d.control_in(op::REQ_PRESET_GET_NAME, 0, 32).unwrap();
        assert_eq!(&name[..5], b"Demo\0");
        assert!(d.control_in(op::REQ_PRESET_GET_NAME, 10, 32).is_err());
        assert_eq!(d.control_in(op::REQ_PRESET_GET_ACTIVE, 0, 1).unwrap(), [0]);

        // Change a gain, save to slot 3, name it.
        let outs = off_of("outputs");
        d.control_out(op::REQ_SET_OUTPUT_GAIN, 0, &(-9.0f32).to_le_bytes())
            .unwrap();
        assert_eq!(
            d.control_in(op::REQ_PRESET_SAVE, 3, 1).unwrap(),
            [st::PRESET_OK as u8]
        );
        d.control_out(op::REQ_PRESET_SET_NAME, 3, b"Night").unwrap();
        let dir = d.control_in(op::REQ_PRESET_GET_DIR, 0, 7).unwrap();
        assert_eq!(dir[..2], [0b1001, 0]);
        assert_eq!(dir[4], 3);
        assert_eq!(
            &d.control_in(op::REQ_PRESET_GET_NAME, 3, 32).unwrap()[..6],
            b"Night\0"
        );
        // The master volume is independent of presets by default: a load
        // brings the gain back but leaves the volume.
        d.control_out(op::REQ_SET_MASTER_VOLUME, 0, &(-30.0f32).to_le_bytes())
            .unwrap();
        assert_eq!(drain(&mut src).len(), 3, "gain, master volume v1 and v2");
        assert_eq!(d.control_in(op::REQ_PRESET_LOAD, 0, 1).unwrap(), [0]);
        let img = d.remote().image();
        assert_eq!(f32_at(&img, outs + 4), f32_at(DEMO_BULK, outs + 4));
        assert_eq!(f32_at(&img, off_of("master_volume")), -30.0);
        assert_eq!(
            drain(&mut src),
            vec![
                vec![2, 4, 0, 2, 0, 0, 0, 0],
                vec![2, 3, 0, 3, nfy::PARAM_SRC_PRESET as u8, 0, 0, 0],
            ]
        );
        assert_eq!(d.control_in(op::REQ_PRESET_GET_ACTIVE, 0, 1).unwrap(), [0]);
        assert_eq!(d.control_in(op::REQ_PRESET_DELETE, 3, 1).unwrap(), [0]);
        assert_eq!(d.control_in(op::REQ_PRESET_GET_DIR, 0, 7).unwrap()[0], 1);
        assert_eq!(
            d.control_in(op::REQ_PRESET_SAVE, 10, 1).unwrap(),
            [st::PRESET_ERR_INVALID_SLOT as u8]
        );
        d.control_out(op::REQ_PRESET_SET_STARTUP, 0, &[1, 3])
            .unwrap();
        assert_eq!(
            d.control_in(op::REQ_PRESET_GET_STARTUP, 0, 3).unwrap(),
            [1, 3, 0]
        );
    }

    // ---- extensions ------------------------------------------------------

    #[test]
    fn wp_t4_extensions_answer_only_when_enabled() {
        let mut plain = FakeDevice::new(Capture::demo(), FakeOptions::default());
        assert!(matches!(
            plain.control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64),
            Err(TransportError::Stalled { .. })
        ));

        let mut d = FakeDevice::demo();
        let list = d
            .control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64)
            .unwrap();
        assert_eq!(dspi_proto::ext::decode_list(&list), [1, 2]);
        assert!(
            d.control_out(op::REQ_EXTENSION, gext::EXT_LIST, &[])
                .is_err()
        );
        assert!(d.control_in(op::REQ_EXTENSION, 0x7777, 4).is_err());
    }

    #[test]
    fn wp_t4_pair_map_validates_normalises_and_notifies() {
        let mut d = FakeDevice::demo();
        let mut src = d.notifications().unwrap();
        let read = |d: &mut FakeDevice| {
            d.control_in(op::REQ_EXTENSION, gext::EXT_CROSSFEED_PAIR_MAP, 4)
                .unwrap()
        };
        assert_eq!(read(&mut d), [0; 4]);
        // The K340 map, with an explicit default for pair 2.
        d.control_out(
            op::REQ_EXTENSION,
            gext::EXT_CROSSFEED_PAIR_MAP,
            &[0x02, 0x13, 0x54, 0x00],
        )
        .unwrap();
        assert_eq!(read(&mut d), [0x02, 0x13, 0, 0]);
        let p = drain(&mut src);
        assert_eq!(p.len(), 1);
        assert_eq!(u16::from_le_bytes([p[0][4], p[0][5]]), 44);
        assert_eq!(p[0][12..], [0x02, 0x13, 0, 0]);
        // Invalid maps are acknowledged and ignored: an output in two
        // pairs, a pair onto itself, the PDM sub (output 8).
        for bad in [[0x02, 0x12, 0, 0], [0x11, 0, 0, 0], [0x80, 0, 0, 0]] {
            d.control_out(op::REQ_EXTENSION, gext::EXT_CROSSFEED_PAIR_MAP, &bad)
                .unwrap();
            assert_eq!(read(&mut d), [0x02, 0x13, 0, 0]);
        }
        assert!(drain(&mut src).is_empty());
        // The wrong length stalls.
        assert!(
            d.control_out(op::REQ_EXTENSION, gext::EXT_CROSSFEED_PAIR_MAP, &[0; 3])
                .is_err()
        );
        // Five outputs (RP2040): two pairs, so pairs 2 and 3 stay default.
        assert!(!PairMap([0, 0, 0x54, 0]).is_valid(2));
        let explicit = PairMap([0x10, 0x32, 0, 0]);
        assert!(explicit.is_valid(2));
        assert_eq!(explicit.normalized().0, [0; 4]);
    }

    #[test]
    fn wp_t4_core_vreg_requests() {
        use dspi_proto::ext::{CoreVregStatus, VregOp};
        let mut d = FakeDevice::demo();
        let status = |d: &mut FakeDevice| {
            CoreVregStatus::decode(
                &d.control_in(op::REQ_EXTENSION, gext::EXT_CORE_VREG, 16)
                    .unwrap(),
            )
            .unwrap()
        };
        let req = |d: &mut FakeDevice, o: VregOp| {
            d.control_out(op::REQ_EXTENSION, gext::EXT_CORE_VREG, &o.encode())
        };
        assert_eq!(status(&mut d).live_vreg, DEMO_VREG);
        req(&mut d, VregOp::Trial(0x0F)).unwrap();
        let s = status(&mut d);
        assert!(s.trial_active);
        assert_eq!((s.live_vreg, s.trial_vreg), (0x0F, 0x0F));
        assert_eq!(
            d.control_in(op::REQ_GET_STATUS, 14, 4).unwrap(),
            1300u32.to_le_bytes()
        );
        req(&mut d, VregOp::TrialEnd).unwrap();
        assert_eq!(status(&mut d).live_vreg, DEMO_VREG);
        req(&mut d, VregOp::Set(0x0E)).unwrap();
        let s = status(&mut d);
        assert_eq!((s.stored_sel, s.live_vreg), (0x0E, 0x0E));
        // Below the build default, outside the window, an unknown op.
        assert!(req(&mut d, VregOp::Set(0x0C)).is_err());
        assert!(req(&mut d, VregOp::Trial(0x0B)).is_err());
        assert!(
            d.control_out(op::REQ_EXTENSION, gext::EXT_CORE_VREG, &[9, 0])
                .is_err()
        );
        req(&mut d, VregOp::Auto(false)).unwrap();
        assert_eq!(status(&mut d).flags, CoreVregStatus::FLAG_AUTO_DISABLED);
    }

    // ---- the device's side ---------------------------------------------

    #[test]
    fn wp_t4_remote_pokes_unplugs_and_close_ends_the_stream() {
        let d = FakeDevice::demo();
        let mut src = d.notifications().unwrap();
        let r = d.remote();
        let off = off_of("user_volume");
        assert!(r.poke(off, &(-5.0f32).to_le_bytes(), nfy::PARAM_SRC_GPIO as u8));
        assert!(!r.poke(0, &[0], 5), "the header is not data");
        assert!(!r.poke(generated::BULK_SIZE - 1, &[0, 0], 5));
        let p = src.read(Duration::from_millis(100)).unwrap();
        assert_eq!(p[8], nfy::PARAM_SRC_GPIO as u8);

        // A reader blocked in `read` wakes when a packet arrives.
        let r2 = r.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            r2.poke(off, &(-6.0f32).to_le_bytes(), 5);
        });
        let p = src.read(Duration::from_secs(5)).unwrap();
        assert_eq!(f32_at(&p, 12), -6.0);
        t.join().unwrap();
        // Silence is an empty packet once the timeout runs out.
        assert!(src.read(Duration::from_millis(10)).unwrap().is_empty());

        drop(d);
        assert!(matches!(
            src.read(Duration::from_millis(10)),
            Err(TransportError::Disconnected)
        ));

        let mut d = FakeDevice::demo();
        d.remote().disconnect();
        assert!(matches!(
            d.control_in(op::REQ_GET_PLATFORM, 0, 7),
            Err(TransportError::Disconnected)
        ));
    }

    #[test]
    fn wp_t4_a_bad_capture_never_panics() {
        let captures = [
            vec![],
            vec![32],
            vec![0xFF; 40],
            DEMO_BULK[..5000].to_vec(),
            DEMO_BULK.to_vec(),
        ];
        for bulk in captures {
            let mut d = FakeDevice::new(
                Capture {
                    platform: vec![],
                    bulk,
                },
                FakeOptions {
                    extensions: true,
                    ..FakeOptions::default()
                },
            );
            for opcode in 0..=255u8 {
                for value in [0u16, 1, 2, 9, 14, 0x0103, 0x10FF, 0xFFFF] {
                    let _ = d.control_in(opcode, value, 64);
                    let _ = d.control_in_upto(opcode, value, 4096);
                    let _ = d.control_out(opcode, value, &[0x21, 2, 3, 4, 5, 6, 7, 8]);
                    let _ = d.control_out(opcode, value, &[0xFF; 20]);
                    let _ = d.control_out(opcode, value, &[0x21; 44]);
                    let _ = d.control_out(opcode, value, &[0; 16]);
                    let _ = d.control_out(opcode, value, &[]);
                }
            }
        }
    }

    /// WP CG16: control surfaces, this test's example until then, have a model
    /// now, and firmware without surfaces still stalls theirs (`tests_wp_cg16`
    /// in `fake/surfaces.rs`). WP CG22: so has the signal generator, on the
    /// demo; firmware without it stalls it.
    #[test]
    fn wp_t4_unknown_requests_stall() {
        let mut d = FakeDevice::new(Capture::demo(), FakeOptions::default());
        assert!(matches!(
            d.control_in(op::REQ_SIGGEN_GET_STATUS, 0, 4),
            Err(TransportError::Stalled { opcode }) if opcode == op::REQ_SIGGEN_GET_STATUS
        ));
        assert!(matches!(
            d.control_out(op::REQ_FACTORY_RESET, 0, &[]),
            Err(TransportError::Stalled { .. })
        ));
    }

    // ---- review fixups (batch 4) ------------------------------------------

    #[test]
    fn wp_t4_unknown_sets_ack_only_when_asked() {
        let mut strict = FakeDevice::demo();
        assert!(strict.control_out(op::REQ_FACTORY_RESET, 0, &[]).is_err());
        let mut lax = FakeDevice::new(
            Capture::demo(),
            FakeOptions {
                ack_unknown_sets: true,
                ..FakeOptions::default()
            },
        );
        lax.control_out(op::REQ_FACTORY_RESET, 0, &[]).unwrap();
        assert_eq!(
            lax.remote().image(),
            DEMO_BULK,
            "acknowledged, not modelled"
        );
        // The extension channel's refusals stall even so.
        assert!(lax.control_out(op::REQ_EXTENSION, 1, &[0; 4]).is_err());
        // GETs still stall.
        assert!(lax.control_in(op::REQ_GET_CS_CAPS, 0xFFFF, 4).is_err());
    }

    #[test]
    fn wp_t4_transfers_past_max_transfer_are_refused() {
        let mut d = FakeDevice::demo();
        assert_eq!(d.max_transfer(), 65535);
        d.control_in(op::REQ_GET_ALL_PARAMS, 0, generated::BULK_SIZE as u16)
            .unwrap();
        let mut win = FakeDevice::new(
            Capture::demo(),
            FakeOptions {
                max_transfer: Some(4096),
                ..FakeOptions::default()
            },
        );
        assert_eq!(win.max_transfer(), 4096);
        assert!(matches!(
            win.control_in(op::REQ_GET_ALL_PARAMS, 0, generated::BULK_SIZE as u16),
            Err(TransportError::Usb(_))
        ));
        assert!(matches!(
            win.control_in_upto(op::REQ_GET_ALL_PARAMS, 0, 4097),
            Err(TransportError::Usb(_))
        ));
        assert!(matches!(
            win.control_out(op::REQ_SET_ALL_PARAMS, 0, DEMO_BULK),
            Err(TransportError::Usb(_))
        ));
        // The chunked path fits.
        assert_eq!(read_chunked(&mut win, 4096), DEMO_BULK);
    }

    #[test]
    fn wp_t4_a_failed_chunk_closes_every_session() {
        let mut d = FakeDevice::demo();
        d.control_out(op::REQ_SET_ALL_PARAMS_CHUNK, 0, &DEMO_BULK[..2048])
            .unwrap();
        // A read chunk at a later offset fails, and takes the write with it.
        assert!(
            d.control_in(op::REQ_GET_ALL_PARAMS_CHUNK, 2048, 1024)
                .is_err()
        );
        assert!(
            d.control_out(op::REQ_SET_ALL_PARAMS_CHUNK, 2048, &DEMO_BULK[2048..4096])
                .is_err()
        );
        // And the other way round.
        d.control_in(op::REQ_GET_ALL_PARAMS_CHUNK, 0, 1024).unwrap();
        assert!(
            d.control_out(op::REQ_SET_ALL_PARAMS_CHUNK, 1024, &[0; 16])
                .is_err()
        );
        assert!(
            d.control_in(op::REQ_GET_ALL_PARAMS_CHUNK, 1024, 1024)
                .is_err()
        );
    }

    #[test]
    fn wp_t4_remote_pushes_events_status_and_images() {
        let mut d = FakeDevice::demo();
        let mut src = d.notifications().unwrap();
        let r = d.remote();
        // A v2 packet takes the next sequence number; anything else is raw.
        r.push(vec![2, 7, 0, 0xEE, 1, 0, 3, 0xFF]);
        r.push(vec![1, 0, 0, 0, 0, 0, 0xA0, 0xC1]);
        r.push(vec![0x55]);
        assert_eq!(
            drain(&mut src),
            vec![
                vec![2, 7, 0, 0, 1, 0, 3, 0xFF],
                vec![1, 0, 0, 0, 0, 0, 0xA0, 0xC1],
                vec![0x55],
            ]
        );

        r.set_status(vec![9; 41]);
        assert_eq!(d.control_in(op::REQ_GET_STATUS, 9, 41).unwrap(), [9; 41]);
        r.set_status(vec![1, 2]);
        assert!(d.control_in(op::REQ_GET_STATUS, 9, 41).is_err());

        let mut img = DEMO_BULK.to_vec();
        img[off_of("master_volume")] ^= 0x40;
        r.load_image(img.clone(), nfy::PARAM_SRC_GPIO as u8);
        assert_eq!(read_chunked(&mut d, 1024), img);
        assert_eq!(
            drain(&mut src),
            vec![vec![2, 3, 0, 1, nfy::PARAM_SRC_GPIO as u8, 0, 0, 0]]
        );
    }

    // ---- WP B2 ------------------------------------------------------------

    fn image_f32(d: &FakeDevice, section_name: &str, offset: usize) -> f32 {
        let (at, _) = section(section_name).unwrap();
        let img = d.remote().image();
        f32::from_le_bytes(img[at + offset..at + offset + 4].try_into().unwrap())
    }

    #[test]
    fn wp_b2_global_preamp_follows_input_0() {
        let mut d = FakeDevice::demo();
        d.control_out(op::REQ_SET_PREAMP_CH, 0, &(-7.0f32).to_le_bytes())
            .unwrap();
        assert_eq!(image_f32(&d, "global", G_PREAMP.1), -7.0);
        d.control_out(op::REQ_SET_PREAMP_CH, 1, &(-3.0f32).to_le_bytes())
            .unwrap();
        assert_eq!(
            image_f32(&d, "global", G_PREAMP.1),
            -7.0,
            "input 1 is not it"
        );
        d.control_out(op::REQ_SET_PREAMP, 0, &(-2.0f32).to_le_bytes())
            .unwrap();
        assert_eq!(image_f32(&d, "global", G_PREAMP.1), -2.0);
    }

    #[test]
    fn wp_b2_an_output_delay_is_also_its_channel_delay() {
        let mut d = FakeDevice::demo();
        let inputs = usize::from(d.shared.lock().num_inputs());
        let mut notes = d.notifications().unwrap();
        while !notes.read(Duration::from_millis(1)).unwrap().is_empty() {}
        d.control_out(op::REQ_SET_OUTPUT_DELAY, 2, &12.5f32.to_le_bytes())
            .unwrap();
        assert_eq!(image_f32(&d, "delays", (inputs + 2) * 4), 12.5);
        // Both offsets notify, as the firmware does.
        let mut offsets = Vec::new();
        for _ in 0..2 {
            let p = notes.read(Duration::from_millis(50)).unwrap();
            offsets.push(u16::from_le_bytes([p[4], p[5]]) as usize);
        }
        assert!(offsets.contains(&(section("delays").unwrap().0 + (inputs + 2) * 4)));
    }

    #[test]
    fn wp_b2_save_params_and_factory_reset_are_modelled() {
        let mut d = FakeDevice::demo();
        d.control_out(op::REQ_SET_PREAMP_CH, 0, &(-7.0f32).to_le_bytes())
            .unwrap();
        // Saved: reloading the active slot brings the value back.
        assert_eq!(d.control_in(op::REQ_SAVE_PARAMS, 0, 1).unwrap(), [FLASH_OK]);
        d.control_out(op::REQ_SET_PREAMP_CH, 0, &(-1.0f32).to_le_bytes())
            .unwrap();
        d.control_in(op::REQ_PRESET_LOAD, 0, 1).unwrap();
        assert_eq!(image_f32(&d, "global", G_PREAMP.1), -7.0);

        let mut notes = d.notifications().unwrap();
        while !notes.read(Duration::from_millis(1)).unwrap().is_empty() {}
        assert_eq!(
            d.control_in(op::REQ_FACTORY_RESET, 0, 1).unwrap(),
            [FLASH_OK]
        );
        assert_eq!(d.remote().image(), DEMO_BULK, "back to the factory image");
        let p = notes.read(Duration::from_millis(50)).unwrap();
        assert_eq!(u16::from(p[1]), nfy::NOTIFY_EVT_BULK_INVALIDATED);
        assert_eq!(u16::from(p[4]), nfy::PARAM_SRC_FACTORY);
    }

    #[test]
    fn wp_b2_bulk_source_tags_the_invalidation_and_reports_the_feature() {
        for tagged in [false, true] {
            let mut d = FakeDevice::new(
                Capture::demo(),
                FakeOptions {
                    extensions: true,
                    bulk_source: tagged,
                    ..FakeOptions::default()
                },
            );
            let list = d
                .control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64)
                .unwrap();
            let listed = list
                .chunks(2)
                .any(|c| c == gext::EXT_NOTIFY_FEATURES.to_le_bytes());
            assert_eq!(listed, tagged);
            let features = d.control_in(op::REQ_EXTENSION, gext::EXT_NOTIFY_FEATURES, 4);
            if tagged {
                assert_eq!(features.unwrap(), [1, 0, 0, 0]);
            } else {
                assert!(matches!(features, Err(TransportError::Stalled { .. })));
            }

            let mut notes = d.notifications().unwrap();
            while !notes.read(Duration::from_millis(1)).unwrap().is_empty() {}
            d.control_out(op::REQ_SET_ALL_PARAMS, 0, DEMO_BULK).unwrap();
            let p = notes.read(Duration::from_millis(50)).unwrap();
            assert_eq!(u16::from(p[1]), nfy::NOTIFY_EVT_BULK_INVALIDATED);
            let want = if tagged {
                nfy::PARAM_SRC_HOST_SET
            } else {
                nfy::PARAM_SRC_BULK_SET
            };
            assert_eq!(u16::from(p[4]), want);
        }
    }
}

// -- WP TM16 --
#[cfg(test)]
mod tests_wp_tm16 {
    use super::*;
    use crate::Transport;

    #[test]
    fn wp_tm16_the_fake_acks_the_watermark_reset() {
        let mut d = FakeDevice::demo();
        let reply = d.control_in(op::REQ_RESET_BUFFER_STATS, 1, 1).unwrap();
        assert_eq!(reply, [1]);
    }
}

// -- WP CG12 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg12 {
    use dspi_proto::packets::BufferStatsPacket;

    use super::*;
    use crate::Transport;

    fn stats(d: &mut FakeDevice) -> Vec<u8> {
        d.control_in(op::REQ_GET_BUFFER_STATS, 0, 44).unwrap()
    }

    fn target(d: &mut FakeDevice) -> u16 {
        let s = d.control_in(op::REQ_OUTPUT_LATENCY, 0, 12).unwrap();
        u16::from_le_bytes([s[4], s[5]])
    }

    #[test]
    fn wp_cg12_lowest_fill_starts_as_the_firmware_does() {
        let mut d = FakeDevice::demo();
        let v = d.control_in(op::REQ_GET_STATUS, 28, 4).unwrap();
        assert_eq!(v, u32::MAX.to_le_bytes());
    }

    #[test]
    fn wp_cg12_buffer_stats_have_the_firmware_layout() {
        let mut d = FakeDevice::demo();
        let frames = u32::from(target(&mut d));
        let raw = stats(&mut d);
        assert_eq!(raw.len(), BufferStatsPacket::SIZE);

        let p = BufferStatsPacket::decode(&raw).unwrap();
        assert_eq!(p.num_spdif, 4);
        assert_eq!(p.flags, 0);
        let buckets = (frames / 48) as u8;
        let live = p.spdif[3];
        assert_eq!(live.consumer_prepared, buckets);
        assert_eq!(live.consumer_free, 16 - buckets);
        assert_eq!(live.consumer_playing, 1);
        assert_eq!(
            u32::from(live.consumer_fill_pct),
            u32::from(buckets) * 100 / 16
        );
        assert_eq!(live.consumer_min_fill_pct, live.consumer_fill_pct);
        assert_eq!(live.consumer_max_fill_pct, live.consumer_fill_pct);
        // fill_centi_pct, bytes 6-7 of the last entry: frames over 768.
        let centi = u32::from(u16::from_le_bytes([raw[34], raw[35]]));
        assert_eq!(centi, frames * 10_000 / 768);
        assert_eq!(&raw[36..44], &[0; 8]);
    }

    #[test]
    fn wp_cg12_the_sequence_counts_reads() {
        let mut d = FakeDevice::demo();
        let first = stats(&mut d);
        let second = stats(&mut d);
        let seq = |b: &[u8]| u16::from_le_bytes([b[2], b[3]]);
        assert_eq!(seq(&second), seq(&first).wrapping_add(1));
    }

    #[test]
    fn wp_cg12_the_fill_follows_the_latency_setting() {
        let mut d = FakeDevice::demo();
        let before = target(&mut d);
        let centi = |d: &mut FakeDevice| {
            let raw = stats(d);
            u32::from(u16::from_le_bytes([raw[10], raw[11]]))
        };
        let was = centi(&mut d);
        // 8 ms, the longest option, runs on every source.
        d.control_out(op::REQ_OUTPUT_LATENCY, 0, &[7]).unwrap();
        let after = target(&mut d);
        assert_ne!(before, after);
        assert_ne!(was, centi(&mut d));
        assert_eq!(centi(&mut d), u32::from(after) * 10_000 / 768);
    }
}

/// WP CG11: the simulation's own hook; its effects are tested through the core
/// (`dspi-core`'s `vreg/tests_wp_cg11.rs`).
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg11 {
    use super::*;

    fn with_extensions() -> FakeDevice {
        FakeDevice::new(
            Capture::demo(),
            FakeOptions {
                extensions: true,
                ..FakeOptions::default()
            },
        )
    }

    #[test]
    fn wp_cg11_the_sim_payload_round_trips_and_a_bad_one_is_refused() {
        for sim in [
            VregSim::Hang,
            VregSim::Reboot,
            VregSim::PowerCycle,
            VregSim::Advance(0x0102_0304),
        ] {
            assert_eq!(VregSim::decode(&sim.encode()), Some(sim));
        }
        assert_eq!(VregSim::decode(&[9, 0, 0, 0, 0]), None);
        assert_eq!(VregSim::decode(&[0]), None);

        let mut d = with_extensions();
        assert!(matches!(
            d.control_out(op::REQ_EXTENSION, EXT_FAKE_VREG_SIM, &[9, 0, 0, 0, 0]),
            Err(TransportError::Stalled { .. })
        ));
    }

    #[test]
    fn wp_cg11_the_sim_is_not_listed_and_a_stock_device_stalls_it() {
        let mut d = with_extensions();
        let list = d
            .control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64)
            .unwrap();
        assert!(
            !list
                .chunks(2)
                .any(|c| c == EXT_FAKE_VREG_SIM.to_le_bytes().as_slice())
        );
        let mut stock = FakeDevice::new(Capture::demo(), FakeOptions::default());
        assert!(matches!(
            stock.control_out(
                op::REQ_EXTENSION,
                EXT_FAKE_VREG_SIM,
                &VregSim::Hang.encode()
            ),
            Err(TransportError::Stalled { .. })
        ));
    }

    #[test]
    fn wp_cg11_staging_keeps_one_request_and_a_reset_loses_it() {
        let d = with_extensions();
        let r = d.remote();
        r.vreg_stage_requests(true);
        let mut d = d;
        let status = |d: &mut FakeDevice| {
            d.control_in(op::REQ_EXTENSION, gext::EXT_CORE_VREG, 16)
                .unwrap()
        };
        d.control_out(op::REQ_EXTENSION, gext::EXT_CORE_VREG, &[1, 0x0E])
            .unwrap();
        d.control_out(op::REQ_EXTENSION, gext::EXT_CORE_VREG, &[1, 0x0F])
            .unwrap();
        assert_eq!(status(&mut d)[5], 0, "not applied yet");
        r.vreg_run_main_loop();
        assert_eq!(status(&mut d)[6], 0x0F, "the later request won");
        d.control_out(op::REQ_EXTENSION, gext::EXT_CORE_VREG, &[2, 0])
            .unwrap();
        r.vreg_sim(VregSim::Reboot);
        r.vreg_run_main_loop();
        assert_eq!(status(&mut d)[5], 0);
        // A refused request is not staged.
        assert!(
            d.control_out(op::REQ_EXTENSION, gext::EXT_CORE_VREG, &[1, 0x10])
                .is_err()
        );
    }
}

/// WP CG16: the list of fake-only codes.
#[cfg(test)]
mod tests_wp_cg16_extensions {
    use super::*;

    // -- WP QD31 --
    #[test]
    fn wp_qd31_the_output_config_save_answers_as_the_firmware_and_the_sim_says() {
        let mut d = FakeDevice::demo();
        let save = |d: &mut FakeDevice| d.control_in(op::REQ_SAVE_OUTPUT_CONFIG, 0, 1);
        assert_eq!(save(&mut d).unwrap(), [st::PRESET_OK as u8]);
        d.control_out(
            op::REQ_EXTENSION,
            EXT_FAKE_OUTPUT_SAVE_SIM,
            &[st::PRESET_ERR_FLASH_WRITE as u8],
        )
        .unwrap();
        assert_eq!(save(&mut d).unwrap(), [st::PRESET_ERR_FLASH_WRITE as u8]);
        d.control_out(
            op::REQ_EXTENSION,
            EXT_FAKE_OUTPUT_SAVE_SIM,
            &[OUTPUT_SAVE_STALL],
        )
        .unwrap();
        assert!(matches!(save(&mut d), Err(TransportError::Stalled { .. })));
        d.control_out(op::REQ_EXTENSION, EXT_FAKE_OUTPUT_SAVE_SIM, &[0])
            .unwrap();
        assert_eq!(save(&mut d).unwrap(), [st::PRESET_OK as u8]);
        // One byte, or it is refused
        assert!(
            d.control_out(op::REQ_EXTENSION, EXT_FAKE_OUTPUT_SAVE_SIM, &[])
                .is_err()
        );
    }

    #[test]
    fn wp_cg16_fake_only_extension_codes_are_unique() {
        let mut codes = FAKE_ONLY_EXTENSIONS.to_vec();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(
            codes.len(),
            FAKE_ONLY_EXTENSIONS.len(),
            "a code is used twice"
        );
        // The experimental range (config.h), and never one a device could list.
        assert!(codes.iter().all(|c| *c >= 0x8000));
        let mut d = FakeDevice::new(
            Capture::demo(),
            FakeOptions {
                extensions: true,
                bulk_source: true,
                surfaces: true,
                ..FakeOptions::default()
            },
        );
        let list = d
            .control_in_upto(op::REQ_EXTENSION, gext::EXT_LIST, 64)
            .unwrap();
        for code in codes {
            assert!(
                !list.chunks(2).any(|c| c == code.to_le_bytes().as_slice()),
                "0x{code:04X} is listed"
            );
        }
    }
}

// -- WP UPW2 --
#[cfg(test)]
mod tests_wp_upw2 {
    use super::*;
    use crate::Transport;

    fn f32_reply(d: &mut FakeDevice, opcode: u8) -> f32 {
        let b = d.control_in(opcode, 0, 4).unwrap();
        f32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    #[test]
    fn wp_upw2_save_master_volume_answers_preset_ok_and_keeps_the_live_value() {
        let mut d = FakeDevice::demo();
        assert_eq!(f32_reply(&mut d, op::REQ_GET_SAVED_MASTER_VOLUME), -20.0);
        d.control_out(op::REQ_SET_MASTER_VOLUME, 0, &(-7.5f32).to_le_bytes())
            .unwrap();
        // Nothing is saved until asked.
        assert_eq!(f32_reply(&mut d, op::REQ_GET_SAVED_MASTER_VOLUME), -20.0);
        let reply = d.control_in(op::REQ_SAVE_MASTER_VOLUME, 0, 1).unwrap();
        assert_eq!(reply, [st::PRESET_OK as u8]);
        assert_eq!(f32_reply(&mut d, op::REQ_GET_SAVED_MASTER_VOLUME), -7.5);
    }

    #[test]
    fn wp_upw2_an_ignored_set_is_acked_and_dropped() {
        let mut d = FakeDevice::demo();
        let remote = d.remote();
        remote.ignore(op::REQ_SET_SUBHARM_SOLO);
        d.control_out(op::REQ_SET_SUBHARM_SOLO, 0, &[1]).unwrap();
        assert_eq!(d.control_in(op::REQ_GET_SUBHARM_SOLO, 0, 1).unwrap(), [0]);
    }

    #[test]
    fn wp_upw2_injected_faults_stall_an_opcode_and_fail_notification_reads() {
        let mut d = FakeDevice::demo();
        let remote = d.remote();
        let mut events = d.notifications().unwrap();
        remote.stall(op::REQ_GET_SUBHARM_SOLO);
        assert!(matches!(
            d.control_in(op::REQ_GET_SUBHARM_SOLO, 0, 1),
            Err(TransportError::Stalled { .. })
        ));
        remote.stall(op::REQ_SET_SUBHARM_SOLO);
        assert!(d.control_out(op::REQ_SET_SUBHARM_SOLO, 0, &[1]).is_err());
        // Other requests are untouched.
        d.control_in(op::REQ_GET_SERIAL, 0, 16).unwrap();

        remote.fail_notifications(2);
        for _ in 0..2 {
            assert!(matches!(
                events.read(Duration::from_secs(5)),
                Err(TransportError::Usb(_))
            ));
        }
        assert!(events.read(Duration::ZERO).unwrap().is_empty());
        remote.fail_notifications(u32::MAX);
        remote.disconnect();
        // An unplug still reads as one.
        assert!(matches!(
            events.read(Duration::ZERO),
            Err(TransportError::Disconnected)
        ));
    }

    #[test]
    fn wp_upw2_band_bypass_reads_back_what_was_set() {
        let mut d = FakeDevice::demo();
        let at = |ch: u16, band: u16| (ch << 8) | band;
        assert_eq!(
            d.control_in(op::REQ_GET_BAND_BYPASS, at(0, 2), 1).unwrap(),
            [0]
        );
        d.control_out(op::REQ_SET_BAND_BYPASS, at(0, 2), &[1])
            .unwrap();
        assert_eq!(
            d.control_in(op::REQ_GET_BAND_BYPASS, at(0, 2), 1).unwrap(),
            [1]
        );
        assert_eq!(
            d.control_in(op::REQ_GET_BAND_BYPASS, at(0, 3), 1).unwrap(),
            [0]
        );
        // A channel past the end stalls, as the firmware's `return false`.
        assert!(
            d.control_in(op::REQ_GET_BAND_BYPASS, at(200, 0), 1)
                .is_err()
        );
    }

    #[test]
    fn wp_upw2_subharmonic_solo_round_trips_without_a_notification() {
        let mut d = FakeDevice::demo();
        let mut events = d.notifications().unwrap();
        assert_eq!(d.control_in(op::REQ_GET_SUBHARM_SOLO, 0, 1).unwrap(), [0]);
        d.control_out(op::REQ_SET_SUBHARM_SOLO, 0, &[1]).unwrap();
        assert_eq!(d.control_in(op::REQ_GET_SUBHARM_SOLO, 0, 1).unwrap(), [1]);
        // Silence reads as an empty packet.
        assert_eq!(
            events.read(Duration::from_millis(20)).unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn wp_upw2_meters_and_usb_error_counters_answer_as_the_firmware_does() {
        let mut d = FakeDevice::demo();
        let outputs = usize::from(d.run(|s| Ok(s.num_outputs())).unwrap());
        let meter = d.control_in_upto(op::REQ_GET_SUBHARM_METER, 0, 64).unwrap();
        assert_eq!(meter, vec![0; outputs * 2]);
        assert_eq!(
            d.control_in(op::REQ_GET_USB_ERROR_STATS, 0, 24).unwrap(),
            [0; 24]
        );
        assert_eq!(
            d.control_in(op::REQ_RESET_USB_ERROR_STATS, 0, 1).unwrap(),
            [1]
        );
    }
}
