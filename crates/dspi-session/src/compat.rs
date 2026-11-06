//! Reading a device whose firmware speaks another wire format, for display.
//!
//! [`dspi_proto::wire_compat`] maps the image; this reads it. The image of an
//! older firmware is shorter than 6136 bytes (a chunked read past its end
//! stalls), a newer one longer (stopping early leaves the device's snapshot
//! session open), so the length comes from the header
//! ([`dspi_proto::wire_compat::image_length`]). Over a link without chunked
//! bulk ([`dspi_transport::TransportCaps::chunked_bulk`]) the single-shot
//! `0xA0` is used instead.
//!
//! The read is split sans-IO (PLAN §1.3): `ChunkedImage` plans the chunk
//! requests and assembles the replies, `single_shot_image` judges a
//! single-shot reply, and the transport loop in [`read_bulk_bytes`] only
//! drives them.
//!
//! Owned by WP S2 (docs/core-merge/PLAN.md).
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::generated::BULK_SIZE;
use dspi_proto::generated::opcodes as op;
use dspi_proto::packets::FirmwareVersion;
use dspi_proto::wire::WireError;
use dspi_proto::wire_compat::{self, MIN_WIRE_FORMAT_VERSION, Normalized};
use dspi_transport::{Transport, TransportError, with_busy_retry};

/// The largest chunk asked for in one `0xA2` transfer, whatever the
/// transport allows: the size the strict reader ([`crate::read_bulk`]) uses.
const MAX_CHUNK: usize = 1024;

/// Bytes of the header that [`wire_compat::image_length`] needs: the version
/// at 0 and `payload_length` at 6..8.
const HEAD_LEN: usize = 8;

/// Read the bulk image whatever its wire format, and map it onto V32.
///
/// A V32 image comes back as [`dspi_proto::wire_compat::WireRead::Exact`] and
/// may be written back; anything else is display only.
///
/// Fails with [`TransportError::Incompatible`] (inside
/// [`crate::WriteError::Transport`]) for a format that cannot be mapped (older
/// than V16, or a newer image whose V32 prefix does not line up), with
/// [`TransportError::ShortRead`] when the device stops short of the length its
/// header announced, and with the transport's own error otherwise.
pub fn read_bulk_any(t: &mut dyn Transport) -> Result<Normalized, crate::WriteError> {
    Ok(read_any(t, None)?)
}

/// Read the raw image bytes: chunked (`0xA2`, sized by the header) when the
/// transport allows it, else single-shot `0xA0`.
///
/// The bytes are in the device's own format, `payload_length` long (6136 for
/// V32, whose header length is never trusted). Errors as for
/// [`read_bulk_any`], except that no mapping is attempted, so only an image
/// older than V16 is refused (on its first chunk, before the rest is asked
/// for).
pub fn read_bulk_bytes(t: &mut dyn Transport) -> Result<Vec<u8>, crate::WriteError> {
    Ok(read_bytes(t, None)?)
}

/// [`read_bulk_any`] with the transport's error type, knowing which firmware
/// answered GET_PLATFORM so a refusal can name it. Used by
/// [`crate::probe::probe_any`].
pub(crate) fn read_any(
    t: &mut dyn Transport,
    firmware: Option<&FirmwareVersion>,
) -> Result<Normalized, TransportError> {
    let opcode = bulk_opcode(t);
    let image = read_bytes(t, firmware)?;
    wire_compat::normalize(&image).map_err(|e| wire_error(e, opcode, firmware))
}

/// [`read_bulk_bytes`] with the transport's error type.
pub(crate) fn read_bytes(
    t: &mut dyn Transport,
    firmware: Option<&FirmwareVersion>,
) -> Result<Vec<u8>, TransportError> {
    if !t.caps().chunked_bulk {
        // The firmware answers min(wLength, image) (vendor_commands.c,
        // REQ_GET_ALL_PARAMS), so ask for as much as the link carries.
        let max = u16::try_from(t.max_transfer()).unwrap_or(u16::MAX);
        let reply = with_busy_retry(
            || t.control_in_upto(op::REQ_GET_ALL_PARAMS, 0, max),
            op::REQ_GET_ALL_PARAMS,
        )?;
        return single_shot_image(reply);
    }

    let mut image = ChunkedImage::new(t.max_transfer());
    while let Some((offset, len)) = image.next_request() {
        let part = with_busy_retry(
            || t.control_in_upto(op::REQ_GET_ALL_PARAMS_CHUNK, offset, len),
            op::REQ_GET_ALL_PARAMS_CHUNK,
        )?;
        image
            .accept(&part)
            .map_err(|e| e.into_transport(firmware))?;
    }
    Ok(image.into_image())
}

