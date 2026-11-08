//! The read-only fallback: the one place a write to such a device is refused.
//!
//! A device on another wire format is shown, not changed: an older image is
//! mapped onto this build's layout and a newer one keeps its prefix
//! ([`dspi_proto::wire_compat`]), and neither may be written back (PLAN §1.5).
//!
//! Writes reach a device by more routes than [`dspi_session::Session::write`]:
//! `Session::send` and `write_band`, the pair map and core voltage over
//! `Session::transport_mut`, control surfaces, macros and groups over
//! `Surfaces`, the analyser's own pushes, the preset flows, and `dspi raw`.
//! All of them end at a [`Transport`], so that is where the refusal sits:
//! [`ReadOnlyTransport`] wraps the link, and nothing else in the terminal needs
//! to know. It also keeps the layers above in step with the mapped image, so a
//! re-read and a notification mean the same on every wire format.
//!
//! **Deny by default.** Some mutating commands are dispatched on the IN path
//! and answer a status byte (`Dir::WriteAsRead`), so the direction proves
//! nothing: an IN goes out only if [`allows_in`] knows it for a read. A command
//! this build has never heard of is refused too, which is what matters on a
//! firmware newer than this build.
//!
//! Owned by WP TM4 (docs/core-merge/PLAN-CLIENTS.md).

use std::time::Duration;

use dspi_proto::generated::opcodes as op;
use dspi_proto::generated::wire::WIRE_FORMAT_VERSION;
use dspi_proto::packets::FirmwareVersion;
use dspi_session::preset_file::{self, PresetDocument};
use dspi_session::surfaces::learn;
use dspi_session::{Capabilities, compat, notify, probe};
use dspi_transport::{
    Attributed, DeviceDescriptor, NotificationSource, Result, Transport, TransportCaps,
    TransportError,
};

use super::connect::printable;

/// The name a refusal uses for this application.
const APP: &str = "DSPi Terminal";

/// Every IN command that only reads: the `read` class of the Link policy table
/// (`tests/fixtures/link/policy/commands.json`, which a test holds this to),
/// plus [`REQ_EXTENSION`](op::REQ_EXTENSION), which that table leaves out and
/// which reads on IN and writes on OUT.
const READS: &[u8] = &[
    op::REQ_EXTENSION,
    op::REQ_OUTPUT_LATENCY,
    op::REQ_GET_CS_AUX_STATE,
    op::REQ_GET_CS_AUX_LEVEL,
    op::REQ_RTA_GET_CONFIG,
    op::REQ_RTA_GET_CAPS,
    op::REQ_RTA_GET_BANDS,
    op::REQ_RTA_GET_BINS,
    op::REQ_RTA_GET_STATUS,
    op::REQ_RTA_GET_BANDS_ALL,
    op::REQ_GET_SUBHARM,
    op::REQ_GET_SUBHARM_LOW,
    op::REQ_GET_SUBHARM_HIGH,
    op::REQ_GET_SUBHARM_BOOST,
    op::REQ_GET_SUBHARM_MASK,
    op::REQ_GET_SUBHARM_HEADROOM,
    op::REQ_GET_SUBHARM_TOP,
    op::REQ_GET_SUBHARM_SELECT,
    op::REQ_GET_SUBHARM_METER,
    op::REQ_GET_CS_GROUP,
    op::REQ_GET_CS_MACRO,
    op::REQ_GET_CS_EXT_STATUS,
    op::REQ_GET_CS_DISPLAY_CFG,
    op::REQ_GET_CS_DISPLAY_PAGE,
    op::REQ_GET_CS_DISPLAY_STATUS,
    op::REQ_GET_SUBHARM_SOLO,
    op::REQ_GET_SUBHARM_LINK,
    op::REQ_GET_PSYBASS,
    op::REQ_GET_PSYBASS_CUTOFF,
    op::REQ_GET_PSYBASS_HARMONICS,
    op::REQ_GET_PSYBASS_DRIVE,
    op::REQ_GET_PSYBASS_CHARACTER,
    op::REQ_GET_PSYBASS_ORIGINAL,
    op::REQ_GET_PSYBASS_MASK,
    op::REQ_GET_TUBE_PARAM,
    op::REQ_GET_EQ_PARAM,
    op::REQ_GET_PREAMP,
    op::REQ_GET_BYPASS,
    op::REQ_GET_DELAY,
    op::REQ_UPMIX_GET_CONFIG,
    op::REQ_UPMIX_GET_PARAM,
    op::REQ_UPMIX_GET_STATUS,
    op::REQ_GET_STATUS,
    op::REQ_GET_INPUT_SOURCE_NAME,
    op::REQ_GET_DEVICE_NAME,
    op::REQ_GET_LOUDNESS,
    op::REQ_GET_LOUDNESS_REF,
    op::REQ_GET_LOUDNESS_INTENSITY,
    op::REQ_GET_CROSSFEED,
    op::REQ_GET_CROSSFEED_PRESET,
    op::REQ_GET_CROSSFEED_FREQ,
    op::REQ_GET_CROSSFEED_FEED,
    op::REQ_GET_CROSSFEED_ITD,
    op::REQ_GET_ADAT_INPUT_ENABLE,
    op::REQ_GET_ADAT_INPUT_PIN,
    op::REQ_GET_ADAT_INPUT_CLOCK_MODE,
    op::REQ_GET_ADAT_INPUT_STATUS,
    op::REQ_GET_MATRIX_ROUTE,
    op::REQ_GET_OUTPUT_ENABLE,
    op::REQ_GET_OUTPUT_GAIN,
    op::REQ_GET_OUTPUT_MUTE,
    op::REQ_GET_OUTPUT_DELAY,
    op::REQ_GET_CORE1_MODE,
    op::REQ_GET_CORE1_CONFLICT,
    op::REQ_GET_OUTPUT_PIN,
    op::REQ_GET_SERIAL,
    op::REQ_GET_PLATFORM,
    op::REQ_GET_BUILD_INFO,
    op::REQ_LIMITER,
    op::REQ_GET_CLOCK_DIAG,
    op::REQ_GET_CS_BINDING,
    op::REQ_GET_CS_CAPS,
    op::REQ_GET_CS_STATUS,
    op::REQ_GET_I2S_CLOCK_MODE,
    op::REQ_GET_I2S_SLAVE_STATUS,
    op::REQ_GET_CS_NAME,
    op::REQ_GET_CS_IR_CMD,
    op::REQ_PRESET_GET_NAME,
    op::REQ_PRESET_GET_DIR,
    op::REQ_PRESET_GET_STARTUP,
    op::REQ_GET_OUTPUT_CONFIG_MODE,
    op::REQ_PRESET_GET_ACTIVE,
    op::REQ_GET_CHANNEL_NAME,
    op::REQ_GET_ALL_PARAMS,
    op::REQ_GET_ALL_PARAMS_CHUNK,
    op::REQ_SIGGEN_GET_CONFIG,
    op::REQ_SIGGEN_GET_STATUS,
    op::REQ_SIGGEN_GET_CAPS,
    op::REQ_GET_SUBHARM_DEPTH,
    op::REQ_GET_SUBHARM_HOLD,
    op::REQ_GET_SUBHARM_CEILING,
    op::REQ_GET_BUFFER_STATS,
    op::REQ_GET_USB_ERROR_STATS,
    op::REQ_GET_LEVELLER_ENABLE,
    op::REQ_GET_LEVELLER_AMOUNT,
    op::REQ_GET_LEVELLER_SPEED,
    op::REQ_GET_LEVELLER_MAX_GAIN,
    op::REQ_GET_LEVELLER_LOOKAHEAD,
    op::REQ_GET_LEVELLER_GATE,
    op::REQ_GET_OUTPUT_TYPE,
    op::REQ_GET_I2S_BCK_PIN,
    op::REQ_GET_MCK_ENABLE,
    op::REQ_GET_MCK_PIN,
    op::REQ_GET_MCK_MULTIPLIER,
    op::REQ_GET_ADAT_ENABLE,
    op::REQ_GET_ADAT_PIN,
    op::REQ_GET_ADAT_STATUS,
    op::REQ_GET_PREAMP_CH,
    op::REQ_GET_MASTER_VOLUME,
    op::REQ_GET_MASTER_VOLUME_MODE,
    op::REQ_GET_SAVED_MASTER_VOLUME,
    op::REQ_GET_BAND_BYPASS,
    op::REQ_GET_USER_VOLUME,
    op::REQ_GET_USER_MUTE,
    op::REQ_GET_LEVELLER_MASKS,
    op::REQ_GET_INPUT_SOURCE,
    op::REQ_GET_SPDIF_RX_STATUS,
    op::REQ_GET_SPDIF_RX_CH_STATUS,
    op::REQ_GET_SPDIF_RX_PIN,
    op::REQ_GET_LG_SOUND_SYNC_ENABLE,
    op::REQ_GET_LG_SOUND_SYNC_STATUS,
    op::REQ_GET_DAC_HW_MUTE_CONFIG,
    op::REQ_GET_INPUT_RATE,
    op::REQ_GET_SPDIF_INPUT_CONFIG,
    op::REQ_GET_I2S_RX_PIN,
    op::REQ_GET_I2S_INPUT_CHANNELS,
    op::REQ_GET_UART_CONFIG,
    op::REQ_GET_I2C_CONFIG,
    op::REQ_GET_CTRL_IFACE_STATUS,
    op::REQ_GET_LOUDNESS_MASK,
    op::REQ_GET_CROSSFEED_OUTPUTS,
    op::REQ_GET_I2S_CLOCK_PIN_MODE,
];

