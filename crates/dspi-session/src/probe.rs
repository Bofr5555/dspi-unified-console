//! Capability discovery.
//!
//! Run once at connect, before anything else reads or writes. Everything about
//! the device's shape comes from here: channel counts, feature availability,
//! parameter ranges. Nothing downstream is allowed to assume a topology.
//!
//! Where a capability has no descriptor to read, we **probe and cache**: issue
//! the GET once and treat a stall as "this firmware does not have it". That is
//! more durable than comparing firmware versions, because it keeps working when
//! a feature is backported or a platform lacks it.

use dspi_proto::generated::opcodes as op;
use dspi_proto::generated::wire::WIRE_FORMAT_VERSION;
use dspi_proto::packets::{BuildInfo, FirmwareVersion, PlatformInfo};
use dspi_proto::wire::{BulkPacket, WireHeader};
use dspi_proto::{Platform, generated};
use dspi_transport::{InRead, Result, Transport, TransportError, TransportKind, with_busy_retry};

use crate::ext_state::ExtState;

/// Everything discovered about a connected device.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Capabilities {
    pub serial: String,
    pub platform: Platform,
    /// The version as the Console spells it: "1.1.6 beta 4".
    pub firmware: String,
    /// The same version, typed, for comparisons (GET_PLATFORM, config.h:325).
    pub firmware_version: FirmwareVersion,
    /// `git describe` and build date (`REQ_GET_BUILD_INFO`, config.h:326), for
    /// people only. `None` on firmware that stalls the request.
    pub build_info: Option<BuildInfo>,
    pub wire_format: u8,
    pub num_channels: u8,
    pub num_inputs: u8,
    pub num_outputs: u8,
    /// PEQ bands per channel that the firmware will actually accept.
    ///
    /// Not the header's `max_bands`, which is the wire array's depth. See
    /// [`probe_band_count`].
    pub max_bands: u8,
    /// Depth of the wire EQ array, which is what a bulk packet is indexed by.
    /// Larger than `max_bands` whenever the firmware reserves room to grow.
    pub band_storage: u8,
    pub channels: Vec<ChannelInfo>,
    /// Features answered a probe. Absent means this firmware stalled on it.
    pub features: Vec<Feature>,
    pub cs: Option<ControlSurfaceCaps>,
    pub siggen: Option<SiggenCaps>,
    pub active_preset: Option<u8>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ChannelInfo {
    pub index: u8,
    pub name: String,
    pub slug: String,
    pub is_output: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Feature {
    pub name: String,
    pub present: bool,
    /// Why we concluded that, so a surprising answer is debuggable.
    pub evidence: String,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ControlSurfaceCaps {
    pub caps_version: u8,
    pub max_bindings: u8,
    pub type_count: u8,
    pub noun_count: u8,
    /// `CS_MAX_IR_COMMANDS`, 8 before caps v6 and 16 from it.
    pub max_ir_commands: u8,
    /// `CS_MAX_GROUPS`, added at caps v9. Zero on older firmware, which is how
    /// "this device has no groups" is reported.
    pub max_groups: u8,
    /// `CS_MAX_MACROS`, added at caps v9.
    pub max_macros: u8,
    /// `CS_MAX_MACRO_STEPS`, added at caps v9.
    pub max_macro_steps: u8,
    /// `CS_MAX_DISPLAY_PAGES`, from `REQ_GET_CS_DISPLAY_CFG` (caps v10); it
    /// is published nowhere else. Zero before v10.
    pub max_pages: u8,
    /// How many display models the firmware knows (caps v10).
    pub display_models: u8,
    /// The per-type capability rows, in `CsType` order.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub types: Vec<dspi_proto::packets::CsTypeDesc>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SiggenCaps {
    pub version: u8,
    pub type_count: u8,
    pub output_channels: u8,
    pub multitone_max: u8,
    pub valid_channel_mask: u16,
    /// One descriptor per signal type, from `REQ_SIGGEN_GET_CAPS` with the
    /// type index in `wValue` (siggen.h): names, timing models, parameter
    /// ranges and defaults, so a panel follows the device rather than a
    /// transcribed table.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub types: Vec<dspi_proto::packets::SiggenTypeDesc>,
}

/// Read the full bulk snapshot.
///
/// The packet is 6136 bytes, which exceeds WinUSB's 4 KB single-transfer cap, so
/// on USB the chunked opcode is used on every platform rather than only on
/// Windows. Offset 0 snapshots under the firmware's bulk lock, so the read must
/// start there and proceed in order.
///
/// A link without chunked bulk (MIDI, DSPi Link: `TransportCaps::chunked_bulk`)
/// reads the image in one single-shot `0xA0` instead.
pub fn read_bulk(t: &mut dyn Transport) -> Result<BulkPacket> {
    read_bulk_from(t, None)
}

/// [`read_bulk`], knowing which firmware answered GET_PLATFORM so a refusal
/// can name it.
///
/// The header arrives with the first chunk, and its version is checked before
/// the rest is asked for: a device on another wire format has a packet of
/// another size, and walking this build's length off the end of it would fail
/// with a stall or a short read rather than with the reason.
pub(crate) fn read_bulk_from(
    t: &mut dyn Transport,
    firmware: Option<&FirmwareVersion>,
) -> Result<BulkPacket> {
    if !t.caps().chunked_bulk {
        let image = crate::compat::read_bytes(t, firmware)?;
        if let Some(&wire) = image.first()
            && wire != WIRE_FORMAT_VERSION as u8
        {
            return Err(TransportError::Incompatible(refusal(firmware, wire)));
        }
        return BulkPacket::decode(image).map_err(|e| TransportError::Malformed(e.to_string()));
    }

    let total = generated::BULK_SIZE;
    let chunk = t.max_transfer().min(1024);
    let mut buf = Vec::with_capacity(total);

    while buf.len() < total {
        let offset = buf.len();
        let want = chunk.min(total - offset);
        let kind = t.caps().kind;
        let part = with_busy_retry(
            kind,
            || t.control_in(op::REQ_GET_ALL_PARAMS_CHUNK, offset as u16, want as u16),
            op::REQ_GET_ALL_PARAMS_CHUNK,
        )?;
        buf.extend_from_slice(&part[..want.min(part.len())]);
        if offset == 0
            && let Some(&wire) = buf.first()
            && wire != WIRE_FORMAT_VERSION as u8
        {
            return Err(TransportError::Incompatible(refusal(firmware, wire)));
        }
    }

    BulkPacket::decode(buf).map_err(|e| TransportError::Malformed(e.to_string()))
}

/// What to tell someone whose device speaks another wire format.
///
/// The Console's wording for a mismatch is "This device runs firmware X;
/// DSPi Console expects Y." (FirmwareUpdateView.swift:513); this follows it,
/// adds the wire formats, which are what actually differ, and says what to
/// update. The firmware apply accepts exactly its own version and length
/// (bulk_params.h:481-485), so there is no partial compatibility to offer.
pub fn refusal(firmware: Option<&FirmwareVersion>, wire: u8) -> String {
    refusal_for("DSPi Terminal", firmware, wire)
}

/// [`refusal`] naming any application: `app` is how the sentence calls it
/// ("DSPi Terminal", or "this application" where the product is not known,
/// as in the core every app shares).
pub fn refusal_for(app: &str, firmware: Option<&FirmwareVersion>, wire: u8) -> String {
    let expected = FirmwareVersion::expected();
    let ours = WIRE_FORMAT_VERSION;
    let device = firmware.map_or_else(|| "an unknown version".to_string(), |f| f.to_string());
    if (wire as u16) < ours {
        format!(
            "This device runs firmware {device} (wire format V{wire}); {app} expects \
             {expected} (wire format V{ours}). Update the device's firmware to v{expected} to \
             use it with {app}."
        )
    } else {
        format!(
            "This device runs firmware {device} (wire format V{wire}), which is newer than \
             {app} expects ({expected}, wire format V{ours}). Update {app} to use it."
        )
    }
}

/// Probe a device whatever its wire format: what [`probe`] reads, but a
/// firmware of another wire format is described rather than refused, and the
/// bulk image's read mode says which ([`dspi_proto::wire_compat::WireRead`];
/// `Exact` for this build's own format, the only writable one).
///
/// The image is read by [`crate::compat`] (sized by its header, single-shot
/// where the transport has no chunked bulk) and mapped onto V32, so the
/// channel model comes from the device's own header; `wire_format` is the
/// device's version, not the mapped image's. A format that cannot be mapped
/// (older than V16, or a newer one whose V32 prefix does not line up) is still
/// refused, by name, with [`TransportError::Incompatible`].
///
/// Owned by WP S2 (docs/core-merge/PLAN.md).
pub fn probe_any(
    t: &mut dyn Transport,
) -> Result<(Capabilities, dspi_proto::wire_compat::WireRead)> {
    let (caps, image) = probe_any_with_image(t)?;
    Ok((caps, image.read))
}

/// [`probe_any`], also handing back the image it read (mapped onto V32), so a
/// connect does not read the bulk image a second time (about 6 KB: two
/// seconds over MIDI). The read mode is `image.read`.
pub fn probe_any_with_image(
    t: &mut dyn Transport,
) -> Result<(Capabilities, dspi_proto::wire_compat::Normalized)> {
    let (info, serial, _) = identify(t)?;
    let image = crate::compat::read_any(t, Some(&info.version))?;
    let (caps, _) = describe(t, info, serial, &image.packet, image.device_version, false)?;
    Ok((caps, image))
}

// -- WP LK4 --
/// What [`probe_connect`] read.
pub struct Probed {
    pub caps: Capabilities,
    /// The bulk image, mapped onto V32 (as [`probe_any_with_image`]'s).
    pub image: dspi_proto::wire_compat::Normalized,
    /// The `GET_PLATFORM` reply, so the mirror needs no second read.
    pub platform: Vec<u8>,
    /// The extension channel ([`crate::ext_state::ExtState::probe`]), read in
    /// the probe's own batches. An error is a transport failure there; the
    /// channel being absent is the empty state.
    pub ext: Result<ExtState>,
    /// The feature reads were skipped: an earlier probe of the same device
    /// and firmware served ([`Reusable`]).
    pub reused: bool,
    key: ProbeKey,
}

/// What must match for an earlier probe to describe a device: the platform
/// reply (firmware version), the serial, the build, and the image's header
/// and wire format. Anything the user can change (names, presets) is not in
/// the probe's reusable part.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProbeKey {
    platform: Vec<u8>,
    serial: String,
    build: Option<BuildInfo>,
    header: WireHeader,
    device_version: u8,
}

/// A probe kept to describe the same device again without its feature reads
/// (a DSPi Link re-attach after a lost session): see [`probe_connect`].
#[derive(Debug, Clone)]
pub struct Reusable {
    key: ProbeKey,
    caps: Capabilities,
    ext: ExtState,
}

impl Probed {
    /// This probe, to pass to a later [`probe_connect`] of the same device.
    /// `None` when the extension channel could not be read.
    pub fn reusable(&self) -> Option<Reusable> {
        Some(Reusable {
            key: self.key.clone(),
            caps: self.caps.clone(),
            ext: self.ext.as_ref().ok()?.clone(),
        })
    }
}

/// The connect's probe: what [`probe_any_with_image`] reads plus the
/// extension channel, in as few round trips as the transport allows (over
/// DSPi Link up to the hub's `max_inflight` reads at once; elsewhere one at a
/// time, as before). `on_image` runs once the image is in, before the
/// feature reads: the channel count is known from then on (the core starts
/// the Link meter feed there).
///
/// With `reuse`, the build info and active preset are read with the identity;
/// when the platform reply, serial, build and the image's header all match
/// `reuse`'s, its features, caps and extension channel describe the device and
/// nothing more is read (the channel names come from the new image). Anything
/// that differs means a full probe.
pub fn probe_connect(
    t: &mut dyn Transport,
    on_image: &mut dyn FnMut(&dspi_proto::wire_compat::Normalized),
    reuse: Option<&Reusable>,
) -> Result<Probed> {
    // The build and the active preset ride along with the identity
    let extra = if reuse.is_some() {
        vec![
            InRead::exact(op::REQ_GET_BUILD_INFO, 0, BuildInfo::SIZE as u16),
            InRead::exact(op::REQ_PRESET_GET_ACTIVE, 0, 1),
        ]
    } else {
        Vec::new()
    };
    let (info, serial, platform, extra) = identify_with(t, &extra)?;
    let again = reuse.filter(|r| r.key.platform == platform && r.key.serial == serial).and_then(|r| {
        let mut answers = extra.into_iter();
        let build = answers.next().and_then(Result::ok).and_then(|d| BuildInfo::decode(&d).ok());
        let active = answers.next().and_then(Result::ok).and_then(|d| d.first().copied());
        (r.key.build == build).then_some((r, active))
    });
    let image = crate::compat::read_any(t, Some(&info.version))?;
    on_image(&image);
    let header = *image.packet.header();
    if let Some((r, active_preset)) = again
        && r.key.header == header
        && r.key.device_version == image.device_version
    {
        let caps = Capabilities {
            channels: channels_of(&image.packet)?,
            active_preset,
            ..r.caps.clone()
        };
        return Ok(Probed {
            caps,
            image,
            platform,
            ext: Ok(r.ext.clone()),
            reused: true,
            key: r.key.clone(),
        });
    }
    let (caps, ext) = describe(t, info, serial.clone(), &image.packet, image.device_version, true)?;
    let key = ProbeKey {
        platform: platform.clone(),
        serial,
        build: caps.build_info.clone(),
        header,
        device_version: image.device_version,
    };
    Ok(Probed {
        caps,
        image,
        platform,
        ext: ext.unwrap_or_else(|| Ok(ExtState::default())),
        reused: false,
        key,
    })
}

/// Ask the device what it is and what it can do.
///
/// Strict: a device on another wire format is refused with a message that
/// names its firmware ([`refusal`]). [`probe_any`] describes it instead.
pub fn probe(t: &mut dyn Transport) -> Result<Capabilities> {
    let (info, serial, _) = identify(t)?;
    let bulk = read_bulk_from(t, Some(&info.version))?;
    let wire = bulk.header().format_version;
    describe(t, info, serial, &bulk, wire, false).map(|(caps, _)| caps)
}

/// The liveness read and the serial: what GET_PLATFORM says, the serial as
/// the device reports it (empty if it does not), and GET_PLATFORM's reply.
fn identify(t: &mut dyn Transport) -> Result<(PlatformInfo, String, Vec<u8>)> {
    identify_with(t, &[]).map(|(info, serial, platform, _)| (info, serial, platform))
}

/// What [`identify_with`] read: the platform, the serial, GET_PLATFORM's
/// reply and the answers to the extra reads.
type Identity = (PlatformInfo, String, Vec<u8>, Vec<Result<Vec<u8>>>);

/// [`identify`], and the answers to `extra` reads sent along (over DSPi Link
/// in the same round trip; elsewhere after the identity).
fn identify_with(
    t: &mut dyn Transport,
    extra: &[InRead],
) -> Result<Identity> {
    // GET_PLATFORM is the canonical liveness read; do it first so a dead or
    // wrongly-bound device fails here with a clear message rather than midway
    // through a 6136-byte transfer. Ask for 7 bytes and take what comes: older
    // firmware answers 4 or 6, and the decode follows the spec's fallback
    // rule (firmware_versioning_spec.md:109).
    let kind = t.caps().kind;
    let platform_read = InRead::upto(op::REQ_GET_PLATFORM, 0, PlatformInfo::REQUEST_LEN as u16);
    let (plat, serial_raw, extra) = if kind == TransportKind::Link {
        // WP LK4: all in one round trip. The batch retries a busy answer
        // itself, and a dead link fails them all quickly.
        let mut reads = vec![platform_read, InRead::exact(op::REQ_GET_SERIAL, 0, 16)];
        reads.extend_from_slice(extra);
        let mut answers = t.control_in_batch(&reads).into_iter();
        let plat = answers.next().unwrap_or(Err(TransportError::Disconnected))?;
        let serial = answers.next().and_then(Result::ok).unwrap_or_default();
        (plat, serial, answers.collect())
    } else {
        let r = platform_read;
        let plat = with_busy_retry(kind, || t.control_in_upto(r.opcode, r.value, r.len), op::REQ_GET_PLATFORM)?;
        let serial = t.control_in(op::REQ_GET_SERIAL, 0, 16).unwrap_or_default();
        let extra = if extra.is_empty() { Vec::new() } else { t.control_in_batch(extra) };
        (plat, serial, extra)
    };
    let info = PlatformInfo::decode(&plat)
        .map_err(|e| TransportError::Usb(format!("the device could not identify itself: {e}")))?;
    let serial = String::from_utf8_lossy(&serial_raw)
        .trim_end_matches('\0')
        .trim()
        .to_string();
    Ok((info, serial, plat, extra))
}

/// Everything after the bulk read: the channel model from `bulk` (a V32
/// layout, mapped or not), then the per-feature probes. `wire_format` is the
/// device's own wire format. With `ext`, the extension channel is read in the
/// same batches ([`probe_connect`]).
fn describe(
    t: &mut dyn Transport,
    info: PlatformInfo,
    serial: String,
    bulk: &BulkPacket,
    wire_format: u8,
    ext: bool,
) -> Result<(Capabilities, Option<Result<ExtState>>)> {
    let platform = info.platform();
    let firmware_version = info.version;
    let h: &WireHeader = bulk.header();

    let channels = channels_of(bulk)?;

    // WP LK2: every read that needs no earlier answer goes out in one batch,
    // pipelined over DSPi Link (one at a time elsewhere): the feature probes,
    // the control-surface and generator caps headers, the build info, the
    // active preset and the first band; then what depends on them.
    // WP LK4: also the band search's first round, the whole control-surface
    // caps header in one read, and the extension list; then one more batch
    // for everything that depends on those answers.
    let mut reads = feature_reads();
    let first = reads.len();
    reads.extend([
        InRead::upto(op::REQ_GET_CS_CAPS, 0xFFFF, CS_CAPS_MAX),
        InRead::exact(op::REQ_SIGGEN_GET_CAPS, 0xFFFF, 8),
        InRead::exact(op::REQ_GET_BUILD_INFO, 0, BuildInfo::SIZE as u16),
        InRead::exact(op::REQ_PRESET_GET_ACTIVE, 0, 1),
    ]);
    let mut bands = BandSearch::new(h.max_bands);
    let round1 = if h.max_bands != 0 {
        reads.push(band_read(0));
        let points = bands.points();
        reads.extend(points.iter().map(|b| band_read(*b)));
        points
    } else {
        Vec::new()
    };
    if ext {
        reads.push(ExtState::list_read());
    }
    let mut answers = t.control_in_batch(&reads).into_iter();
    let features = features_from(answers.by_ref().take(first));
    let mut next = || answers.next().unwrap_or(Err(TransportError::Disconnected));
    let (cs_header, siggen_header) = (next(), next());
    // Informational, and absent before beta4: a stall is the answer, so no
    // retry (config.h:326).
    let build_info = next().ok().and_then(|d| BuildInfo::decode(&d).ok());
    let active_preset = next().ok().and_then(|d| d.first().copied());
    let band0 = h.max_bands != 0 && next().is_ok();
    let round1: Vec<(u8, bool)> = round1.into_iter().map(|b| (b, next().is_ok())).collect();
    let ext_list = ext.then(&mut next);
    if band0 {
        bands.answer(&round1);
    }

    // The second batch: what the answers above decide.
    let cs = cs_caps_from(cs_header);
    let display = cs
        .as_ref()
        .is_some_and(|c| c.caps_version >= 10)
        .then(|| InRead::exact(op::REQ_GET_CS_DISPLAY_CFG, 0, 16));
    let siggen = siggen_header_from(siggen_header);
    let descriptors = siggen.as_ref().map_or(0, |s| s.type_count);
    let round2 = if band0 { bands.points() } else { Vec::new() };
    let ext_reads = ext_list.as_ref().map(ExtState::follow_up).unwrap_or_default();
    let mut reads: Vec<InRead> = (0..u16::from(descriptors))
        .map(|i| InRead::exact(op::REQ_SIGGEN_GET_CAPS, i, dspi_proto::packets::SiggenTypeDesc::SIZE as u16))
        .collect();
    reads.extend(display);
    reads.extend(round2.iter().map(|b| band_read(*b)));
    reads.extend(ext_reads.iter().copied());
    let mut answers = if reads.is_empty() { Vec::new() } else { t.control_in_batch(&reads) }.into_iter();
    let mut next = || answers.next().unwrap_or(Err(TransportError::Disconnected));
    let types = (0..descriptors)
        .filter_map(|_| dspi_proto::packets::SiggenTypeDesc::decode(&next().ok()?).ok())
        .collect();
    let siggen = siggen.map(|s| SiggenCaps { types, ..s });
    let cs = cs.map(|mut c| {
        if display.is_some() {
            // The display page limit lives in the display config reply, not
            // the caps header (control_surfaces.h, REQ_GET_CS_DISPLAY_CFG: 16
            // bytes with `max_pages` and `model_count` ahead of the 12-byte
            // config).
            (c.max_pages, c.display_models) = next()
                .ok()
                .and_then(|d| dspi_proto::packets::CsDisplayCfgReply::decode(&d).ok())
                .map(|r| (r.max_pages, r.model_count))
                .unwrap_or((0, 0));
        }
        c
    });
    let round2: Vec<(u8, bool)> = round2.into_iter().map(|b| (b, next().is_ok())).collect();
    bands.answer(&round2);
    let ext = ext_list.map(|list| ExtState::from_answers(list, ext_reads.iter().map(|_| next()).collect()));
    let max_bands = if band0 { bands.finish(t) } else { h.max_bands };

    Ok((
        Capabilities {
            serial: if serial.is_empty() {
                t.descriptor().serial.clone()
            } else {
                serial
            },
            platform,
            firmware: firmware_version.to_string(),
            firmware_version,
            build_info,
            wire_format,
            num_channels: h.num_channels,
            num_inputs: h.num_input_channels,
            num_outputs: h.num_output_channels,
            max_bands,
            band_storage: h.max_bands,
            channels,
            features,
            cs,
            siggen,
            active_preset,
        },
        ext,
    ))
}

/// The channel model from the image, names included.
fn channels_of(bulk: &BulkPacket) -> Result<Vec<ChannelInfo>> {
    let h = bulk.header();
    let map = bulk
        .channel_map()
        .ok_or_else(|| TransportError::Usb("device reported an impossible channel map".into()))?;
    let map = map.with_names(channel_names(bulk, h.num_channels));
    Ok((0..h.num_channels)
        .map(|i| ChannelInfo {
            index: i,
            name: map.label(i),
            slug: map.slug(i),
            is_output: map.is_output(i),
        })
        .collect())
}

/// `wLength` of the control-surface caps header read: room for the header
/// with a type table of 26 rows (52 bytes at caps v18); the firmware answers
/// what it has (`vendor_send_response`, cut to `wLength`).
const CS_CAPS_MAX: u16 = 128;

/// Bands asked at once in each round of the band search.
const BAND_FANOUT: u16 = 3;

/// WP LK4: the search for the first dead band ([`probe_band_count`]), a few
/// bands per round instead of one, so that over a pipelined link it takes two
/// rounds (the first inside the probe's main batch) instead of four serial
/// reads. Invariant: `lo` is live, `hi` is not.
struct BandSearch {
    lo: u8,
    hi: u8,
}

impl BandSearch {
    fn new(storage: u8) -> Self {
        Self { lo: 0, hi: storage }
    }

    /// The bands to ask next: every one left when that is few enough, else
    /// [`BAND_FANOUT`] that split the interval evenly.
    fn points(&self) -> Vec<u8> {
        let (lo, hi) = (u16::from(self.lo), u16::from(self.hi));
        if hi <= lo + 1 {
            return Vec::new();
        }
        let gap = hi - lo;
        let mut points: Vec<u16> = if gap - 1 <= BAND_FANOUT {
            (lo + 1..hi).collect()
        } else {
            (1..=BAND_FANOUT).map(|k| lo + gap * k / (BAND_FANOUT + 1)).collect()
        };
        points.retain(|p| *p > lo && *p < hi);
        points.dedup();
        points.into_iter().filter_map(|p| u8::try_from(p).ok()).collect()
    }

    /// Narrow by `answers` (band, answered): the highest live band and the
    /// lowest dead one above it. An answer that contradicts the invariant
    /// (a dead band below a live one) is ignored.
    fn answer(&mut self, answers: &[(u8, bool)]) {
        for &(band, live) in answers {
            if live && band > self.lo && band < self.hi {
                self.lo = band;
            }
        }
        for &(band, live) in answers {
            if !live && band > self.lo && band < self.hi {
                self.hi = band;
            }
        }
    }

    /// The count: what is left of the interval is searched one read at a
    /// time (only for band arrays far deeper than today's).
    fn finish(mut self, t: &mut dyn Transport) -> u8 {
        while self.hi - self.lo > 1 {
            let mid = self.lo + (self.hi - self.lo) / 2;
            let r = band_read(mid);
            let live = t.control_in(r.opcode, r.value, r.len).is_ok();
            self.answer(&[(mid, live)]);
        }
        self.hi
    }
}

/// How many PEQ bands the firmware will actually accept per channel.
///
/// The header's `max_bands` is the depth of the wire array, not the number of
/// live bands: `config.h` reserves 12 slots and says "only bands 0..9 are
/// active today", with the vendor handlers rejecting everything between the
/// live count and the crossover base. Trusting the header therefore offers the
/// user two bands that silently do nothing.
///
/// No opcode reports the count, so it is measured: `GET_EQ_PARAM` answers for a
/// live band and stalls for a reserved one, and that boundary is monotonic, so
/// a search finds it in a few transfers (WP LK4: three bands per round, two
/// rounds for today's 12-deep array, the first inside the probe's main batch). Measuring rather than hardcoding
/// means a firmware that grows to 20 bands is picked up without a code change.
///
/// Deliberately not wrapped in `with_busy_retry`: here a stall is the answer,
/// and retrying it would cost 600 ms per probe.
#[cfg(test)]
fn probe_band_count(t: &mut dyn Transport, storage: u8) -> u8 {
    let live = |t: &mut dyn Transport, band: u8| {
        let r = band_read(band);
        t.control_in(r.opcode, r.value, r.len).is_ok()
    };
    // If even band 0 will not answer, the device is not in a state to be
    // measured. Report the header's depth rather than claiming it has no EQ.
    if storage == 0 || !live(t, 0) {
        return storage;
    }
    // The probe's two rounds, one read at a time here
    let mut search = BandSearch::new(storage);
    for _ in 0..2 {
        let answers: Vec<(u8, bool)> = search.points().into_iter().map(|b| (b, live(t, b))).collect();
        search.answer(&answers);
    }
    search.finish(t)
}

/// The read of `band`'s filter type on channel 0: any live band answers,
/// reserved ones stall.
fn band_read(band: u8) -> InRead {
    InRead::exact(op::REQ_GET_EQ_PARAM, (band as u16) << 3, 4)
}

/// Channel names live in the bulk packet, so they cost nothing extra to read.
fn channel_names(bulk: &BulkPacket, count: u8) -> Vec<String> {
    let Some(section) = bulk.section("channel_names") else {
        return Vec::new();
    };
    (0..count as usize)
        .map(|i| {
            let start = i * 32;
            section
                .get(start..start + 32)
                .map(|b| {
                    String::from_utf8_lossy(b)
                        .trim_end_matches('\0')
                        .trim()
                        .to_string()
                })
                .unwrap_or_default()
        })
        .collect()
}

/// (name, opcode, wValue, length) of each feature probe.
type FeatureProbe = (&'static str, u8, u16, u16);

/// The feature probes' reads, in [`feature_probes`]' order.
fn feature_reads() -> Vec<InRead> {
    feature_probes()
        .iter()
        .map(|(_, opcode, value, len)| InRead::exact(*opcode, *value, *len))
        .collect()
}

/// Features with no capability descriptor: probe once, cache the answer.
///
/// A stall here is information, not a failure. The loudness output mask, for
/// instance, only exists from wire V19, and older firmware stalls on it.
fn feature_probes() -> &'static [FeatureProbe] {
    use dspi_proto::generated::{limiter, tube};
    &[
        // The Console's `inputSourceSupported`, which gates its whole input
        // page: firmware without a selectable source stalls 0xE1.
        ("input_source", op::REQ_GET_INPUT_SOURCE, 0, 1),
        ("loudness_output_mask", op::REQ_GET_LOUDNESS_MASK, 0, 2),
        ("crossfeed_output_mask", op::REQ_GET_CROSSFEED_OUTPUTS, 0, 1),
        ("leveller_masks", op::REQ_GET_LEVELLER_MASKS, 0, 2),
        ("psychoacoustic_bass", op::REQ_GET_PSYBASS, 0, 1),
        ("upmixer", op::REQ_UPMIX_GET_STATUS, 0, 16),
        ("adat_output", op::REQ_GET_ADAT_ENABLE, 0, 1),
        ("adat_input", op::REQ_GET_ADAT_INPUT_ENABLE, 0, 1),
        ("i2s_slave_clock", op::REQ_GET_I2S_CLOCK_MODE, 0, 1),
        ("i2s_input_channels", op::REQ_GET_I2S_INPUT_CHANNELS, 0, 1),
        // 6 bytes at v1.1.6: {count, enable_mask, gpio[0..3]} (config.h:456-457).
        ("spdif_multi_input", op::REQ_GET_SPDIF_INPUT_CONFIG, 0, 6),
        ("lg_sound_sync", op::REQ_GET_LG_SOUND_SYNC_ENABLE, 0, 1),
        ("dac_hardware_mute", op::REQ_GET_DAC_HW_MUTE_CONFIG, 0, 16),
        ("uart_control", op::REQ_GET_UART_CONFIG, 0, 8),
        ("i2c_control", op::REQ_GET_I2C_CONFIG, 0, 8),
        ("test_signals", op::REQ_SIGGEN_GET_STATUS, 0, 16),
        // Beta4's tools have no capability bit, so each is probed. The
        // subharmonic synthesizer from wire V29 (config.h:184-185).
        ("subharmonic_synth", op::REQ_GET_SUBHARM, 0, 1),
        // The tube preamp stalls an unknown index, so index 0 answers exactly
        // when the module exists (vendor_commands.c:2246-2253).
        (
            "tube_preamp",
            op::REQ_GET_TUBE_PARAM,
            tube::TUBE_PARAM_ENABLED,
            4,
        ),
        // The limiter's status block is its feature probe (limiter.h:20,
        // vendor_commands.c:2238).
        (
            "output_limiter",
            op::REQ_LIMITER,
            limiter::LIMITER_GET_STATUS,
            4,
        ),
        // The spectrum analyser's caps header (config.h:149, rta.h:72-87).
        ("spectrum_analyser", op::REQ_RTA_GET_CAPS, 0, 16),
        // v1.1.6 final reassigned 0x54-0x57 from the legacy channel gain and
        // mute. Older firmware answers 0x55 with a 4-byte gain and 0x57 with
        // a 1-byte mute, so only a full 32-byte name counts; a short read is
        // "absent" (device_name_spec.md section 2).
        ("input_source_names", op::REQ_GET_INPUT_SOURCE_NAME, 0, 32),
        ("device_name", op::REQ_GET_DEVICE_NAME, 0, 32),
        // OutputLatencyStatus, 12 bytes (config.h REQ_OUTPUT_LATENCY).
        ("output_latency", op::REQ_OUTPUT_LATENCY, 0, 12),
    ]
}

/// The features from the answers to [`feature_reads`], in order.
fn features_from(answers: impl Iterator<Item = Result<Vec<u8>>>) -> Vec<Feature> {
    feature_probes()
        .iter()
        .zip(answers)
        .map(|((name, opcode, _, _), answer)| match answer {
            Ok(_) => Feature {
                name: (*name).into(),
                present: true,
                evidence: format!("0x{opcode:02X} answered"),
            },
            Err(TransportError::Stalled { .. }) => Feature {
                name: (*name).into(),
                present: false,
                evidence: format!("0x{opcode:02X} stalled"),
            },
            Err(e) => Feature {
                name: (*name).into(),
                present: false,
                evidence: format!("0x{opcode:02X}: {e}"),
            },
        })
        .collect()
}

#[cfg(test)]
fn probe_cs_caps(t: &mut dyn Transport) -> Option<ControlSurfaceCaps> {
    // wValue 0xFFFF asks for the header rather than a single noun descriptor.
    let header = t.control_in(op::REQ_GET_CS_CAPS, 0xFFFF, 4);
    cs_caps_after(t, header)
}

/// The control-surface caps from the header read (wValue 0xFFFF, up to
/// [`CS_CAPS_MAX`] bytes); the display page limit is not filled in here.
fn cs_caps_from(header: Result<Vec<u8>>) -> Option<ControlSurfaceCaps> {
    let d = header.ok()?;
    let caps_version = *d.first()?;
    let max_bindings = *d.get(1)?;
    let type_count = *d.get(2)?;
    let noun_count = *d.get(3)?;

    // The v3+ additions sit after the variable-length type table, so the header
    // length depends on type_count: `max_ir_commands`, then the three v9
    // maxima, at `4 + 4*type_count` (control_surfaces.h:644-651). The table grew
    // by a row at caps v10 when CS_TYPE_DISPLAY arrived, taking the header from
    // 40 bytes to 44, and by two at v18 for the auxiliary outputs, taking it to
    // 52, so a fixed offset here would read the wrong four bytes. A reply
    // shorter than that has no table to trust (as a short exact read before).
    let post_table = 4 + 4 * type_count as usize;
    let full_len = post_table + 4;
    let full = d.get(..full_len).unwrap_or_default();
    let at = |i: usize| full.get(post_table + i).copied().unwrap_or(0);
    let types = dspi_proto::packets::CsCapsHeader::decode(full)
        .map(|h| h.types)
        .unwrap_or_default();

    Some(ControlSurfaceCaps {
        caps_version,
        max_bindings,
        type_count,
        noun_count,
        max_ir_commands: at(0),
        max_groups: at(1),
        max_macros: at(2),
        max_macro_steps: at(3),
        max_pages: 0,
        display_models: 0,
        types,
    })
}

#[cfg(test)]
fn cs_caps_after(t: &mut dyn Transport, header: Result<Vec<u8>>) -> Option<ControlSurfaceCaps> {
    let header = header.and_then(|_| t.control_in_upto(op::REQ_GET_CS_CAPS, 0xFFFF, CS_CAPS_MAX));
    let mut c = cs_caps_from(header)?;
    if c.caps_version >= 10 {
        (c.max_pages, c.display_models) = t
            .control_in(op::REQ_GET_CS_DISPLAY_CFG, 0, 16)
            .ok()
            .and_then(|d| dspi_proto::packets::CsDisplayCfgReply::decode(&d).ok())
            .map(|r| (r.max_pages, r.model_count))
            .unwrap_or((0, 0));
    }
    Some(c)
}

/// The generator's caps from the 8-byte header's answer, without the
/// per-type descriptors (each its own transfer, indexed by wValue; read in
/// the probe's second batch, where one that fails to read is skipped rather
/// than failing the probe, so a device with an odd table still gets a
/// generator panel).
fn siggen_header_from(header: Result<Vec<u8>>) -> Option<SiggenCaps> {
    let d = header.ok()?;
    Some(SiggenCaps {
        version: *d.first()?,
        type_count: *d.get(1)?,
        output_channels: *d.get(2)?,
        multitone_max: *d.get(3)?,
        valid_channel_mask: u16::from_le_bytes([*d.get(4)?, *d.get(5)?]),
        types: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dspi_proto::wire_compat::WireRead;
    use dspi_transport::MockTransport;
    use dspi_transport::mock::Reply;

    /// A device with a given number of live PEQ bands.
    ///
    /// The shared mock keys its replies on the opcode alone, but band validity
    /// is carried in `wValue`, so it cannot express "answers for band 9, stalls
    /// for band 10", which is the whole of what the probe reads.
    struct BandLimited {
        live: u8,
        descriptor: dspi_transport::DeviceDescriptor,
        /// Every band index asked about, in order.
        asked: Vec<u8>,
    }

    impl BandLimited {
        fn new(live: u8) -> Self {
            Self {
                live,
                descriptor: dspi_transport::DeviceDescriptor {
                    serial: "TEST".into(),
                    bus_id: "001".into(),
                    address: 1,
                },
                asked: Vec::new(),
            }
        }
    }

    impl Transport for BandLimited {
        fn control_in(&mut self, opcode: u8, value: u16, len: u16) -> Result<Vec<u8>> {
            if opcode == op::REQ_GET_EQ_PARAM {
                // wValue: channel in the high byte, band in bits 3..7.
                let band = ((value >> 3) & 0x1F) as u8;
                self.asked.push(band);
                if band >= self.live {
                    return Err(TransportError::Stalled { opcode });
                }
            }
            Ok(vec![0u8; len as usize])
        }

        fn control_out(&mut self, _opcode: u8, _value: u16, _data: &[u8]) -> Result<()> {
            Ok(())
        }

        fn descriptor(&self) -> &dspi_transport::DeviceDescriptor {
            &self.descriptor
        }
    }

    /// The header's `max_bands` is the wire array's depth, not the live count.
    /// Believing it offers the user bands the firmware silently rejects.
    #[test]
    fn the_live_band_count_is_measured_not_taken_from_the_header() {
        for live in 1..=12u8 {
            let mut t = BandLimited::new(live);
            assert_eq!(probe_band_count(&mut t, 12), live, "with {live} live");
        }
    }

    /// Six transfers in two rounds, not twelve: the probe runs on every
    /// connect, and a linear walk would put the cost back into startup. (WP
    /// LK4: three per round instead of a binary search's one, so a pipelined
    /// link pays two round trips, the first shared with the feature probes.)
    #[test]
    fn the_probe_is_a_binary_search() {
        let mut t = BandLimited::new(10);
        probe_band_count(&mut t, 12);
        assert!(
            t.asked.len() <= 6,
            "took {} transfers: {:?}",
            t.asked.len(),
            t.asked
        );
    }

    /// A device that will not answer at all is not evidence that it has no EQ,
    /// so the header's depth is the safer report.
    #[test]
    fn an_unresponsive_device_falls_back_to_the_header() {
        let mut t = BandLimited::new(0);
        assert_eq!(probe_band_count(&mut t, 12), 12);
    }

    fn bulk_bytes() -> Vec<u8> {
        let mut b = vec![0u8; generated::BULK_SIZE];
        b[0] = generated::wire::WIRE_FORMAT_VERSION as u8;
        b[1] = 1; // RP2350
        b[2] = 17; // channels
        b[3] = 9; // outputs
        b[4] = 8; // inputs
        b[5] = 12; // max bands
        b[6..8].copy_from_slice(&(generated::BULK_SIZE as u16).to_le_bytes());
        // Name channel 0 so the fallback-vs-device-name path is covered.
        let off = generated::OFF_CHANNEL_NAMES;
        b[off..off + 9].copy_from_slice(b"Turntable");
        b
    }

    /// Beta4's seven-byte GET_PLATFORM: RP2350, 1.1.6, nine outputs, beta 4
    /// (vendor_commands.c:2638-2655).
    const BETA4_PLATFORM: [u8; 7] = [1, 1, 0x16, 9, 1, 6, 4];

    fn device() -> MockTransport {
        MockTransport::new()
            .data(op::REQ_GET_PLATFORM, BETA4_PLATFORM.to_vec())
            .data(op::REQ_GET_SERIAL, b"4BA1DDB9D1443D6A".to_vec())
            .window(op::REQ_GET_ALL_PARAMS_CHUNK, bulk_bytes())
            .data(op::REQ_PRESET_GET_ACTIVE, vec![3])
            // Caps v13: nine component types, so the post-table fields sit at
            // 4 + 4*9 = 40 and the header is 44 bytes.
            .data(op::REQ_GET_CS_CAPS, {
                let mut v = vec![13, 16, 9, 57];
                v.extend(std::iter::repeat_n(0u8, 4 * 9));
                v.extend([16, 8, 8, 8]); // ir commands, groups, macros, steps
                v
            })
            .data(
                op::REQ_SIGGEN_GET_CAPS,
                vec![1, 15, 9, 16, 0xFF, 0x01, 0, 0],
            )
    }

    #[test]
    fn discovers_the_channel_model_without_assuming_it() {
        let mut t = device();
        let caps = probe(&mut t).unwrap();
        assert_eq!(caps.platform, Platform::Rp2350);
        assert_eq!(caps.num_channels, 17);
        assert_eq!(caps.num_inputs, 8);
        assert_eq!(caps.num_outputs, 9);
        assert_eq!(caps.firmware, "1.1.6 beta 4");
        assert_eq!(caps.firmware_version, FirmwareVersion::new(1, 1, 6, 4));
        assert_eq!(caps.wire_format, 32);
        assert_eq!(caps.active_preset, Some(3));
    }

    /// GET_PLATFORM is asked for 7 bytes and older firmware answers fewer;
    /// the short answer is the version, not a failed read.
    #[test]
    fn a_short_platform_reply_still_identifies_the_device() {
        for (reply, want) in [
            (vec![1, 1, 0x16, 9], "1.1.6 early beta"),
            (vec![1, 1, 0x16, 9, 1, 6], "1.1.6 early beta"),
            (BETA4_PLATFORM.to_vec(), "1.1.6 beta 4"),
            (vec![1, 1, 0x16, 9, 1, 6, 0], "1.1.6"),
        ] {
            let mut t = device().data(op::REQ_GET_PLATFORM, reply.clone());
            let caps = probe(&mut t).unwrap();
            assert_eq!(caps.firmware, want, "from {reply:?}");
        }
        let mut t = MockTransport::new().data(op::REQ_GET_PLATFORM, vec![1, 1]);
        assert!(probe(&mut t).is_err(), "two bytes identify nothing");
    }

    /// Build info is for people and absent before beta4; a stall leaves it
    /// `None` rather than failing the probe (config.h:326).
    #[test]
    fn build_info_is_read_when_the_firmware_has_it() {
        let mut t = device();
        assert_eq!(probe(&mut t).unwrap().build_info, None, "stalled");

        let mut blob = vec![0u8; 64];
        blob[..23].copy_from_slice(b"v1.1.6-beta4-0-g557bce7");
        blob[48..58].copy_from_slice(b"2026-09-28");
        let mut t = device().data(op::REQ_GET_BUILD_INFO, blob);
        let info = probe(&mut t).unwrap().build_info.unwrap();
        assert_eq!(info.describe, "v1.1.6-beta4-0-g557bce7");
        assert_eq!(info.date, "2026-09-28");
    }

    /// A link without chunked bulk, over a scripted device.
    struct NoChunks {
        inner: MockTransport,
        kind: dspi_transport::TransportKind,
    }

    impl Transport for NoChunks {
        fn control_in(
            &mut self,
            opcode: u8,
            value: u16,
            len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            self.inner.control_in(opcode, value, len)
        }

        fn control_in_upto(
            &mut self,
            opcode: u8,
            value: u16,
            max_len: u16,
        ) -> dspi_transport::Result<Vec<u8>> {
            self.inner.control_in_upto(opcode, value, max_len)
        }

        fn control_out(
            &mut self,
            opcode: u8,
            value: u16,
            data: &[u8],
        ) -> dspi_transport::Result<()> {
            self.inner.control_out(opcode, value, data)
        }

        fn descriptor(&self) -> &dspi_transport::DeviceDescriptor {
            self.inner.descriptor()
        }

        fn max_transfer(&self) -> usize {
            8192
        }

        fn caps(&self) -> dspi_transport::TransportCaps {
            dspi_transport::TransportCaps {
                kind: self.kind,
                chunked_bulk: false,
                remote: self.kind == dspi_transport::TransportKind::Link,
            }
        }
    }

    const NO_CHUNK_LINKS: [dspi_transport::TransportKind; 2] = [
        dspi_transport::TransportKind::Link,
        dspi_transport::TransportKind::Midi,
    ];

    fn chunk_reads(log: &dspi_transport::mock::LogHandle) -> usize {
        log.lock()
            .unwrap()
            .iter()
            .filter(|e| e.opcode == op::REQ_GET_ALL_PARAMS_CHUNK)
            .count()
    }

    /// Link spec 8.2: a hub refuses the chunked opcodes, and MIDI carries the
    /// image as one SysEx. `device()` also answers chunks, so a read that used
    /// them would pass but for the count.
    #[test]
    fn wp_r1_a_link_without_chunked_bulk_reads_single_shot() {
        for kind in NO_CHUNK_LINKS {
            let inner = device().data(op::REQ_GET_ALL_PARAMS, bulk_bytes());
            let log = inner.log_handle();
            let mut t = NoChunks { inner, kind };
            let bulk = read_bulk(&mut t).unwrap();
            assert_eq!(bulk.header().format_version, WIRE_FORMAT_VERSION as u8);
            let caps = probe(&mut t).unwrap();
            assert_eq!(caps.wire_format, WIRE_FORMAT_VERSION as u8);
            assert_eq!(chunk_reads(&log), 0, "{kind:?}");
        }
    }

    #[test]
    fn wp_r1_a_session_snapshot_without_chunked_bulk_reads_single_shot() {
        for kind in NO_CHUNK_LINKS {
            let inner = device().data(op::REQ_GET_ALL_PARAMS, bulk_bytes());
            let log = inner.log_handle();
            let mut t = NoChunks { inner, kind };
            let caps = probe(&mut t).unwrap();
            let mut session = crate::Session::new(Box::new(t), caps).unwrap();
            session.snapshot().unwrap();
            assert_eq!(chunk_reads(&log), 0, "{kind:?}");
        }
    }

    /// Another wire format is refused by name on this path too, and a reply
    /// shorter than its header says is an error, not a panic.
    #[test]
    fn wp_r1_a_single_shot_read_refuses_what_it_cannot_use() {
        let mut old = vec![0u8; 5944];
        old[0] = 28;
        old[6..8].copy_from_slice(&5944u16.to_le_bytes());
        let inner = MockTransport::new()
            .data(op::REQ_GET_PLATFORM, vec![1, 1, 0x16, 9])
            .data(op::REQ_GET_SERIAL, b"4BA1DDB9D1443D6A".to_vec())
            .data(op::REQ_GET_ALL_PARAMS, old);
        let kind = dspi_transport::TransportKind::Link;
        let err = probe(&mut NoChunks { inner, kind }).unwrap_err();
        let TransportError::Incompatible(msg) = &err else {
            panic!("expected a refusal, got {err:?}");
        };
        assert!(msg.contains("wire format V28"), "{msg}");

        let mut short = bulk_bytes();
        short.truncate(100);
        let inner = MockTransport::new().data(op::REQ_GET_ALL_PARAMS, short);
        let err = read_bulk(&mut NoChunks { inner, kind }).unwrap_err();
        assert!(matches!(err, TransportError::ShortRead { .. }), "{err:?}");
    }

    /// A packet in an older wire format, as a beta2 or beta3 device sends.
    fn old_device(platform: Vec<u8>, wire: u8, size: usize) -> MockTransport {
        let mut b = vec![0u8; size];
        b[0] = wire;
        b[1] = 1;
        b[2] = 17;
        b[3] = 9;
        b[4] = 8;
        b[5] = 12;
        b[6..8].copy_from_slice(&(size as u16).to_le_bytes());
        MockTransport::new()
            .data(op::REQ_GET_PLATFORM, platform)
            .data(op::REQ_GET_SERIAL, b"4BA1DDB9D1443D6A".to_vec())
            .window(op::REQ_GET_ALL_PARAMS_CHUNK, b)
    }

    /// A beta2 device (wire V28, a 4-byte platform reply) is refused with a
    /// message that names its firmware and says what to update it to, and
    /// the refusal comes from the header in the first chunk, before any of
    /// the V32 length is asked for.
    #[test]
    fn a_v28_device_is_refused_by_name() {
        let mut t = old_device(vec![1, 1, 0x16, 9], 28, 5944);
        let log = t.log_handle();
        let err = probe(&mut t).unwrap_err();
        let TransportError::Incompatible(msg) = &err else {
            panic!("expected a refusal, got {err:?}");
        };
        assert_eq!(
            msg,
            "This device runs firmware 1.1.6 early beta (wire format V28); DSPi Terminal \
             expects 1.1.6 (wire format V32). Update the device's firmware to \
             v1.1.6 to use it with DSPi Terminal."
        );
        assert_eq!(err.to_string(), *msg, "shown as is");
        let chunks = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.opcode == op::REQ_GET_ALL_PARAMS_CHUNK)
            .count();
        assert_eq!(
            chunks, 1,
            "refused on the header, not after walking off the packet"
        );
    }

    /// Beta3 reports its ordinal (wire V30); the message names it exactly.
    #[test]
    fn a_beta3_device_is_refused_by_its_beta_number() {
        let mut t = old_device(vec![1, 1, 0x16, 9, 1, 6, 3], 30, 5980);
        let msg = probe(&mut t).unwrap_err().to_string();
        assert!(msg.starts_with("This device runs firmware 1.1.6 beta 3 (wire format V30);"));
        assert!(
            msg.contains("Update the device's firmware to v1.1.6 to"),
            "{msg}"
        );
    }

    /// A device ahead of this build is refused too, and the thing to update
    /// is the Terminal.
    #[test]
    fn a_newer_device_asks_for_a_newer_terminal() {
        let mut t = old_device(vec![1, 1, 0x17, 9, 1, 7, 0], 33, 6136);
        let msg = probe(&mut t).unwrap_err().to_string();
        assert!(
            msg.contains("firmware 1.1.7 (wire format V33), which is newer"),
            "{msg}"
        );
        assert!(msg.ends_with("Update DSPi Terminal to use it."), "{msg}");
        // Without a platform reply to go on, the version is unknown.
        assert!(refusal(None, 28).starts_with("This device runs firmware an unknown version"));
    }

    #[test]
    fn uses_device_supplied_channel_names() {
        let mut t = device();
        let caps = probe(&mut t).unwrap();
        assert_eq!(caps.channels[0].name, "Turntable");
        assert_eq!(caps.channels[0].slug, "turntable");
        assert!(!caps.channels[0].is_output);
        // Output channels begin at num_inputs, not at a constant.
        assert!(caps.channels[8].is_output);
        assert!(!caps.channels[7].is_output);
    }

    #[test]
    fn reads_capability_tables_rather_than_hardcoding_them() {
        let mut t = device();
        let caps = probe(&mut t).unwrap();
        let cs = caps.cs.unwrap();
        assert_eq!(cs.caps_version, 13);
        assert_eq!(cs.max_bindings, 16);
        assert_eq!(cs.noun_count, 57);
        assert_eq!(cs.max_ir_commands, 16);
        assert_eq!(cs.max_groups, 8);
        assert_eq!(cs.max_macros, 8);
        assert_eq!(cs.max_macro_steps, 8);

        let sg = caps.siggen.unwrap();
        assert_eq!(sg.type_count, 15);
        assert_eq!(sg.multitone_max, 16);
    }

    /// The post-table fields are found at `4 + 4*type_count`, never at a fixed
    /// offset: caps v10 added a ninth component type and moved them four bytes
    /// down. A build that assumed 36 would read the type table's last row as
    /// `max_ir_commands`.
    #[test]
    fn the_caps_maxima_follow_the_type_table_length() {
        for type_count in [8u8, 9, 12] {
            let mut v = vec![13, 16, type_count, 57];
            // A distinctive type table, so reading it as a maximum is obvious.
            v.extend(std::iter::repeat_n(0xEEu8, 4 * type_count as usize));
            v.extend([16, 8, 8, 8]);

            let mut t = MockTransport::new().data(op::REQ_GET_CS_CAPS, v);
            let cs = probe_cs_caps(&mut t).unwrap();
            assert_eq!(cs.type_count, type_count);
            assert_eq!(cs.max_ir_commands, 16, "with {type_count} types");
            assert_eq!(cs.max_macro_steps, 8, "with {type_count} types");
        }
    }

    /// Caps v20: eleven types, a 52-byte header, the maxima at 48
    /// (control_surfaces.h:637-652).
    #[test]
    fn a_v20_caps_header_is_read_past_eleven_types() {
        let mut v = vec![20, 16, 11, 79];
        v.extend(std::iter::repeat_n(0xEEu8, 4 * 11));
        v.extend([16, 8, 8, 8]);
        assert_eq!(v.len(), 52);
        let mut t = MockTransport::new().data(op::REQ_GET_CS_CAPS, v);
        let cs = probe_cs_caps(&mut t).unwrap();
        assert_eq!(
            (cs.caps_version, cs.type_count, cs.noun_count),
            (20, 11, 79)
        );
        assert_eq!(cs.types.len(), 11);
        assert_eq!(
            (
                cs.max_ir_commands,
                cs.max_groups,
                cs.max_macros,
                cs.max_macro_steps
            ),
            (16, 8, 8, 8)
        );
    }

    /// Pre-v9 firmware has nothing after `max_ir_commands`; the three maxima
    /// must read as zero rather than as whatever followed in the buffer.
    #[test]
    fn a_firmware_without_groups_reports_none() {
        let mut v = vec![6, 16, 8, 52];
        v.extend(std::iter::repeat_n(0u8, 4 * 8));
        v.extend([16, 0, 0, 0]);

        let mut t = MockTransport::new().data(op::REQ_GET_CS_CAPS, v);
        let cs = probe_cs_caps(&mut t).unwrap();
        assert_eq!(cs.max_ir_commands, 16);
        assert_eq!(
            (cs.max_groups, cs.max_macros, cs.max_macro_steps),
            (0, 0, 0)
        );
    }

    /// A stall means "this firmware lacks the feature", which is information we
    /// record, not an error that aborts discovery.
    #[test]
    fn an_absent_feature_is_reported_not_fatal() {
        let mut t = device();
        let caps = probe(&mut t).unwrap();
        let psybass = caps
            .features
            .iter()
            .find(|f| f.name == "psychoacoustic_bass")
            .unwrap();
        assert!(!psybass.present);
        assert!(psybass.evidence.contains("stalled"));
    }

    #[test]
    fn a_present_feature_is_detected() {
        let mut t = device().data(op::REQ_GET_PSYBASS, vec![1]);
        let caps = probe(&mut t).unwrap();
        assert!(
            caps.features
                .iter()
                .find(|f| f.name == "psychoacoustic_bass")
                .unwrap()
                .present
        );
    }

    /// Beta4's tools are probed one by one; the tube and the limiter carry
    /// their probe index in `wValue` (tube index 0, limiter.h:20 0x81).
    #[test]
    fn the_beta4_tools_are_probed_by_their_own_reads() {
        let mut t = device()
            .data(op::REQ_GET_SUBHARM, vec![0])
            .data(op::REQ_GET_TUBE_PARAM, 0.0f32.to_le_bytes().to_vec())
            .data(op::REQ_LIMITER, vec![0, 32, 16, 9])
            .data(op::REQ_RTA_GET_CAPS, vec![3; 16]);
        let log = t.log_handle();
        let caps = probe(&mut t).unwrap();
        for name in [
            "subharmonic_synth",
            "tube_preamp",
            "output_limiter",
            "spectrum_analyser",
        ] {
            let f = caps.features.iter().find(|f| f.name == name).unwrap();
            assert!(f.present, "{name}: {}", f.evidence);
        }
        let asked = |opcode: u8| {
            log.lock()
                .unwrap()
                .iter()
                .find(|e| e.opcode == opcode)
                .map(|e| e.value)
        };
        assert_eq!(asked(op::REQ_LIMITER), Some(0x0081));
        assert_eq!(asked(op::REQ_GET_TUBE_PARAM), Some(0));

        // And a beta3 device, which has none of the new ones but the subharm.
        let caps = probe(&mut device()).unwrap();
        assert!(
            caps.features
                .iter()
                .any(|f| f.name == "tube_preamp" && !f.present)
        );
    }

    /// The bulk packet exceeds the 4 KB WinUSB cap, so it must arrive in pieces.
    #[test]
    fn bulk_read_chunks_within_the_transfer_limit() {
        let mut t = device().with_max_transfer(512);
        let packet = read_bulk(&mut t).unwrap();
        assert_eq!(packet.as_bytes().len(), generated::BULK_SIZE);

        let chunk_reads = t
            .log()
            .iter()
            .filter(|e| e.opcode == op::REQ_GET_ALL_PARAMS_CHUNK)
            .count();
        assert!(
            chunk_reads > 1,
            "6136 bytes cannot arrive in one 512-byte transfer"
        );

        // Offsets must be sequential from 0: the firmware snapshots at offset 0.
        let offsets: Vec<u16> = t
            .log()
            .iter()
            .filter(|e| e.opcode == op::REQ_GET_ALL_PARAMS_CHUNK)
            .map(|e| e.value)
            .collect();
        assert_eq!(offsets[0], 0);
        assert!(offsets.windows(2).all(|w| w[1] > w[0]));
    }

    /// The tolerant probe describes the beta2 device the strict one refuses:
    /// its own header gives the channel model, its own version the format.
    #[test]
    fn wp_s2_probe_any_describes_an_older_device() {
        let mut t = old_device(vec![1, 1, 0x16, 9], 28, 5944);
        let (caps, read) = probe_any(&mut t).unwrap();
        assert_eq!(read, WireRead::Older);
        assert_eq!(caps.wire_format, 28);
        assert_eq!(caps.firmware, "1.1.6 early beta");
        assert_eq!(
            (caps.num_channels, caps.num_outputs, caps.num_inputs),
            (17, 9, 8)
        );
        assert_eq!(caps.channels.len(), 17);
        assert_eq!(caps.serial, "4BA1DDB9D1443D6A");
    }

    #[test]
    fn wp_s2_probe_any_describes_a_newer_device() {
        let mut t = old_device(vec![1, 1, 0x17, 9, 1, 7, 0], 33, 6200);
        let (caps, read) = probe_any(&mut t).unwrap();
        assert_eq!(read, WireRead::Newer);
        assert_eq!(caps.wire_format, 33);
        assert_eq!(caps.firmware, "1.1.7");
        let last = t
            .log()
            .iter()
            .filter(|e| e.opcode == op::REQ_GET_ALL_PARAMS_CHUNK)
            .map(|e| e.value)
            .max();
        assert_eq!(last, Some(6144), "read past 6136 to the header's length");
    }

    /// On this build's own format it reads what `probe` reads, and says the
    /// image is the writable one.
    #[test]
    fn wp_s2_probe_any_on_the_own_format_is_exact() {
        let (caps, read) = probe_any(&mut device()).unwrap();
        let strict = probe(&mut device()).unwrap();
        assert_eq!(read, WireRead::Exact);
        assert_eq!(caps.wire_format, 32);
        assert_eq!(caps.firmware, strict.firmware);
        assert_eq!(caps.num_channels, strict.num_channels);
        assert_eq!(caps.max_bands, strict.max_bands);
        assert_eq!(caps.active_preset, strict.active_preset);
        let names = |c: &Capabilities| {
            c.channels
                .iter()
                .map(|i| i.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&caps), names(&strict));
    }

    /// The image comes back with the description, so a connect reads it once.
    #[test]
    fn wp_s2_probe_any_with_image_reads_the_image_once() {
        let mut t = old_device(vec![1, 1, 0x16, 9], 28, 5944);
        let (caps, image) = probe_any_with_image(&mut t).unwrap();
        assert_eq!(image.read, WireRead::Older);
        assert_eq!(image.device_version, caps.wire_format);
        let first_chunks = t
            .log()
            .iter()
            .filter(|e| e.opcode == op::REQ_GET_ALL_PARAMS_CHUNK && e.value == 0)
            .count();
        assert_eq!(first_chunks, 1, "one snapshot");
    }

    /// What cannot be mapped is still refused, by name.
    #[test]
    fn wp_s2_probe_any_refuses_a_pre_v16_device_by_name() {
        let mut t = old_device(vec![1, 1, 0x15, 9], 15, 5000);
        let err = probe_any(&mut t).unwrap_err();
        let TransportError::Incompatible(msg) = &err else {
            panic!("expected a refusal, got {err:?}");
        };
        assert!(msg.contains("firmware 1.1.5 (wire format V15)"), "{msg}");
    }

    #[test]
    fn a_dead_device_fails_on_the_liveness_read() {
        let mut t = MockTransport::new().reply(op::REQ_GET_PLATFORM, Reply::Disconnect);
        assert!(matches!(probe(&mut t), Err(TransportError::Disconnected)));
    }
}

/// Live meter readings: one transfer for the whole device.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Meters {
    /// Per-channel peak, 0..1.
    pub peaks: Vec<f32>,
    /// Sticky per-channel clip latch.
    pub clipped: Vec<bool>,
    pub cpu0: u8,
    pub cpu1: u8,
    /// How many input channels are actually carrying audio right now, which the
    /// firmware works out from the active source.
    pub active_inputs: u8,
}

/// Read every meter at once.
///
/// `GET_STATUS` sub-query 9 returns per-channel peaks, both processor loads, the
/// sticky clip bitmask and the live input count in a single transfer, which is
/// why meters can be on screen everywhere without costing anything noticeable.
///
/// The layout is `peaks[n]*2, cpu0, cpu1, clip_flags(4), active_inputs(1)`, so
/// `n*2 + 7` bytes: 41 on RP2350, 21 on RP2040. Note that the released
/// `commands.md` documents `n*2 + 4` with a 16-bit clip mask, which is two
/// changes out of date: the mask became 32-bit and the input count was appended.
/// Reading the documented length would truncate, and a 16-bit mask cannot even
/// represent channel 17's clip flag.
pub fn read_meters(t: &mut dyn Transport, num_channels: u8) -> Result<Meters> {
    let n = num_channels as usize;
    let len = n * 2 + 7;
    let d = t.control_in(op::REQ_GET_STATUS, 9, len as u16)?;

    // Q15 full scale, 0..32767, on every path (audio_pipeline.c:424, 704, 974).
    let peaks = (0..n)
        .map(|i| u16::from_le_bytes([d[i * 2], d[i * 2 + 1]]) as f32 / 32767.0)
        .collect();

    let base = n * 2;
    let flags = u32::from_le_bytes([d[base + 2], d[base + 3], d[base + 4], d[base + 5]]);

    Ok(Meters {
        peaks,
        // Shift on u32, not u16: channel 17 exists and its bit is real. A
        // channel count past 32 from a confused device has no flag.
        clipped: (0..n)
            .map(|i| flags.checked_shr(i as u32).unwrap_or(0) & 1 != 0)
            .collect(),
        cpu0: d[base],
        cpu1: d[base + 1],
        active_inputs: d[base + 6],
    })
}

#[cfg(test)]
mod meter_tests {
    use super::*;
    use dspi_transport::MockTransport;

    fn packet(peaks: &[u16], cpu: (u8, u8), clip: u32, inputs: u8) -> Vec<u8> {
        let mut d = Vec::new();
        for p in peaks {
            d.extend(p.to_le_bytes());
        }
        d.push(cpu.0);
        d.push(cpu.1);
        d.extend(clip.to_le_bytes());
        d.push(inputs);
        d
    }

    #[test]
    fn meters_decode_peaks_cpu_clip_flags_and_input_count() {
        let d = packet(&[32767, 16384], (34, 8), 0b10, 2);
        let mut t = MockTransport::new().data(op::REQ_GET_STATUS, d);
        let m = read_meters(&mut t, 2).unwrap();

        assert!((m.peaks[0] - 1.0).abs() < 1e-4);
        assert!((m.peaks[1] - 0.5).abs() < 1e-3);
        assert_eq!((m.cpu0, m.cpu1), (34, 8));
        assert_eq!(m.clipped, vec![false, true]);
        assert_eq!(m.active_inputs, 2);
    }

    /// The packet length follows the channel count, and the documented
    /// `n*2 + 4` would truncate it.
    #[test]
    fn the_read_length_follows_the_channel_count() {
        let d = packet(&[0; 17], (0, 0), 0, 8);
        assert_eq!(d.len(), 17 * 2 + 7, "RP2350 status packet is 41 bytes");

        let mut t = MockTransport::new().data(op::REQ_GET_STATUS, d);
        assert!(read_meters(&mut t, 17).is_ok());
        assert_eq!(t.log()[0].value, 9, "sub-query 9 is the combined packet");
    }

    /// Channel 17 has no bit in a 16-bit mask. Shifting on the wrong width
    /// either overflows or silently loses the PDM subwoofer's clip flag.
    #[test]
    fn the_seventeenth_channel_clip_flag_survives() {
        let d = packet(&[0; 17], (0, 0), 1 << 16, 8);
        let mut t = MockTransport::new().data(op::REQ_GET_STATUS, d);
        let m = read_meters(&mut t, 17).unwrap();
        assert_eq!(m.clipped.len(), 17);
        assert!(m.clipped[16], "the PDM sub's clip flag was lost");
        assert!(!m.clipped[15]);
    }

    /// A channel count past the flag word's 32 bits, from a confused device,
    /// reads as unclipped rather than overflowing the shift.
    #[test]
    fn a_channel_past_the_flag_word_is_not_clipped() {
        let d = packet(&[0; 40], (0, 0), u32::MAX, 8);
        let mut t = MockTransport::new().data(op::REQ_GET_STATUS, d);
        let m = read_meters(&mut t, 40).unwrap();
        assert!(m.clipped[31]);
        assert!(!m.clipped[32] && !m.clipped[39]);
    }

    #[test]
    fn an_rp2040_packet_is_twentyone_bytes() {
        let d = packet(&[0; 7], (12, 0), 0, 2);
        assert_eq!(d.len(), 21);
        let mut t = MockTransport::new().data(op::REQ_GET_STATUS, d);
        assert_eq!(read_meters(&mut t, 7).unwrap().cpu0, 12);
    }
}