/// The opcode a bulk read on `t` uses, for naming it in an error.
fn bulk_opcode(t: &dyn Transport) -> u8 {
    if t.caps().chunked_bulk {
        op::REQ_GET_ALL_PARAMS_CHUNK
    } else {
        op::REQ_GET_ALL_PARAMS
    }
}

/// Put a mapping failure in the transport's terms.
fn wire_error(e: WireError, opcode: u8, firmware: Option<&FirmwareVersion>) -> TransportError {
    match e {
        WireError::UnsupportedVersion { got, .. } => {
            TransportError::Incompatible(crate::probe::refusal(firmware, got))
        }
        WireError::WrongSize { got, expected } => TransportError::ShortRead {
            opcode,
            wanted: expected,
            got,
        },
        // A mapped image whose header does not decode: the device's data is
        // inconsistent, which the strict reader reports the same way.
        other => TransportError::Usb(other.to_string()),
    }
}

/// Why a chunked read stopped, before it is put in the transport's terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChunkError {
    /// A chunk reply added no bytes: the device has nothing more to give at
    /// `offset`, though the header announced `total`.
    NoProgress { offset: usize, total: usize },
    /// The header names a wire format older than any this build can map.
    TooOld(u8),
}

impl ChunkError {
    fn into_transport(self, firmware: Option<&FirmwareVersion>) -> TransportError {
        match self {
            ChunkError::NoProgress { offset, total } => TransportError::ShortRead {
                opcode: op::REQ_GET_ALL_PARAMS_CHUNK,
                wanted: total,
                got: offset,
            },
            ChunkError::TooOld(v) => {
                TransportError::Incompatible(crate::probe::refusal(firmware, v))
            }
        }
    }
}

/// The plan and the assembly of a chunked bulk read, without I/O.
///
/// The read starts at offset 0 (where the firmware snapshots the image) and
/// proceeds in order. Until the header is in, the length is taken to be
/// V32's; once its first [`HEAD_LEN`] bytes are, the length is the header's
/// ([`wire_compat::image_length`], which never trusts it for V32 nor below the
/// header, and which a `u16` bounds), and anything already read past it is
/// dropped.
#[derive(Debug, Clone)]
pub(crate) struct ChunkedImage {
    buf: Vec<u8>,
    /// The image length: V32's until the header is read, then the header's.
    total: usize,
    /// Whether `total` comes from the header yet.
    sized: bool,
    /// Bytes asked for per transfer, at least 1.
    chunk: usize,
}

impl ChunkedImage {
    /// A read through transfers of at most `max_transfer` bytes (capped at
    /// [`MAX_CHUNK`]; a transport claiming 0 still moves one byte at a time).
    pub(crate) fn new(max_transfer: usize) -> Self {
        Self {
            buf: Vec::with_capacity(BULK_SIZE),
            total: BULK_SIZE,
            sized: false,
            chunk: max_transfer.clamp(1, MAX_CHUNK),
        }
    }

    /// The next `(offset, length)` to ask for, or `None` when the image is
    /// complete.
    pub(crate) fn next_request(&self) -> Option<(u16, u16)> {
        let have = self.buf.len();
        let left = self.total.checked_sub(have).filter(|&n| n > 0)?;
        // Both convert: `total` is BULK_SIZE or a u16 from the header, and
        // `have < total`.
        let offset = u16::try_from(have).ok()?;
        let len = u16::try_from(left.min(self.chunk)).ok()?;
        Some((offset, len))
    }

    /// Take the reply to the last [`ChunkedImage::next_request`]. A reply
    /// longer than asked is cut to the request; an empty one ends the read
    /// ([`ChunkError::NoProgress`]), so a device that stops answering cannot
    /// keep the loop going. A header older than V16 ends it too
    /// ([`ChunkError::TooOld`]), before the rest is asked for.
    pub(crate) fn accept(&mut self, reply: &[u8]) -> Result<(), ChunkError> {
        let Some((offset, len)) = self.next_request() else {
            return Ok(());
        };
        let take = reply.get(..usize::from(len)).unwrap_or(reply);
        if take.is_empty() {
            return Err(ChunkError::NoProgress {
                offset: usize::from(offset),
                total: self.total,
            });
        }
        self.buf.extend_from_slice(take);
        if !self.sized && self.buf.len() >= HEAD_LEN {
            self.sized = true;
            if let Some(&v) = self.buf.first()
                && v < MIN_WIRE_FORMAT_VERSION
            {
                return Err(ChunkError::TooOld(v));
            }
            self.total = wire_compat::image_length(&self.buf);
            self.buf.truncate(self.total);
        }
        Ok(())
    }

