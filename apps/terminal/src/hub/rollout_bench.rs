//! WP P3WRO: `--with-rollout-bench`, a test option of `uconsole --demo hub --rollout`
//! for the web console's rollout tests (apps/web/test/e2e/hub_p3wro.spec.ts). It adds
//! one more simulated DSPi "on this hub's USB" (serial [`BENCH_SERIAL`], RP2350, firmware
//! 1.1.6, a K340's 1.20 V core setting) and a fake boot ROM and writer in place of the
//! core's PICOBOOT ones. Nothing touches USB: no device is scanned, opened or written.
//!
//! What the bench does, so a test can drive every outcome from the image it uploads:
//! - Sent to its bootloader, the DSPi leaves the bus and the fake BOOTSEL scan shows one
//!   board with its serial.
//! - The writer takes 2 s. An image of firmware `1.1.N` (N up to 15) brings the DSPi back
//!   running it with its settings kept. Any other version brings it back on its old
//!   firmware with other settings (a botched update: its master volume 10 dB lower), so
//!   the hub's verify fails and says the settings changed.
//! - Its core-voltage reads take [`VREG_READ_DELAY`], as a slow board's might, so a console
//!   has time to cancel a rollout between its check and its bootloader entry.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use dspi_core::{BootDevice, FirmwareGuardBoard, FlashReport};
use dspi_hub::rollout::{BootScan, Writer};
use dspi_hub::{DeviceState, HubHandle};
use dspi_proto::generated::ext as g;
use dspi_proto::generated::opcodes as op;
use dspi_transport::{
    Capture, DeviceDescriptor, FakeDevice, FakeOptions, FakeRemote, NotificationSource, Transport,
    TransportCaps,
};

/// The bench DSPi's serial.
pub(super) const BENCH_SERIAL: &str = "BENCH0000DSPI001";
/// Its USB vendor id: a device "on this hub's USB" (the hub's own scan never sees it).
const BENCH_VID: u16 = 0x2E8B;
/// Its firmware before any rollout: 1.1.6.
const BENCH_PATCH: u8 = 6;
/// How long one of its core-voltage reads takes.
pub(super) const VREG_READ_DELAY: Duration = Duration::from_millis(2500);
/// How long the writer takes.
const WRITE_TIME: Duration = Duration::from_secs(2);
/// The RP2350's platform number (`BootDevice::platform`).
const RP2350: i8 = 1;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The bench DSPi's transport: the demo, noting its bootloader entry and slow to read
/// its core voltage.
struct BenchDevice {
    inner: FakeDevice,
    entered: Arc<AtomicBool>,
}

impl BenchDevice {
    fn intercept(&self, opcode: u8, value: u16) -> Option<Vec<u8>> {
        if opcode == op::REQ_ENTER_BOOTLOADER {
            self.entered.store(true, Ordering::SeqCst);
            return Some(vec![0]);
        }
        if opcode == op::REQ_EXTENSION && value == g::EXT_CORE_VREG {
            std::thread::sleep(VREG_READ_DELAY);
        }
        None
    }
}

impl Transport for BenchDevice {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> dspi_transport::Result<Vec<u8>> {
        match self.intercept(opcode, value) {
            Some(r) => Ok(r),
            None => self.inner.control_in(opcode, value, len),
        }
    }

    fn control_in_upto(
        &mut self,
        opcode: u8,
        value: u16,
        max_len: u16,
    ) -> dspi_transport::Result<Vec<u8>> {
        match self.intercept(opcode, value) {
            Some(r) => Ok(r),
            None => self.inner.control_in_upto(opcode, value, max_len),
        }
    }

    fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> dspi_transport::Result<()> {
        self.inner.control_out(opcode, value, data)
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }

    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        self.inner.notifications()
    }

    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }
}

/// The demo device as firmware `1.1.<patch>` with a 1.20 V core default (a K340).
fn demo_as(patch: u8) -> FakeDevice {
    let mut capture = Capture::demo();
    // `GET_PLATFORM`: major, (minor << 4) | patch, outputs, minor, patch, beta.
    if let Some(b) = capture.platform.get_mut(2) {
        *b = 0x10 | (patch & 0x0F);
    }
    if let Some(b) = capture.platform.get_mut(5) {
        *b = patch;
    }
    let d = FakeDevice::new(
        capture,
        FakeOptions {
            extensions: true,
            surfaces: true,
            analysis: true,
            ..FakeOptions::default()
        },
    )
    .with_serial(BENCH_SERIAL);
    d.remote().vreg_set_build_default(0x0D);
    d
}

/// The patch number of an image of firmware `1.1.N`, else `None`.
fn patch_of(version: &str) -> Option<u8> {
    let n: u8 = version.strip_prefix("1.1.")?.parse().ok()?;
    (n <= 15).then_some(n)
}