/// State-changing commands PLAN §1.2 allows on a read-only connection, as the
/// core does: the clip latch and the buffer watermarks change no setting, and
/// the bootloader is how such a device gets the firmware that ends the
/// read-only mode.
const HOUSEKEEPING: &[u8] = &[
    op::REQ_CLEAR_CLIPS,
    op::REQ_RESET_BUFFER_STATS,
    op::REQ_ENTER_BOOTLOADER,
];

/// Whether an IN transfer may leave for a read-only device.
pub fn allows_in(opcode: u8, value: u16) -> bool {
    if opcode == op::REQ_CS_IR_LEARN {
        // Three-valued: arm, cancel and read, and only the last leaves the
        // receiver alone (`surfaces::read_learn`).
        return value == learn::READ;
    }
    READS.contains(&opcode) || HOUSEKEEPING.contains(&opcode)
}

/// Whether this device is read-only: its wire format is not this build's.
/// `Capabilities::wire_format` is the device's own version, whatever the image
/// was mapped onto ([`dspi_session::probe::probe_any`]).
pub fn is_read_only(caps: &Capabilities) -> bool {
    u16::from(caps.wire_format) != WIRE_FORMAT_VERSION
}

/// The line every screen shows for a read-only device, most important words
/// first, since an 80-column title row cuts the rest off. `None` when writable.
pub fn banner(caps: &Capabilities) -> Option<String> {
    if !is_read_only(caps) {
        return None;
    }
    let wire = caps.wire_format;
    let firmware = printable(&caps.firmware);
    Some(if u16::from(wire) < WIRE_FORMAT_VERSION {
        format!(
            "READ-ONLY  firmware {firmware} (wire V{wire}) is too old: update it to change settings"
        )
    } else {
        format!(
            "READ-ONLY  firmware {firmware} (wire V{wire}) is too new: update {APP} to change settings"
        )
    })
}

/// What to tell someone exporting a whole-device document that leaves settings
/// out, because the device would not report them and no image holds them. This
/// build leaves such a key alone on import. The other importers do not: the Mac
/// Console sets each to its default, and Qt's does for the keys of a channel or
/// a block it does have. `None` when the document holds everything.
pub fn export_warning(doc: &PresetDocument, path: &str) -> Option<String> {
    let left = preset_file::omitted_settings(doc);
    (!left.is_empty()).then(|| {
        format!(
            "this device does not report {}, so {} leaves them out. Importing it here leaves \
             those settings as they are; DSPi Console on a Mac or on Linux or Windows may set \
             them to its defaults instead.",
            left.join(", "),
            printable(path),
        )
    })
}

/// What a read-only device's bulk reads answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkReads {
    /// The image mapped onto this build's layout: for a [`Session`], whose
    /// strict reader refuses any other (`Session::snapshot`).
    ///
    /// [`Session`]: dspi_session::Session
    Mapped,
    /// The device's own bytes: for `dspi raw`, which exists to show them.
    AsIs,
}

/// A link to a device that must not be changed. See the module's comment.
pub struct ReadOnlyTransport {
    inner: Box<dyn Transport>,
    bulk: BulkReads,
    /// What a refusal says.
    why: String,
    /// The mapped image while a chunked read walks it. Offset 0 takes a fresh
    /// one, the way offset 0 opens a snapshot on the device.
    image: Option<Vec<u8>>,
}

impl ReadOnlyTransport {
    /// Guard `inner`, a link to a device of wire format `wire` (`None` when it
    /// could not be read, which is refused all the same).
    pub fn new(
        inner: Box<dyn Transport>,
        bulk: BulkReads,
        firmware: Option<&FirmwareVersion>,
        wire: Option<u8>,
    ) -> Self {
        let why = match wire {
            Some(wire) => format!(
                "this device is read-only. {}",
                probe::refusal_for(APP, firmware, wire)
            ),
            None => "this device's wire format could not be read, so it is read-only".to_string(),
        };
        Self {
            inner,
            bulk,
            why,
            image: None,
        }
    }

    fn refuse(&self) -> TransportError {
        TransportError::Denied(self.why.clone())
    }

    /// The whole image, mapped onto this build's layout. The compat reader
    /// sizes its own requests by the device's header, so it runs on `inner`.
    fn read_mapped(&mut self) -> Result<Vec<u8>> {
        match compat::read_bulk_any(self.inner.as_mut()) {
            Ok(image) => Ok(image.packet.as_bytes().to_vec()),
            Err(dspi_session::WriteError::Transport(e)) => Err(e),
            Err(other) => Err(TransportError::Malformed(other.to_string())),
        }
    }

    /// A mapped bulk read, `None` for anything else. `Ok` replies hold at most
    /// `len` bytes; the caller decides whether a shorter one is an error.
    fn mapped_bulk(&mut self, opcode: u8, value: u16, len: u16) -> Option<Result<Vec<u8>>> {
        if self.bulk != BulkReads::Mapped {
            return None;
        }
        let len = usize::from(len);
        match opcode {
            op::REQ_GET_ALL_PARAMS => Some(
                self.read_mapped()
                    .map(|image| image.into_iter().take(len).collect()),
            ),
            op::REQ_GET_ALL_PARAMS_CHUNK => {
                let offset = usize::from(value);
                if offset == 0 {
                    self.image = None;
                }
                let image = match self.image.take() {
                    Some(image) => image,
                    None => match self.read_mapped() {
                        Ok(image) => image,
                        Err(e) => return Some(Err(e)),
                    },
                };
                let part: Vec<u8> = image.iter().skip(offset).take(len).copied().collect();
                // Walked to the end: the next read is a new one.
                if offset.saturating_add(part.len()) < image.len() {
                    self.image = Some(image);
                }
                Some(Ok(part))
            }
            _ => None,
        }
    }
}