    /// The assembled image (complete once `next_request` is `None`).
    pub(crate) fn into_image(self) -> Vec<u8> {
        self.buf
    }
}

/// Judge a single-shot (`0xA0`) reply: cut it to the length its header
/// announces, or fail with [`TransportError::ShortRead`] if it holds less.
pub(crate) fn single_shot_image(mut reply: Vec<u8>) -> Result<Vec<u8>, TransportError> {
    let wanted = wire_compat::image_length(&reply);
    if reply.len() < wanted {
        return Err(TransportError::ShortRead {
            opcode: op::REQ_GET_ALL_PARAMS,
            wanted,
            got: reply.len(),
        });
    }
    reply.truncate(wanted);
    Ok(reply)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_s2 {
    use super::*;
    use dspi_proto::wire_compat::WireRead;
    use dspi_transport::mock::{Direction, Reply};
    use dspi_transport::{
        DeviceDescriptor, MockTransport, NotificationSource, Result as TResult, TransportCaps,
        TransportKind,
    };
    use proptest::prelude::*;

    /// An image of wire format `v`, `len` bytes, its header filled in so that
    /// it maps (17 channels, 9 out, 8 in, 12 band slots).
    fn image(v: u8, len: usize) -> Vec<u8> {
        let mut b = vec![0u8; len];
        b[0] = v;
        b[1] = 1;
        b[2] = 17;
        b[3] = 9;
        b[4] = 8;
        b[5] = 12;
        b[6..8].copy_from_slice(&(len as u16).to_le_bytes());
        b
    }

    fn chunk_reads(t: &MockTransport) -> Vec<u16> {
        t.log()
            .iter()
            .filter(|e| e.direction == Direction::In && e.opcode == op::REQ_GET_ALL_PARAMS_CHUNK)
            .map(|e| e.value)
            .collect()
    }

    /// A transport without chunked bulk (a DSPi Link hub): the mock with its
    /// caps overridden.
    struct Unchunked(MockTransport);

    impl Transport for Unchunked {
        fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> TResult<Vec<u8>> {
            self.0.control_in(opcode, value, len)
        }
        fn control_in_upto(&mut self, opcode: u8, value: u16, max: u16) -> TResult<Vec<u8>> {
            self.0.control_in_upto(opcode, value, max)
        }
        fn control_out(&mut self, opcode: u8, value: u16, data: &[u8]) -> TResult<()> {
            self.0.control_out(opcode, value, data)
        }
        fn descriptor(&self) -> &DeviceDescriptor {
            self.0.descriptor()
        }
        fn max_transfer(&self) -> usize {
            8192
        }
        fn notifications(&self) -> Option<Box<dyn NotificationSource>> {
            None
        }
        fn caps(&self) -> TransportCaps {
            TransportCaps {
                kind: TransportKind::Link,
                chunked_bulk: false,
                remote: true,
            }
        }
    }

    #[test]
    fn an_older_image_is_read_to_its_own_length_and_mapped() {
        let size = wire_compat::bulk_size(28).unwrap();
        let mut img = image(28, size);
        let mv = dspi_proto::generated::OFF_MASTER_VOLUME;
        img[mv..mv + 4].copy_from_slice(&(-12.0f32).to_le_bytes());
        // The window answers nothing past the image, as a stall would: a read
        // that walked V32's length off its end would fail on no progress.
        let mut t = MockTransport::new().window(op::REQ_GET_ALL_PARAMS_CHUNK, img.clone());
        assert_eq!(read_bulk_bytes(&mut t).unwrap(), img);
        assert_eq!(chunk_reads(&t), vec![0, 1024, 2048, 3072, 4096, 5120]);

        let mut t = MockTransport::new().window(op::REQ_GET_ALL_PARAMS_CHUNK, img);
        let n = read_bulk_any(&mut t).unwrap();
        assert_eq!(n.read, WireRead::Older);
        assert_eq!(n.device_version, 28);
        assert!(!n.is_writable());
        let b = n.packet.as_bytes();
        assert_eq!(b.len(), BULK_SIZE);
        assert_eq!(f32::from_le_bytes(b[mv..mv + 4].try_into().unwrap()), -12.0);
    }

    #[test]
    fn wp_s2_a_newer_image_is_read_past_the_v32_length() {
        let mut img = image(33, 6200);
        img[6150] = 0xEE; // in the section appended after V32
        let mut t = MockTransport::new().window(op::REQ_GET_ALL_PARAMS_CHUNK, img.clone());
        let raw = read_bulk_bytes(&mut t).unwrap();
        assert_eq!(raw, img, "the whole snapshot session is drained");
        assert_eq!(chunk_reads(&t).last(), Some(&6144));

        let mut t = MockTransport::new().window(op::REQ_GET_ALL_PARAMS_CHUNK, img.clone());
        let n = read_bulk_any(&mut t).unwrap();
        assert_eq!(n.read, WireRead::Newer);
        assert_eq!(n.device_version, 33);
        assert_eq!(n.packet.as_bytes()[1..6], img[1..6]);
    }

    #[test]
    fn wp_s2_a_v32_image_ignores_its_header_length() {
        let mut img = image(32, BULK_SIZE);
        img[6..8].copy_from_slice(&100u16.to_le_bytes());
        // The window holds more than the image: only V32's length is read.
        let mut long = img.clone();
        long.extend([0xAB; 64]);
        let mut t = MockTransport::new().window(op::REQ_GET_ALL_PARAMS_CHUNK, long);
        let n = read_bulk_any(&mut t).unwrap();
        assert_eq!(n.read, WireRead::Exact);
        assert!(n.is_writable());
        // The mapped header states V32's length, as the packet decodes.
        img[6..8].copy_from_slice(&(BULK_SIZE as u16).to_le_bytes());
        assert_eq!(n.packet.as_bytes(), &img[..]);
    }

    #[test]
    fn a_link_without_chunked_bulk_reads_single_shot() {
        let img = image(28, wire_compat::bulk_size(28).unwrap());
        let mock = MockTransport::new().data(op::REQ_GET_ALL_PARAMS, img);
        let log = mock.log_handle();
        let mut t = Unchunked(mock);
        let n = read_bulk_any(&mut t).unwrap();
        assert_eq!(n.read, WireRead::Older);
        let ops: Vec<u8> = log.lock().unwrap().iter().map(|e| e.opcode).collect();
        assert_eq!(ops, vec![op::REQ_GET_ALL_PARAMS], "one request, no chunks");

        // A V32 image arrives whole; trailing bytes are cut.
        let mut long = image(32, BULK_SIZE);
        long.extend([1, 2, 3]);
        let mut t = Unchunked(MockTransport::new().data(op::REQ_GET_ALL_PARAMS, long));
        assert_eq!(read_bulk_bytes(&mut t).unwrap().len(), BULK_SIZE);
    }

    #[test]
    fn wp_s2_a_short_single_shot_reply_is_a_short_read() {
        let mut t = Unchunked(MockTransport::new().data(op::REQ_GET_ALL_PARAMS, image(32, 4000)));
        let err = read_bulk_any(&mut t).unwrap_err();
        assert!(
            matches!(
                err,
                crate::WriteError::Transport(TransportError::ShortRead {
                    opcode: op::REQ_GET_ALL_PARAMS,
                    wanted: BULK_SIZE,
                    got: 4000
                })
            ),
            "{err:?}"
        );
    }

    #[test]
    fn wp_s2_a_chunk_that_adds_nothing_ends_the_read() {
        let img = image(32, BULK_SIZE);
        let mut t = MockTransport::new().reply(
            op::REQ_GET_ALL_PARAMS_CHUNK,
            Reply::Sequence(vec![
                Reply::Data(img[..1024].to_vec()),
                Reply::Data(Vec::new()),
            ]),
        );
        let err = read_bulk_bytes(&mut t).unwrap_err();
        assert!(
            matches!(
                err,
                crate::WriteError::Transport(TransportError::ShortRead {
                    opcode: op::REQ_GET_ALL_PARAMS_CHUNK,
                    wanted: BULK_SIZE,
                    got: 1024
                })
            ),
            "{err:?}"
        );
        assert_eq!(chunk_reads(&t), vec![0, 1024], "no endless loop");
    }

    #[test]
    fn wp_s2_a_pre_v16_image_is_refused_on_the_first_chunk() {
        let mut t = MockTransport::new().window(op::REQ_GET_ALL_PARAMS_CHUNK, image(15, 5000));
        let err = read_bulk_any(&mut t).unwrap_err();
        assert!(
            matches!(
                &err,
                crate::WriteError::Transport(TransportError::Incompatible(m)) if m.contains("V15")
            ),
            "{err:?}"
        );
        assert_eq!(chunk_reads(&t), vec![0]);
    }

    #[test]
    fn wp_s2_an_implausible_newer_image_is_incompatible() {
        let mut img = image(33, 6200);
        img[5] = 20; // band slots changed: the V32 prefix does not line up
        let mut t = MockTransport::new().window(op::REQ_GET_ALL_PARAMS_CHUNK, img);
        let err = read_bulk_any(&mut t).unwrap_err();
        assert!(
            matches!(
                &err,
                crate::WriteError::Transport(TransportError::Incompatible(m)) if m.contains("V33")
            ),
            "{err:?}"
        );
    }

    // -- the pure parts --

    /// The `(offset, length)` requests a read made.
    type Plan = Vec<(u16, u16)>;

    /// Drive a plan against an image the way the transport loop does, with a
    /// device that answers min(asked, what is left).
    fn drive(max_transfer: usize, img: &[u8]) -> (Plan, Result<Vec<u8>, ChunkError>) {
        let mut c = ChunkedImage::new(max_transfer);
        let mut asked = Vec::new();
        while let Some((off, len)) = c.next_request() {
            asked.push((off, len));
            let start = usize::from(off).min(img.len());
            let end = (start + usize::from(len)).min(img.len());
            if let Err(e) = c.accept(&img[start..end]) {
                return (asked, Err(e));
            }
        }
        (asked, Ok(c.into_image()))
    }

    #[test]
    fn wp_s2_the_plan_follows_the_header_length() {
        let img = image(28, 5944);
        let (asked, out) = drive(4096, &img);
        assert_eq!(
            asked,
            vec![
                (0, 1024),
                (1024, 1024),
                (2048, 1024),
                (3072, 1024),
                (4096, 1024),
                (5120, 824)
            ]
        );
        assert_eq!(out.unwrap(), img);
    }

    #[test]
    fn wp_s2_a_header_split_across_tiny_chunks_is_still_read() {
        let img = image(30, 5980);
        let (asked, out) = drive(3, &img);
        assert_eq!(&asked[..3], &[(0, 3), (3, 3), (6, 3)]);
        assert_eq!(asked.last(), Some(&(5979, 1)));
        assert_eq!(out.unwrap(), img);
        // A transport claiming no transfer size still makes progress.
        let (_, out) = drive(0, &img);
        assert_eq!(out.unwrap(), img);
    }

    #[test]
    fn wp_s2_an_implausible_header_length_is_not_trusted() {
        // V33 announcing 3 bytes: image_length falls back to V32's length.
        let mut img = image(33, BULK_SIZE);
        img[6..8].copy_from_slice(&3u16.to_le_bytes());
        let (_, out) = drive(1024, &img);
        assert_eq!(out.unwrap().len(), BULK_SIZE);
    }

    #[test]
    fn wp_s2_a_reply_longer_than_asked_is_cut() {
        let mut c = ChunkedImage::new(1024);
        c.accept(&image(28, 5944)).unwrap();
        assert_eq!(c.next_request(), Some((1024, 1024)));
    }

    #[test]
    fn wp_s2_an_empty_reply_is_no_progress() {
        let mut c = ChunkedImage::new(1024);
        assert_eq!(
            c.accept(&[]),
            Err(ChunkError::NoProgress {
                offset: 0,
                total: BULK_SIZE
            })
        );
    }

    #[test]
    fn wp_s2_single_shot_judges_the_reply() {
        assert_eq!(single_shot_image(image(28, 5944)).unwrap().len(), 5944);
        let mut long = image(28, 5944);
        long.resize(6136, 0);
        assert_eq!(single_shot_image(long).unwrap().len(), 5944);
        assert!(matches!(
            single_shot_image(vec![32, 0, 0]),
            Err(TransportError::ShortRead {
                wanted: BULK_SIZE,
                got: 3,
                ..
            })
        ));
    }

    proptest! {
        /// Any replies, of any sizes: the assembly never panics, every
        /// request starts where the image so far ends (so offsets only grow),
        /// and the image never outgrows the length it settled on.
        #[test]
        fn proptest_wp_s2_chunked_image_never_panics(
            max_transfer in 0usize..5000,
            replies in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..1100), 0..24),
        ) {
            let mut c = ChunkedImage::new(max_transfer);
            let mut last = None;
            for r in &replies {
                let Some((off, len)) = c.next_request() else { break };
                prop_assert!(len > 0);
                prop_assert!(last.is_none_or(|l| off > l));
                last = Some(off);
                if c.accept(r).is_err() {
                    break;
                }
            }
            let total = c.total;
            prop_assert!(c.into_image().len() <= total);
        }

        #[test]
        fn proptest_wp_s2_single_shot_never_panics(
            reply in prop::collection::vec(any::<u8>(), 0..7000),
        ) {
            if let Ok(img) = single_shot_image(reply.clone()) {
                prop_assert!(img.len() <= reply.len());
            }
        }
    }
}