#[derive(Default)]
struct State {
    hub: Option<HubHandle>,
    /// The plugged DSPi's: whether it entered its bootloader, its settings' handle.
    entered: Option<Arc<AtomicBool>>,
    remote: Option<FakeRemote>,
    patch: u8,
    /// Its settings when it left for the bootloader.
    kept: Option<Vec<u8>>,
}

/// The bench: one DSPi, its boot ROM and the writer.
#[derive(Default)]
pub(super) struct Bench {
    state: Mutex<State>,
}

impl Bench {
    /// Attach the bench DSPi to `hub` for the first time.
    pub(super) fn start(&self, hub: HubHandle) -> Result<(), String> {
        lock(&self.state).hub = Some(hub);
        self.plug(BENCH_PATCH, None)
    }

    /// Attach the DSPi as firmware `1.1.<patch>`, with `settings` when given.
    fn plug(&self, patch: u8, settings: Option<Vec<u8>>) -> Result<(), String> {
        let dev = demo_as(patch);
        if let Some(image) = settings {
            dev.remote().load_image(image, 0);
        }
        let entered = Arc::new(AtomicBool::new(false));
        let hub = {
            let mut st = lock(&self.state);
            st.entered = Some(Arc::clone(&entered));
            st.remote = Some(dev.remote());
            st.patch = patch;
            st.hub.clone().ok_or("the bench has no hub")?
        };
        hub.attach(
            Box::new(BenchDevice {
                inner: dev,
                entered,
            }),
            BENCH_VID,
        )
        .map(|_| ())
        .map_err(|e| format!("the bench DSPi did not attach: {e}"))
    }
}

impl BootScan for Bench {
    fn boards(&self) -> Vec<BootDevice> {
        let (hub, in_boot, remote) = {
            let st = lock(&self.state);
            let in_boot = st
                .entered
                .as_ref()
                .is_some_and(|e| e.load(Ordering::SeqCst));
            (st.hub.clone(), in_boot, st.remote.clone())
        };
        let Some(hub) = hub else {
            return Vec::new();
        };
        if !in_boot {
            return Vec::new();
        }
        // The DSPi left the bus for its boot ROM; its settings stay in its flash.
        if let Some(d) = hub
            .devices()
            .into_iter()
            .find(|d| d.identity.serial == BENCH_SERIAL && d.state == DeviceState::Online)
        {
            lock(&self.state).kept = remote.map(|r| r.image());
            let _ = hub.detach(d.handle);
        }
        vec![BootDevice {
            platform: RP2350,
            serial: Some(BENCH_SERIAL.into()),
            bus_location: "bench".into(),
        }]
    }
}

impl Writer for Bench {
    fn flash(
        &self,
        image: &[u8],
        target: &BootDevice,
        _board: &FirmwareGuardBoard,
    ) -> Result<FlashReport, String> {
        let info = dspi_core::uf2_inspect(image.to_vec());
        if !info.valid {
            return Err(format!("not a firmware file: {}", info.problem));
        }
        std::thread::sleep(WRITE_TIME);
        let (old, kept) = {
            let mut st = lock(&self.state);
            st.entered = None;
            (st.patch, st.kept.take())
        };
        // Back with the image's firmware and its settings; a version the bench cannot
        // run: back on the old firmware with the demo's settings.
        match patch_of(&info.firmware_version) {
            Some(n) => self.plug(n, kept)?,
            None => self.plug(old, kept.map(|k| botched(&k)))?,
        }
        Ok(FlashReport {
            platform: target.platform,
            serial: target.serial.clone(),
            firmware_version: info.firmware_version,
            sectors: 2,
            bytes_written: u64::try_from(image.len()).unwrap_or(u64::MAX),
            elapsed_ms: u64::try_from(WRITE_TIME.as_millis()).unwrap_or(u64::MAX),
        })
    }
}

/// `settings` after a botched update: the master volume 10 dB lower (the settings as
/// they were when they do not decode).
fn botched(settings: &[u8]) -> Vec<u8> {
    let mut s = dspi_core::state::default_state();
    if dspi_core::state::decode_bulk(settings, &mut s).is_err() {
        return settings.to_vec();
    }
    s.master_volume_db -= 10.0;
    dspi_core::state::encode_bulk(&s)
}

/// Put the bench's scan and writer into the hub's rollout configuration.
pub(super) fn install(bench: &Arc<Bench>, config: &mut dspi_hub::HubConfig) {
    if let Some(r) = config.services.rollout.as_mut() {
        r.boot_scan = Some(Arc::clone(bench) as Arc<dyn BootScan>);
        r.writer = Some(Arc::clone(bench) as Arc<dyn Writer>);
    }
}

#[cfg(test)]
#[path = "tests_wp_p3wro.rs"]
mod tests_wp_p3wro;