impl Transport for ReadOnlyTransport {
    fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
        if !allows_in(opcode, value) {
            return Err(self.refuse());
        }
        match self.mapped_bulk(opcode, value, len) {
            // The contract of `control_in`: fewer bytes than asked is an error.
            Some(Ok(part)) if part.len() < usize::from(len) => Err(TransportError::ShortRead {
                opcode,
                wanted: usize::from(len),
                got: part.len(),
            }),
            Some(reply) => reply,
            None => self.inner.control_in(opcode, value, len),
        }
    }

    fn control_in_upto(&mut self, opcode: u8, value: u16, max_len: u16) -> Result<Vec<u8>> {
        if !allows_in(opcode, value) {
            return Err(self.refuse());
        }
        match self.mapped_bulk(opcode, value, max_len) {
            Some(reply) => reply,
            None => self.inner.control_in_upto(opcode, value, max_len),
        }
    }

    fn control_out(&mut self, _opcode: u8, _value: u16, _data: &[u8]) -> Result<()> {
        Err(self.refuse())
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }

    fn max_transfer(&self) -> usize {
        self.inner.max_transfer()
    }

    fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
        let inner = self.inner.notifications()?;
        Some(Box::new(ReadOnlyNotifications { inner }))
    }

    fn caps(&self) -> TransportCaps {
        self.inner.caps()
    }

    fn session_id(&self) -> Option<u16> {
        self.inner.session_id()
    }

    /// A hub's device lock holds other clients off, which is itself a change,
    /// and is taken only around a write. Refused first, so such a flow fails
    /// before its first step.
    ///
    /// -- WP TM25 -- Except under [`BulkReads::AsIs`] (`dspi raw`, `dspi
    /// flash`): the core above sees the device's own image, so it knows the
    /// device is read-only and refuses a write's lock itself; the one it takes
    /// there is the bootloader reboot's (WP CG24), which must still work.
    fn lock(&mut self, reason: &str, timeout: Duration) -> Result<()> {
        if self.bulk == BulkReads::AsIs {
            return self.inner.lock(reason, timeout);
        }
        Err(self.refuse())
    }

    /// Only what [`Self::lock`] passed on.
    fn unlock(&mut self) {
        if self.bulk == BulkReads::AsIs {
            self.inner.unlock();
        }
    }
}

/// A read-only device's notifications, with every `PARAM_CHANGED` turned into
/// the `BULK_INVALIDATED` it means here. Its offset is into the device's image,
/// not the mapped one, so patching the mirror with it would show the wrong
/// field; re-reading is always right (as the core does, §1.5).
struct ReadOnlyNotifications {
    inner: Box<dyn NotificationSource>,
}

impl NotificationSource for ReadOnlyNotifications {
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        self.inner.read(timeout).map(without_offsets)
    }

    fn read_attributed(&mut self, timeout: Duration) -> Result<Attributed> {
        let mut note = self.inner.read_attributed(timeout)?;
        note.packet = without_offsets(note.packet);
        Ok(note)
    }
}

/// `[version, id, flags, seq, offset(2), size(2), source, ..]` becomes
/// `[version, BULK_INVALIDATED, flags, seq, source]`. Anything else, a short
/// packet included, goes to the decoder as it came.
fn without_offsets(packet: Vec<u8>) -> Vec<u8> {
    match packet.as_slice() {
        [version, id, flags, seq, _, _, _, _, source, ..]
            if *version == notify::PROTOCOL_VERSION && *id == notify::evt::PARAM_CHANGED =>
        {
            vec![
                *version,
                notify::evt::BULK_INVALIDATED,
                *flags,
                *seq,
                *source,
            ]
        }
        _ => packet,
    }
}

#[cfg(test)]
mod tests_wp_tm4 {
    use std::collections::{HashMap, HashSet};
    use std::time::Instant;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use dspi_proto::generated::wire::WIRE_FORMAT_VERSION;
    use dspi_proto::registry::{Kind, REGISTRY, Requires};
    use dspi_proto::value::{EqParamPacket, Value};
    use dspi_proto::wire_compat::{bulk_size, normalize};
    use dspi_proto::{FilterType, generated};
    use dspi_session::notify::{Event, decode, evt};
    use dspi_session::preset_file::{self, ApplyOptions};
    use dspi_session::rta::{Budget, Request, RtaEngine};
    use dspi_session::surfaces::{Limits, Surfaces};
    use dspi_session::{DeviceState, Session, ext, surfaces};
    use dspi_transport::mock::{Direction, Exchange, LogHandle, Reply};
    use dspi_transport::{FakeDevice, MockTransport};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use serde_json::Value as Json;

    use super::*;
    use crate::live::{ConsoleScreens, Live};
    use crate::perf::Performance;
    use crate::shell::connect::{self, OpenError, connected_on, guard_raw, session_on};
    use crate::shell::{Tool, fixture};
    use crate::theme::{ColorDepth, Glyphs, Theme};

    const POLICY: &str = include_str!("../../../../tests/fixtures/link/policy/commands.json");

    /// The firmware a section sits at, by name.
    fn off(name: &str) -> usize {
        generated::SECTIONS
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, o, _)| *o)
            .unwrap()
    }

    fn demo_image() -> Vec<u8> {
        FakeDevice::demo().remote().image()
    }

    fn set_header(image: &mut [u8], wire: u8) {
        image[0] = wire;
        let len = image.len() as u16;
        image[6..8].copy_from_slice(&len.to_le_bytes());
    }

    /// A device image of this wire format, built from the demo device's by
    /// undoing what `wire_compat` maps: so a value seen after the mapping
    /// proves it was found in the device's own layout.
    fn image_as(wire: u8) -> Vec<u8> {
        let v32 = demo_image();
        match wire {
            33 => {
                // A newer firmware appends: the V32 prefix, then what it added.
                let mut d = v32;
                d.extend_from_slice(&[0xEE; 16]);
                set_header(&mut d, 33);
                d
            }
            31 => {
                let mut d = v32[..bulk_size(31).unwrap()].to_vec();
                set_header(&mut d, 31);
                d
            }
            17 => {
                let (leveller, preamp, ic, adat) = (
                    off("leveller"),
                    off("preamp"),
                    off("input_config"),
                    off("adat_config"),
                );
                let mut laid = v32.clone();
                // Before V28 the input config's tail sat one byte earlier.
                laid.copy_within(ic + 11..ic + 16, ic + 10);
                let mut d = vec![0u8; bulk_size(17).unwrap()];
                d[..leveller + 16].copy_from_slice(&v32[..leveller + 16]);
                // Before V18 everything from the preamp on sat four earlier.
                d[preamp - 4..adat + 4].copy_from_slice(&laid[preamp..adat + 8]);
                set_header(&mut d, 17);
                d
            }
            other => panic!("no fixture for V{other}"),
        }
    }

    /// A device of this image that answers everything with zeroes, so a write
    /// that reached it would succeed: only the guard can be why none is seen.
    fn device(image: Vec<u8>) -> (MockTransport, LogHandle) {
        let platform = FakeDevice::demo()
            .control_in_upto(op::REQ_GET_PLATFORM, 0, 7)
            .unwrap();
        let mock = MockTransport::new()
            .data(op::REQ_GET_PLATFORM, platform)
            .reply(op::REQ_GET_ALL_PARAMS, Reply::Data(image.clone()))
            .window(op::REQ_GET_ALL_PARAMS_CHUNK, image)
            .answering_everything(vec![0u8; 512]);
        let log = mock.log_handle();
        (mock, log)
    }

    /// Anything on `log` that a read-only device must never see.
    fn writes(log: &LogHandle) -> Vec<Exchange> {
        log.lock()
            .unwrap()
            .iter()
            .filter(|e| match e.direction {
                Direction::Out => true,
                Direction::In => !allows_in(e.opcode, e.value) || HOUSEKEEPING.contains(&e.opcode),
            })
            .cloned()
            .collect()
    }

    fn read_only_session(wire: u8) -> (Session, LogHandle) {
        let (mock, log) = device(image_as(wire));
        (session_on(Box::new(mock)).unwrap(), log)
    }

    #[test]
    fn wp_tm4_v17_v31_v33_connect_as_their_own_format_and_read_only() {
        let demo = demo_image();
        for wire in [17, 31, 33] {
            let (mock, log) = device(image_as(wire));
            let mut c = connected_on(Box::new(mock)).unwrap();
            assert_eq!(c.state.caps.wire_format, wire);
            assert!(is_read_only(&c.state.caps), "V{wire}");

            // The mapping found the device's values where its format has them.
            let bytes = c.state.bulk.as_bytes();
            assert_eq!(
                bytes[off("channel_names")..off("channel_names") + 64],
                demo[off("channel_names")..off("channel_names") + 64],
                "V{wire}"
            );
            assert_eq!(
                bytes[off("preamp")..off("preamp") + 8],
                demo[off("preamp")..off("preamp") + 8],
                "V{wire}"
            );
            assert_eq!(bytes[0] as u16, WIRE_FORMAT_VERSION, "the mapped layout");

            // A re-read goes through the strict reader above the guard.
            let again = c.session.snapshot().unwrap();
            assert_eq!(again.as_bytes(), c.state.bulk.as_bytes(), "V{wire}");

            assert_eq!(writes(&log), vec![], "V{wire}: connecting wrote");
        }
    }

    #[test]
    fn wp_tm4_v32_is_left_alone_and_can_be_written() {
        let (mock, log) = device(demo_image());
        let mut c = connected_on(Box::new(mock)).unwrap();
        assert!(!is_read_only(&c.state.caps));
        assert_eq!(banner(&c.state.caps), None);
        let _ = c.session.write("vol.master", &[], Value::Float(-20.0));
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|e| e.direction == Direction::Out),
            "a V32 device takes writes"
        );
    }

    #[test]
    fn wp_tm4_an_image_that_cannot_be_mapped_is_still_refused_by_name() {
        let mut image = vec![0u8; 512];
        image[0] = 15;
        let (mock, _) = device(image);
        let Err(e) = session_on(Box::new(mock)) else {
            panic!("a V15 device opened");
        };
        assert!(matches!(
            e,
            OpenError::Transport(TransportError::Incompatible(_))
        ));
        assert!(e.to_string().contains("DSPi Terminal"), "{e}");
        assert!(e.is_final(), "not worth retrying");
    }

    /// Draw a frame, as text by row.
    fn rows(live: &mut Live, w: u16, h: u16) -> Vec<String> {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        live.draw(area, &mut buf);
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn live_on(wire: u8) -> (Live, Session, LogHandle) {
        let (mock, log) = device(image_as(wire));
        let c = connected_on(Box::new(mock)).unwrap();
        let theme = Theme::console(ColorDepth::TrueColor, Glyphs::Braille);
        let live = Live::new(
            c.state,
            theme,
            Performance::lite(),
            Box::new(ConsoleScreens::new()),
        );
        (live, c.session, log)
    }

    /// Done criterion: the fixtures render. Every screen the keys reach, at the
    /// minimum size and the reference size, with the device being polled, shows
    /// the banner and sends nothing.
    #[test]
    fn wp_tm4_every_screen_renders_the_fixtures_under_the_banner() {
        for (wire, words) in [(17, "too old"), (31, "too old"), (33, "too new")] {
            let (mut live, mut session, log) = live_on(wire);
            let mut seen = 0;
            let check = |live: &mut Live, what: &str| {
                for (w, h) in [(80, 24), (120, 40)] {
                    let frame = rows(live, w, h);
                    assert!(
                        frame[0].contains("READ-ONLY") && frame[0].contains(&format!("V{wire}")),
                        "V{wire} {what} {w}x{h}: {}",
                        frame[0]
                    );
                    if w > 80 {
                        assert!(frame[0].contains(words), "V{wire} {what}: {}", frame[0]);
                    }
                }
            };
            check(&mut live, "overview");

            // Down the sidebar: the inputs, the outputs.
            for _ in 0..24 {
                live.handle_key(&mut session, key(KeyCode::Down));
                live.tick(&mut session, None);
                check(&mut live, "a channel");
            }

            for c in 'A'..='Z' {
                let Some(tool) = Tool::from_key(c) else {
                    continue;
                };
                live.handle_key(&mut session, key(KeyCode::Char(c)));
                live.tick(&mut session, None);
                check(&mut live, tool.title());
                live.handle_key(&mut session, key(KeyCode::Esc));
                seen += 1;
            }
            assert!(seen >= 13, "only {seen} tools");

            // Settings has the whole screen, and keeps the banner.
            live.handle_key(&mut session, key(KeyCode::Char(',')));
            live.tick(&mut session, None);
            let frame = rows(&mut live, 120, 40);
            assert!(frame[0].contains("Settings"), "{}", frame[0]);
            check(&mut live, "settings");

            assert_eq!(writes(&log), vec![], "V{wire}: looking wrote");
        }
    }

    #[test]
    fn wp_tm4_a_writable_device_has_no_banner() {
        let mut caps = fixture::caps();
        assert_eq!(banner(&caps), None);
        caps.wire_format = 17;
        caps.firmware = "1.0\u{1b}[2J".into();
        let b = banner(&caps).unwrap();
        assert!(b.contains("too old"), "{b}");
        assert!(!b.contains('\u{1b}'), "device text is cleaned: {b}");
    }

    #[test]
    fn wp_tm4_a_refused_edit_says_why_and_changes_nothing() {
        let (mut live, mut session, log) = live_on(17);
        let before = live.state.bulk.as_bytes().to_vec();
        live.set(&mut session, "vol.master", &[], Value::Float(-1.0));
        assert_eq!(writes(&log), vec![]);
        assert_eq!(
            live.state.bulk.as_bytes(),
            before,
            "the mirror did not move"
        );
        let frame = rows(&mut live, 200, 60).join("\n");
        assert!(frame.contains("read-only"), "{frame}");
    }

    /// What `dspi raw` and everything else that fails hears.
    #[test]
    fn wp_tm4_a_refusal_is_typed_and_names_the_firmware() {
        let (mut session, _) = read_only_session(17);
        let e = session.transport_mut().control_out(1, 0, &[0]).unwrap_err();
        let TransportError::Denied(why) = &e else {
            panic!("{e:?}");
        };
        assert!(why.contains("read-only") && why.contains("V17"), "{why}");
        assert!(why.contains("Update the device's firmware"), "{why}");

        let (mut session, _) = read_only_session(33);
        let e = session.transport_mut().control_in(0x53, 0, 1).unwrap_err();
        assert!(e.to_string().contains("Update DSPi Terminal"), "{e}");
    }

    #[test]
    fn wp_tm4_housekeeping_is_the_plans_short_list() {
        let (mut session, log) = read_only_session(17);
        let t = session.transport_mut();
        for opcode in [
            op::REQ_CLEAR_CLIPS,
            op::REQ_RESET_BUFFER_STATS,
            op::REQ_ENTER_BOOTLOADER,
        ] {
            assert!(t.control_in(opcode, 0, 1).is_ok(), "0x{opcode:02X}");
        }
        // Resets diagnostics too, but PLAN §1.2 has no line for it.
        assert!(t.control_in(op::REQ_RESET_USB_ERROR_STATS, 0, 1).is_err());
        assert_eq!(writes(&log).len(), 3);
    }

    /// Runs `route` on a read-only and on a writable session. On the writable
    /// one the write must reach the wire, or the test proves nothing; on the
    /// read-only one it must not.
    fn assert_refused(name: &str, route: impl Fn(&mut Session)) {
        let (mock, log) = device(demo_image());
        let mut session = session_on(Box::new(mock)).unwrap();
        let mark = log.lock().unwrap().len();
        route(&mut session);
        let reached = log.lock().unwrap().split_off(mark);
        assert!(
            reached
                .iter()
                .any(|e| e.direction == Direction::Out || !allows_in(e.opcode, e.value)),
            "{name}: writes nothing even when it may, so this proves nothing: {reached:?}"
        );

        for wire in [17, 31, 33] {
            let (mut session, log) = read_only_session(wire);
            route(&mut session);
            assert_eq!(writes(&log), vec![], "{name} wrote to V{wire}");
        }
    }

    /// Every route found outside `Session::write`. See the WP's report.
    #[test]
    fn wp_tm4_no_route_around_session_write_reaches_the_device() {
        let band = EqParamPacket {
            channel: 0,
            band: 0,
            filter_type: FilterType::from_raw(1),
            bypass: false,
            freq: 1000.0,
            q: 1.0,
            gain_db: 1.0,
            qp: None,
        };
        assert_refused("Session::write_band", |s| {
            let _ = s.write_band(&band);
        });
        assert_refused("Session::enable_output_confirmed", |s| {
            let _ = s.enable_output_confirmed(0);
        });

        // Pair map: screens/crossfeed.rs write_map and `dspi ext xfeed-map`.
        assert_refused("ext::write_pair_map, transport_mut", |s| {
            let _ = ext::write_pair_map(s.transport_mut(), ext::PairMap([2, 0x13, 0, 0]));
        });
        assert_refused("ext::write_pair_map, with_transport", |s| {
            let _ = s.with_transport(|t| {
                let _ = ext::write_pair_map(t, ext::PairMap([2, 0x13, 0, 0]));
                Ok(())
            });
        });
        // Core voltage: `dspi ext vreg`, settings/overview.rs.
        assert_refused("ext::core_vreg_op", |s| {
            // With the word, so only the guard stands between it and the wire.
            let _ = ext::core_vreg_op(
                s.transport_mut(),
                ext::VregOp::Trial(0x0E),
                Some("OVERVOLT"),
            );
        });

        // Control surfaces, macros, groups, display, aux: screens and `cs`.
        fn on_surfaces(s: &mut Session, f: impl FnOnce(&mut Surfaces<'_>)) {
            let mut surfaces =
                Surfaces::new(s.transport_mut(), Limits::default()).with_poll(Duration::ZERO, 1);
            f(&mut surfaces);
        }
        assert_refused("Surfaces::write_binding", |s| {
            on_surfaces(s, |c| drop(c.write_binding(0, &Default::default())));
        });
        assert_refused("Surfaces::write_name", |s| {
            on_surfaces(s, |c| drop(c.write_name(0, "x")));
        });
        assert_refused("Surfaces::write_ir_command", |s| {
            on_surfaces(s, |c| drop(c.write_ir_command(0, &Default::default())));
        });
        assert_refused("Surfaces::write_group", |s| {
            on_surfaces(s, |c| drop(c.write_group(0, &Default::default())));
        });
        assert_refused("Surfaces::write_macro", |s| {
            on_surfaces(s, |c| drop(c.write_macro(0, &Default::default(), &[])));
        });
        assert_refused("Surfaces::write_macro_step", |s| {
            on_surfaces(s, |c| drop(c.write_macro_step(0, 0, &Default::default())));
        });
        assert_refused("Surfaces::clear_macro_step", |s| {
            on_surfaces(s, |c| drop(c.clear_macro_step(0, 0)));
        });
        assert_refused("Surfaces::fire_macro", |s| {
            on_surfaces(s, |c| drop(c.fire_macro(0)));
        });
        assert_refused("Surfaces::cancel_macro", |s| {
            on_surfaces(s, |c| drop(c.cancel_macro()));
        });
        assert_refused("Surfaces::write_display_cfg", |s| {
            on_surfaces(s, |c| drop(c.write_display_cfg(&Default::default())));
        });
        assert_refused("Surfaces::write_display_page", |s| {
            on_surfaces(s, |c| drop(c.write_display_page(0, &Default::default())));
        });
        assert_refused("Surfaces::set_aux_state", |s| {
            on_surfaces(s, |c| drop(c.set_aux_state(0, true)));
        });
        assert_refused("Surfaces::set_aux_level", |s| {
            on_surfaces(s, |c| drop(c.set_aux_level(0, 256)));
        });
        assert_refused("Surfaces::save", |s| {
            on_surfaces(s, |c| drop(c.save()));
        });
        assert_refused("Surfaces::revert", |s| {
            on_surfaces(s, |c| drop(c.revert()));
        });
        assert_refused("Surfaces::arm_learn", |s| {
            on_surfaces(s, |c| drop(c.arm_learn()));
        });
        assert_refused("Surfaces::cancel_learn", |s| {
            on_surfaces(s, |c| drop(c.cancel_learn()));
        });
        assert_refused("surfaces::arm_learn", |s| {
            let _ = surfaces::arm_learn(s.transport_mut());
        });
        assert_refused("surfaces::cancel_learn", |s| {
            let _ = surfaces::cancel_learn(s.transport_mut());
        });
        assert_refused("surfaces::save", |s| {
            let _ = surfaces::save(s.transport_mut());
        });
        assert_refused("surfaces::revert", |s| {
            let _ = surfaces::revert(s.transport_mut());
        });

        // The model's own entry points: `dspi cs import --save`, the IR screen.
        assert_refused("cs_model::apply", |s| {
            let _ = dspi_session::cs_model::apply(s, |c| c.save());
        });
        assert_refused("cs_model::apply_learn", |s| {
            let _ = dspi_session::cs_model::apply_learn(s, true);
        });

        // The analyser pushes its own configuration, and stops the device.
        assert_refused("RtaEngine::tick", |s| {
            let mut engine = RtaEngine::new();
            engine.load(rta_caps(), (0..37).map(|i| 10 + i * 100).collect());
            let id = engine.subscribe(Request {
                tap: 0,
                mask: 1,
                wants_bins: false,
            });
            let budget = Budget {
                max_bin_chunks: 1,
                max_time: Duration::from_millis(50),
            };
            let now = Instant::now();
            engine.tick(s.transport_mut(), now, budget);
            engine.release(id);
            engine.tick(s.transport_mut(), now, budget);
        });

        // A preset document's whole apply, hardware I/O included.
        assert_refused("preset_file::apply", |s| {
            let Ok(doc) = preset_file::capture(s, None) else {
                return;
            };
            let _ = preset_file::apply(
                s,
                &doc,
                ApplyOptions {
                    audio_processing: true,
                    volume_levels: true,
                    hardware_io: true,
                },
            );
        });

        // `dspi raw out`, `dspi raw war`, and a raw IN of a mutating opcode.
        assert_refused("raw OUT", |s| {
            let _ = s
                .transport_mut()
                .control_out(op::REQ_SET_PREAMP, 0, &[0; 4]);
        });
        assert_refused("raw write-as-read", |s| {
            let _ = s.transport_mut().control_in(op::REQ_FACTORY_RESET, 0, 1);
        });
        assert_refused("raw IN, shorter reply allowed", |s| {
            let _ = s.transport_mut().control_in_upto(op::REQ_SAVE_PARAMS, 0, 1);
        });
    }

    /// The preset load the TUI issues itself (live.rs `load_preset`) and the
    /// writes of the TUI's own `set`.
    #[test]
    fn wp_tm4_the_interface_cannot_write_either() {
        let act = |live: &mut Live, session: &mut Session| {
            live.set(session, "preset.load", &[1], Value::Trigger);
            live.set(session, "vol.master", &[], Value::Float(-3.0));
            live.tick(session, None);
        };

        // The control: on this build's format both reach the wire.
        let (mock, log) = device(demo_image());
        let c = connected_on(Box::new(mock)).unwrap();
        let (mut session, theme) = (
            c.session,
            Theme::console(ColorDepth::TrueColor, Glyphs::Braille),
        );
        let mut live = Live::new(
            c.state,
            theme,
            Performance::lite(),
            Box::new(ConsoleScreens::new()),
        );
        act(&mut live, &mut session);
        let sent = log.lock().unwrap().clone();
        assert!(
            sent.iter().any(|e| e.opcode == op::REQ_PRESET_LOAD),
            "{sent:?}"
        );
        assert!(
            sent.iter().any(|e| e.direction == Direction::Out),
            "{sent:?}"
        );

        for wire in [17, 31, 33] {
            let (mut live, mut session, log) = live_on(wire);
            act(&mut live, &mut session);
            assert_eq!(writes(&log), vec![], "V{wire}");
        }
    }

    fn rta_caps() -> dspi_proto::packets::RtaCaps {
        dspi_proto::packets::RtaCaps {
            input_channels: 8,
            output_channels: 9,
            order_min: 8,
            order_max: 10,
            order_default: 10,
            bass_bands: 14,
            max_bands: 37,
            level_zero: 243,
            dynamic_range_db: 120,
            idle_timeout_ms: 5000,
            max_bin_frame: 529,
            bass_dynamic_range_db: 70,
        }
    }

    /// Caps on which every registry row is available.
    fn everything_caps(wire: u8) -> Capabilities {
        let mut caps = fixture::caps();
        caps.wire_format = wire;
        caps.platform = dspi_proto::Platform::Rp2350;
        caps.cs = Some(dspi_session::probe::ControlSurfaceCaps {
            caps_version: 10,
            max_bindings: 16,
            type_count: 0,
            noun_count: 1,
            max_ir_commands: 16,
            max_groups: 8,
            max_macros: 8,
            max_macro_steps: 8,
            max_pages: 16,
            display_models: 0,
            types: Vec::new(),
        });
        for d in REGISTRY {
            if let Requires::Feature(name) = d.requires
                && !caps.features.iter().any(|f| f.name == name && f.present)
            {
                caps.features.retain(|f| f.name != name);
                caps.features.push(dspi_session::probe::Feature {
                    name: name.into(),
                    present: true,
                    evidence: "test".into(),
                });
            }
        }
        caps
    }

    /// A value the registry accepts for this row.
    fn sample(kind: Kind) -> Value {
        match kind {
            Kind::Bool => Value::Bool(true),
            Kind::Trigger | Kind::Status => Value::Trigger,
            Kind::Float { min, max, .. } => Value::Float((min + max) / 2.0),
            Kind::Int { min, .. } => Value::Int(min),
            Kind::Choice(list) => Value::Choice(list[0].0),
            Kind::Mask => Value::Mask(1),
            Kind::Text { .. } => Value::Text("x".into()),
            Kind::Packet => Value::Bytes(vec![0; 16]),
        }
    }

    /// Every setter the registry has, through both of `Session`'s write paths:
    /// nothing leaves, and on a writable device each of them would have.
    #[test]
    fn wp_tm4_no_registry_row_reaches_the_device() {
        let mut tried = 0;
        let mut skipped = Vec::new();
        let mut housekeeping = HashSet::new();
        for d in REGISTRY.iter().filter(|d| d.is_writable()) {
            // S/PDIF input 0 is always on, so its enable starts at 1.
            let first = u8::from(d.path == "in.spdif.enable");
            let indices = vec![first; d.target.arity()];
            // -- WP CG37 -- a delay's midpoint (42.5 ms) is past every line.
            let value = if dspi_cmd::is_delay(d.path) {
                Value::Float(1.0)
            } else {
                sample(d.kind)
            };
            let is_exception = d.set.is_some_and(|s| HOUSEKEEPING.contains(&s));

            // The control: on a writable device this row does write.
            // -- WP CG34 -- eq.fp and eq.qp only on a Linkwitz Transform band
            // (type 11, the first byte of the band's type read).
            let mut answer = vec![0u8; 512];
            if matches!(d.path, "eq.fp" | "eq.qp") {
                answer[0] = 11;
            }
            let mock = MockTransport::new().answering_everything(answer.clone());
            let log = mock.log_handle();
            let mut s = Session::new(Box::new(mock), everything_caps(32)).unwrap();
            if s.check(d.path, &indices, &value).is_err() {
                // Refused above the wire on any device (a target this fixture
                // has none of, say); nothing to learn from it.
                skipped.push(d.path);
                continue;
            }
            let _ = s.write(d.path, &indices, value.clone());
            assert!(
                log.lock()
                    .unwrap()
                    .iter()
                    .any(|e| e.direction == Direction::Out || !allows_in(e.opcode, e.value))
                    || is_exception,
                "{} writes nothing on a writable device",
                d.path
            );
            tried += 1;

            for send in [false, true] {
                // -- WP CG35 -- the same answer as the control (review c6v
                // CG34-N1), so eq.fp/eq.qp pass the type guard and reach the
                // ReadOnlyTransport, which must be what refuses them.
                let mock = MockTransport::new().answering_everything(answer.clone());
                let log = mock.log_handle();
                let guarded =
                    ReadOnlyTransport::new(Box::new(mock), BulkReads::Mapped, None, Some(17));
                let mut s = Session::new(Box::new(guarded), everything_caps(17)).unwrap();
                let r = if send {
                    s.send(d.path, &indices, value.clone()).map(|_| ())
                } else {
                    s.write(d.path, &indices, value.clone()).map(|_| ())
                };
                if matches!(d.path, "eq.fp" | "eq.qp") {
                    assert!(
                        matches!(
                            r,
                            Err(dspi_session::WriteError::Transport(TransportError::Denied(
                                _
                            )))
                        ),
                        "{} (send: {send}) must be refused by the guard: {r:?}",
                        d.path
                    );
                }
                let leaked = writes(&log);
                if is_exception {
                    housekeeping.insert(d.path);
                    assert!(
                        leaked.iter().all(|e| HOUSEKEEPING.contains(&e.opcode)),
                        "{}: {leaked:?}",
                        d.path
                    );
                } else {
                    assert_eq!(leaked, vec![], "{} (send: {send})", d.path);
                }
            }
        }
        assert!(tried > 100, "only {tried} rows tried");
        // The rows the plan's exceptions make writable, and no others.
        assert_eq!(skipped, Vec::<&str>::new());
        assert_eq!(
            housekeeping,
            HashSet::from(["dev.bootloader", "meters.clear", "diag.buffers.reset"])
        );
    }

    /// The table against the Link policy: every read in it is allowed, and
    /// nothing else is but the named exceptions.
    #[test]
    fn wp_tm4_the_read_table_is_the_link_policys() {
        let policy: Json = serde_json::from_str(POLICY).unwrap();
        let mut classes: HashMap<u8, String> = HashMap::new();
        for c in policy["commands"].as_array().unwrap() {
            let has_get = c["dirs"].as_array().unwrap().iter().any(|d| d == "get");
            if has_get {
                let class = c["classes"]["get"]
                    .as_str()
                    .or(c["class"].as_str())
                    .unwrap();
                classes.insert(c["code"].as_u64().unwrap() as u8, class.to_owned());
            }
        }
        assert!(classes.len() > 100);
        for (&code, class) in &classes {
            let allowed = allows_in(code, 0);
            if code == op::REQ_CS_IR_LEARN {
                assert!(!allowed && allows_in(code, 1) == (learn::READ == 1));
                assert!(allows_in(code, learn::READ));
                assert!(!allows_in(code, learn::ARM) && !allows_in(code, learn::CANCEL));
            } else if HOUSEKEEPING.contains(&code) {
                assert!(allowed, "0x{code:02X}");
            } else {
                assert_eq!(allowed, class == "read", "0x{code:02X} is {class}");
            }
        }
        for &code in READS {
            assert!(
                code == op::REQ_EXTENSION || classes.get(&code).is_some_and(|c| c == "read"),
                "0x{code:02X} is not a read"
            );
        }
        // An opcode nobody has heard of stays on the device's side of the line.
        let unknown: Vec<u8> = (0..=255u8)
            .filter(|c| !classes.contains_key(c) && *c != op::REQ_EXTENSION)
            .collect();
        assert!(unknown.iter().all(|&c| !allows_in(c, 0)), "{unknown:?}");
    }

    #[test]
    fn wp_tm4_a_change_notification_becomes_a_re_read() {
        let (mock, _) = device(image_as(17));
        let param = vec![
            2,
            evt::PARAM_CHANGED,
            0,
            9,
            0x10,
            0,
            4,
            0,
            3,
            0,
            0,
            0,
            1,
            2,
            3,
            4,
        ];
        mock.push_notification(param.clone());
        mock.push_notification(vec![2, evt::PRESET_LOADED, 0, 10, 4]);
        mock.push_notification(vec![2, evt::PARAM_CHANGED, 0]);

        let mut session = session_on(Box::new(mock)).unwrap();
        let mut notes = session.transport_mut().notifications().unwrap();
        let timeout = Duration::from_millis(1);

        let first = notes.read(timeout).unwrap();
        let (seq, event) = decode(&first).unwrap();
        assert_eq!(seq, 9, "the sequence still counts");
        let Event::BulkInvalidated { source } = event else {
            panic!("{event:?}");
        };
        assert_eq!(source.raw(), 3, "and whose change it was");

        assert_eq!(
            decode(&notes.read_attributed(timeout).unwrap().packet)
                .unwrap()
                .1,
            Event::PresetLoaded { slot: 4 },
            "the rest pass"
        );
        assert_eq!(notes.read(timeout).unwrap(), vec![2, evt::PARAM_CHANGED, 0]);

        // On a writable device the same packet is left to patch the mirror.
        let (mock, _) = device(demo_image());
        mock.push_notification(param.clone());
        let mut session = session_on(Box::new(mock)).unwrap();
        let mut notes = session.transport_mut().notifications().unwrap();
        assert_eq!(notes.read(timeout).unwrap(), param);
    }

    #[test]
    fn wp_tm4_a_hub_lock_is_refused_and_a_read_is_not() {
        let (mut session, log) = read_only_session(31);
        let t = session.transport_mut();
        assert!(matches!(
            t.lock("preset", Duration::from_millis(1)),
            Err(TransportError::Denied(_))
        ));
        t.unlock();
        assert!(t.control_in(op::REQ_GET_STATUS, 0, 4).is_ok());
        assert_eq!(writes(&log), vec![]);
    }

    #[test]
    fn wp_tm4_a_single_shot_link_reads_the_mapped_image_too() {
        // `TransportCaps::chunked_bulk` false (MIDI, DSPi Link).
        struct SingleShot(MockTransport);
        impl Transport for SingleShot {
            fn control_in(&mut self, o: u8, v: u16, l: u16) -> Result<Vec<u8>> {
                self.0.control_in(o, v, l)
            }
            fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> Result<Vec<u8>> {
                self.0.control_in_upto(o, v, l)
            }
            fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> Result<()> {
                self.0.control_out(o, v, d)
            }
            fn descriptor(&self) -> &DeviceDescriptor {
                self.0.descriptor()
            }
            fn max_transfer(&self) -> usize {
                8192
            }
            fn caps(&self) -> TransportCaps {
                TransportCaps {
                    chunked_bulk: false,
                    ..TransportCaps::USB
                }
            }
        }
        let image = image_as(17);
        let (mock, log) = device(image.clone());
        let mut c = connected_on(Box::new(SingleShot(mock))).unwrap();
        let again = c.session.snapshot().unwrap();
        assert_eq!(
            again.as_bytes(),
            normalize(&image).unwrap().packet.as_bytes()
        );
        assert_eq!(writes(&log), vec![]);
        let reads = log.lock().unwrap();
        assert!(
            reads
                .iter()
                .all(|e| e.opcode != op::REQ_GET_ALL_PARAMS_CHUNK),
            "chunks on a link without them"
        );
    }

    // `dspi raw`

    #[test]
    fn wp_tm4_raw_is_guarded_and_shows_the_devices_own_bytes() {
        let image = image_as(17);
        let (mock, log) = device(image.clone());
        let mut t = guard_raw(Box::new(mock)).unwrap();
        assert!(matches!(
            t.control_out(op::REQ_SET_PREAMP, 0, &[0; 4]),
            Err(TransportError::Denied(_))
        ));
        assert!(matches!(
            t.control_in(op::REQ_SAVE_PARAMS, 0, 1),
            Err(TransportError::Denied(_))
        ));
        assert_eq!(writes(&log), vec![]);
        let head = t.control_in(op::REQ_GET_ALL_PARAMS_CHUNK, 0, 8).unwrap();
        assert_eq!(head[0], 17, "not the mapped header");
        assert_eq!(head, image[..8]);
    }

    #[test]
    fn wp_tm4_raw_on_this_builds_format_is_not_guarded() {
        let (mock, log) = device(demo_image());
        let mut t = guard_raw(Box::new(mock)).unwrap();
        t.control_out(op::REQ_SET_PREAMP, 0, &[0; 4]).unwrap();
        assert_eq!(writes(&log).len(), 1);
    }

    #[test]
    fn wp_tm4_raw_fails_closed_when_the_format_cannot_be_read() {
        let mock = MockTransport::new().data(op::REQ_GET_STATUS, vec![7; 4]);
        let log = mock.log_handle();
        let mut t = guard_raw(Box::new(mock)).unwrap();
        let e = t.control_out(op::REQ_SET_PREAMP, 0, &[0; 4]).unwrap_err();
        assert!(e.to_string().contains("could not be read"), "{e}");
        assert_eq!(t.control_in(op::REQ_GET_STATUS, 0, 4).unwrap(), vec![7; 4]);
        assert_eq!(writes(&log), vec![]);
    }

    #[test]
    fn wp_tm4_raw_reports_a_lost_link() {
        let mock = MockTransport::new().reply(op::REQ_GET_ALL_PARAMS_CHUNK, Reply::Disconnect);
        assert!(matches!(
            guard_raw(Box::new(mock)),
            Err(OpenError::Transport(TransportError::Disconnected))
        ));
    }

    /// Unlike `device`: an opcode with no answer stalls, as firmware does. Over
    /// MIDI a stall is final, which spares a test USB's retries.
    struct Midi(MockTransport);
    impl Transport for Midi {
        fn control_in(&mut self, o: u8, v: u16, l: u16) -> Result<Vec<u8>> {
            self.0.control_in(o, v, l)
        }
        fn control_in_upto(&mut self, o: u8, v: u16, l: u16) -> Result<Vec<u8>> {
            self.0.control_in_upto(o, v, l)
        }
        fn control_out(&mut self, o: u8, v: u16, d: &[u8]) -> Result<()> {
            self.0.control_out(o, v, d)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.0.descriptor()
        }
        fn max_transfer(&self) -> usize {
            8192
        }
        fn caps(&self) -> TransportCaps {
            TransportCaps {
                chunked_bulk: false,
                kind: dspi_transport::TransportKind::Midi,
                ..TransportCaps::USB
            }
        }
    }

    /// A device of this image that refuses every read but the bulk one.
    fn refusing(image: Vec<u8>) -> connect::Connected {
        let platform = FakeDevice::demo()
            .control_in_upto(op::REQ_GET_PLATFORM, 0, 7)
            .unwrap();
        let old = MockTransport::new()
            .data(op::REQ_GET_PLATFORM, platform)
            .data(op::REQ_GET_ALL_PARAMS, image);
        connected_on(Box::new(Midi(old))).unwrap()
    }

    /// Old firmware refuses what it has no getter for. The value comes from the
    /// image instead of coming out as zero, and what no image of that age holds
    /// is left out: importing it then writes the device's real values and nothing
    /// else (WP TM21; TM4 found the export holding 0 dB for a -12 dB master).
    #[test]
    fn wp_tm21_a_preset_export_from_old_firmware_takes_the_image_or_omits() {
        let mut image = image_as(17);
        // Older images have every section from the preamp on four bytes earlier
        // than the V32 offsets `off` gives (`wire_compat`).
        let master = off("master_volume") - 4;
        image[master..master + 4].copy_from_slice(&(-12.0f32).to_le_bytes());
        let user = off("user_volume") - 4;
        image[user..user + 4].copy_from_slice(&(-9.0f32).to_le_bytes());
        let outputs = off("outputs");
        image[outputs + 4..outputs + 8].copy_from_slice(&(-3.0f32).to_le_bytes());
        let mut c = refusing(image);
        assert_eq!(c.state.caps.wire_format, 17);

        let doc = preset_file::capture(&mut c.session, None).unwrap();
        assert_eq!(doc.global.master_volume_db, Some(-12.0), "the device's");
        assert_eq!(doc.global.user_volume_db, Some(-9.0), "the device's");
        let first = doc.channels.iter().find(|c| c.is_output).unwrap();
        assert_eq!(first.gain_db, Some(-3.0), "the device's");
        assert_eq!(doc.meta.wire_format_version, 17);
        assert!(!doc.matrix.is_empty(), "the image holds the matrix");
        assert!(!first.eq.is_empty(), "and the bands");
        // V18, V19 and V20 added these, so a V17 image holds zeros there.
        assert_eq!(doc.leveller.detector_mask, None);
        assert_eq!(doc.loudness.output_mask, None);
        assert_eq!(doc.crossfeed.output_pair_mask, None);
        // And no image holds this.
        assert_eq!(doc.meta.master_volume_mode, None);

        // The warning is down to what is still true.
        let warning = export_warning(&doc, "x.dspipreset").unwrap();
        assert!(warning.contains("x.dspipreset"), "{warning}");
        assert!(warning.contains("leveller.detectorMask"), "{warning}");
        assert!(!warning.contains("masterVolumeDb"), "{warning}");

        // Imported on a writable device: the real values, and no invented mask.
        let (mock, log) = device(demo_image());
        let mut target = session_on(Box::new(mock)).unwrap();
        let report = preset_file::apply(
            &mut target,
            &doc,
            ApplyOptions {
                audio_processing: true,
                volume_levels: true,
                hardware_io: false,
            },
        );
        let log = log.lock().unwrap();
        let sent = |opcode: u8| {
            log.iter()
                .filter(|e| e.direction == Direction::Out && e.opcode == opcode)
                .map(|e| e.payload.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            sent(op::REQ_SET_OUTPUT_GAIN).first().unwrap(),
            &(-3.0f32).to_le_bytes().to_vec()
        );
        assert_eq!(
            sent(op::REQ_SET_USER_VOLUME),
            vec![(-9.0f32).to_le_bytes().to_vec()]
        );
        for opcode in [
            op::REQ_SET_LOUDNESS_MASK,
            op::REQ_SET_CROSSFEED_OUTPUTS,
            op::REQ_SET_LEVELLER_MASKS,
        ] {
            assert_eq!(sent(opcode), Vec::<Vec<u8>>::new(), "0x{opcode:02X}");
        }
        assert!(
            report
                .left_alone
                .contains(&"loudness.outputMask".to_string())
        );
        let lines = report.lines(false).join(" ");
        assert!(lines.contains("V17"), "the old format is named: {lines}");
    }

    /// A format that does carry what V17 lacks gives it up: the firmware answered
    /// the feature probe, then refused the read, and V31's image has the mask.
    #[test]
    fn wp_tm21_a_later_image_gives_up_what_an_older_one_cannot() {
        let image = image_as(31);
        let want = normalize(&image).unwrap();
        let platform = FakeDevice::demo()
            .control_in_upto(op::REQ_GET_PLATFORM, 0, 7)
            .unwrap();
        let once = Reply::Sequence(vec![Reply::Data(vec![0xFF, 0xFF]), Reply::Stall]);
        let mock = MockTransport::new()
            .data(op::REQ_GET_PLATFORM, platform)
            .data(op::REQ_GET_ALL_PARAMS, image)
            .reply(op::REQ_GET_LOUDNESS_MASK, once);
        let mut c = connected_on(Box::new(Midi(mock))).unwrap();
        let state = DeviceState::new(c.state.caps.clone(), want.packet);
        let mask = i32::from(state.global().loudness_output_mask);
        let doc = preset_file::capture(&mut c.session, None).unwrap();
        assert_eq!(doc.loudness.output_mask, Some(mask));
    }

    /// A filter export refused every read takes each bank whole from the image,
    /// band for band in its place.
    #[test]
    fn wp_tm21_a_filter_export_from_old_firmware_takes_the_image() {
        let image = image_as(17);
        let want = normalize(&image).unwrap();
        let mut c = refusing(image);
        let doc = preset_file::capture_filters(&mut c.session).unwrap();
        assert_eq!(doc.unreported(), Vec::<String>::new());
        for (ch, got) in c.state.caps.channels.iter().zip(&doc.channels) {
            assert_eq!(
                got.peq.len(),
                usize::from(c.state.caps.max_bands),
                "{}",
                ch.name
            );
            for (b, block) in got.peq.iter().enumerate() {
                let band = want.packet.band(ch.index, b as u8).unwrap();
                assert_eq!(block.freq_hz, band.freq, "{} band {b}", ch.name);
                assert_eq!(block.gain, band.gain_db, "{} band {b}", ch.name);
            }
        }
    }

    #[test]
    fn wp_tm21_a_lost_device_fails_a_read_only_filter_export() {
        let platform = FakeDevice::demo()
            .control_in_upto(op::REQ_GET_PLATFORM, 0, 7)
            .unwrap();
        let mock = MockTransport::new()
            .data(op::REQ_GET_PLATFORM, platform)
            .data(op::REQ_GET_ALL_PARAMS, image_as(17))
            .reply(op::REQ_GET_EQ_PARAM, Reply::Disconnect);
        let mut c = connected_on(Box::new(Midi(mock))).unwrap();
        assert!(preset_file::capture_filters(&mut c.session).is_err());
    }

    /// A lost device is not "does not report it", on this route either.
    #[test]
    fn wp_tm21_a_lost_device_fails_a_read_only_export() {
        let platform = FakeDevice::demo()
            .control_in_upto(op::REQ_GET_PLATFORM, 0, 7)
            .unwrap();
        let mock = MockTransport::new()
            .data(op::REQ_GET_PLATFORM, platform)
            .data(op::REQ_GET_ALL_PARAMS, image_as(17))
            .reply(op::REQ_GET_OUTPUT_GAIN, Reply::Disconnect);
        let mut c = connected_on(Box::new(Midi(mock))).unwrap();
        let e = preset_file::capture(&mut c.session, None).unwrap_err();
        assert!(e.what.contains("out.gain"), "{e}");
    }

    /// A device that reports everything leaves nothing to warn about.
    #[test]
    fn wp_tm21_nothing_omitted_nothing_to_warn_about() {
        let mut demo = session_on(Box::new(FakeDevice::demo())).unwrap();
        let doc = preset_file::capture(&mut demo, None).unwrap();
        assert_eq!(export_warning(&doc, "x.dspipreset"), None);
    }

    /// The state a read-only connection hands the interface keeps its image.
    #[test]
    fn wp_tm4_connected_state_matches_the_mapping() {
        for wire in [17, 31, 33] {
            let image = image_as(wire);
            let (mock, _) = device(image.clone());
            let c = connected_on(Box::new(mock)).unwrap();
            let want = normalize(&image).unwrap();
            let state: &DeviceState = &c.state;
            assert_eq!(state.bulk.as_bytes(), want.packet.as_bytes());
            assert_eq!(connect::ext_label(&c.ext), "");
        }
    }
}
