//! The channel clipboard in the core (WP CLIP1, PARITY M45): the Console's
//! Copy Parameters and Paste Parameters (`DSPViewModel.copyChannelParams` and
//! `pasteChannelParams`), one implementation for every client.
//!
//! - [`DspiCore::copy_channel`] reads a channel from the mirror (no device
//!   traffic) into a [`ChannelClip`]: its PEQ bank, and for an output its
//!   crossover bank, gain, delay and mute.
//! - [`DspiCore::paste_channel`] puts a clip onto a channel through the
//!   typed setters, so the client read-only mode, a read-only connection,
//!   the installer's lock and a limits profile judge it as any write, in one
//!   undo group: one undo puts everything back.
//! - [`channel_clip_to_json`] and [`channel_clip_from_json`] turn a clip into
//!   a versioned JSON document a client can keep (across sessions, on its own
//!   clipboard) and back; a document of another version is refused.
//!
//! What crosses (the Console's rules):
//!
//! - **PEQ** always: every band of the bank is written, the ones the clip
//!   does not reach flat, so the destination becomes the source rather than a
//!   merge. Bands past the bank are ignored. With a linked partner
//!   ([`ChannelPasteOptions::mirror`]) the partner's bank gets the same
//!   bands, as a linked pair mirrors every other filter edit.
//! - **Crossover, gain, delay and mute** only from an output to an output,
//!   and only to the destination (not the partner). A clip taken from an
//!   input carries none of them, and an output's crossover bank is left
//!   alone rather than cleared. An input destination takes the PEQ only.
//! - The input trim does not travel (the Console's clipboard has none).
//! - The delay is clamped to this device's delay line
//!   ([`DspiCore::max_delay_ms`]), so a clip taken on an RP2350 pastes onto
//!   an RP2040 instead of being refused.
//!
//! A paste is checked before anything is sent: the clip's version, the
//! channels, every value, the connection's writability and the installer's
//! lock, and the limits profile over the whole sequence of writes. A refusal
//! there writes nothing. A transfer failure partway leaves the writes made
//! so far in the one undo step.
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dspi_proto::FilterType;
use dspi_proto::registry::{Kind, by_path};
use dspi_session::limits::{BandView, LimitsProfile, Violation};
use serde::{Deserialize, Serialize};

use crate::consts::{BANDS_PER_CHANNEL, MAX_XOVER_BANDS};
use crate::records::{DspState, FilterParams};
use crate::{DspiCore, DspiError, Result, quantize_delay, quantize_gain, state};

/// The clip format this core writes and reads ([`ChannelClip::version`]).
pub const CHANNEL_CLIP_VERSION: u32 = 1;

/// The `kind` of a clip document ([`channel_clip_to_json`]).
pub const CHANNEL_CLIP_KIND: &str = "dspi.channelClip";

/// The undo step's label for a paste ([`crate::HistoryEntry::label`]).
pub const PASTE_LABEL: &str = "Paste Parameters";

/// The longest clip document [`channel_clip_from_json`] reads.
const MAX_DOCUMENT_LEN: usize = 64 * 1024;

/// An output's strip as a clip carries it.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelClipOutput {
    /// Output gain in dB.
    pub gain_db: f32,
    /// Output delay in ms.
    pub delay_ms: f32,
    /// Whether the output is muted.
    pub muted: bool,
}

/// What [`DspiCore::copy_channel`] took from a channel. A value: a client
/// keeps it as it likes, as a record or as its JSON document
/// ([`channel_clip_to_json`]).
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelClip {
    /// The clip format ([`CHANNEL_CLIP_VERSION`]); a paste refuses another.
    pub version: u32,
    /// The wire channel it was copied from (inputs, then outputs).
    pub source_channel: u8,
    /// The source's name on the device ("" when unnamed); a client may put
    /// the name it shows here, for its "Copied ..." line.
    pub source_name: String,
    /// The PEQ bank, band 0 first.
    pub filters: Vec<FilterParams>,
    /// The crossover bank, slot 0 first: present when copied from an
    /// output, absent from an input.
    pub crossover: Option<Vec<FilterParams>>,
    /// Gain, delay and mute: present when copied from an output.
    pub output: Option<ChannelClipOutput>,
}

/// How [`DspiCore::paste_channel`] pastes.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ChannelPasteOptions {
    /// The destination's linked partner (a wire channel), when the client
    /// has the pair linked: its PEQ bank gets the same bands.
    pub mirror: Option<u8>,
}

/// One write of a paste, in the order it is made.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    Peq { ch: u8, band: u8, p: FilterParams },
    Xover { ch: u8, xband: u8, p: FilterParams },
    Gain { output: u8, db: f32 },
    Delay { output: u8, ms: f32 },
    Mute { output: u8, muted: bool },
}

impl Step {
    /// A band write's channel, band on the wire and values.
    fn band(&self) -> Option<(u8, u8, FilterParams)> {
        match *self {
            Self::Peq { ch, band, p } => Some((ch, band, p)),
            Self::Xover { ch, xband, p } => Some((ch, xband.saturating_add(XOVER_BASE), p)),
            _ => None,
        }
    }
}

/// The first crossover band on the wire.
const XOVER_BASE: u8 = {
    assert!(dspi_proto::generated::filter::XOVER_BAND_BASE <= u8::MAX as u16);
    dspi_proto::generated::filter::XOVER_BAND_BASE as u8
};

fn invalid(msg: String) -> DspiError {
    DspiError::InvalidArgument { msg }
}

/// A flat band, as the Console's `FilterParams()` (its Clear).
fn flat() -> FilterParams {
    FilterParams {
        filter_type: 0,
        bypass: false,
        freq: 1000.0,
        q: 0.707,
        gain: 0.0,
        qp: 0.707,
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    // -- WP CLIP1 --
    /// Copy wire channel `ch` (inputs, then outputs) into a clip, from the
    /// mirror: nothing is sent. An input gives its PEQ bank; an output its
    /// PEQ and crossover banks, gain, delay and mute.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device (or document)
    /// open;
    /// [`DspiError::InvalidArgument`] for a channel out of range.
    pub fn copy_channel(&self, ch: u8) -> Result<ChannelClip> {
        let inner = self.lock();
        if inner.conn.is_none() {
            return Err(DspiError::Disconnected);
        }
        copy_from(&inner.state, ch)
    }

    // -- WP CLIP1 --
    /// Paste `clip` onto wire channel `ch`, by the Console's rules (module
    /// docs): the PEQ bank (onto `options.mirror` too), and from an output
    /// to an output the crossover bank, gain, delay (clamped to this
    /// device's line) and mute. Every write goes through the typed setter,
    /// in one undo group labelled [`PASTE_LABEL`]: one undo step puts every
    /// value back.
    ///
    /// Everything is checked before the first write, and a refusal writes
    /// nothing: the clip's version, the channels, the values, the
    /// connection (no device, the client read-only mode, a read-only wire
    /// format, the installer's lock) and a limits profile over the whole
    /// paste. A transfer failure partway stops the paste; what was written
    /// stays, as one undo step.
    ///
    /// Errors: [`DspiError::InvalidArgument`] for another clip version, a
    /// channel or partner out of range (or the partner the channel itself),
    /// a non-finite value or a crossover slot of a PEQ type; the refusals
    /// of every setter ([`DspiError::Disconnected`], [`DspiError::ReadOnly`],
    /// [`DspiError::InstallerLock`], [`DspiError::LimitExceeded`]) and its
    /// transfer errors.
    pub fn paste_channel(
        &self,
        clip: ChannelClip,
        ch: u8,
        options: ChannelPasteOptions,
    ) -> Result<()> {
        let steps = {
            let mut inner = self.writing()?;
            let mut steps = plan(&inner.state, &clip, ch, options)?;
            if let Some(p) = crate::limits::profile(&mut inner) {
                order_for_highpass(&p, &inner.state, &mut steps);
                judge(&p, &inner.state, &steps)?;
            }
            steps
        };
        self.begin_group(PASTE_LABEL.into());
        let r = steps.iter().try_for_each(|s| self.paste_step(s));
        self.end_group();
        r
    }
}

impl DspiCore {
    /// Make one write of a paste through its setter.
    fn paste_step(&self, s: &Step) -> Result<()> {
        match *s {
            Step::Peq { ch, band, p } => self.set_filter(ch, band, p),
            Step::Xover { ch, xband, p } => self.set_crossover(ch, xband, p),
            Step::Gain { output, db } => self.set_output_gain(output, db),
            Step::Delay { output, ms } => self.set_output_delay(output, ms),
            Step::Mute { output, muted } => self.set_output_mute(output, muted),
        }
    }
}

/// The output whose wire channel is `ch`, if it is one.
fn output_of(s: &DspState, ch: u8) -> Option<u8> {
    ch.checked_sub(s.num_input_channels)
        .filter(|o| *o < s.num_output_channels)
}

/// `ch` checked against the mirror's channels.
fn channel(s: &DspState, ch: u8, what: &str) -> Result<usize> {
    if s.num_channels == 0 {
        return Err(DspiError::Disconnected);
    }
    if ch < s.num_channels {
        Ok(usize::from(ch))
    } else {
        Err(invalid(format!(
            "{what} {ch} is out of range (0..{})",
            s.num_channels
        )))
    }
}

/// [`DspiCore::copy_channel`] on a mirror.
fn copy_from(s: &DspState, ch: u8) -> Result<ChannelClip> {
    let c = channel(s, ch, "channel")?;
    let bank = |banks: &[Vec<FilterParams>], n: usize| -> Vec<FilterParams> {
        banks
            .get(c)
            .map(|b| b.iter().take(n).copied().collect())
            .unwrap_or_default()
    };
    let output = output_of(s, ch).map(usize::from);
    Ok(ChannelClip {
        version: CHANNEL_CLIP_VERSION,
        source_channel: ch,
        source_name: s.channel_names.get(c).cloned().unwrap_or_default(),
        filters: bank(&s.filters, BANDS_PER_CHANNEL),
        crossover: output.map(|_| bank(&s.xover, MAX_XOVER_BANDS)),
        output: output.map(|o| ChannelClipOutput {
            gain_db: s.output_gain_db.get(o).copied().unwrap_or(0.0),
            delay_ms: s.output_delay_ms.get(o).copied().unwrap_or(0.0),
            muted: s.output_muted.get(o).copied().unwrap_or(false),
        }),
    })
}

/// A clip's version, refused unless it is this core's.
fn check_version(version: u32) -> Result<()> {
    if version == CHANNEL_CLIP_VERSION {
        Ok(())
    } else {
        Err(invalid(format!(
            "a channel clip of version {version}; this core reads version {CHANNEL_CLIP_VERSION}"
        )))
    }
}

fn finite(v: f32, what: &str) -> Result<f32> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(invalid(format!("{what} is not a finite number ({v})")))
    }
}

/// A band as its setter shapes it, checked: finite values; a PEQ gain on
/// the 0.1 dB grid unless it is a Linkwitz Transform's fp; a Linkwitz
/// Transform's Q clamped to 0.1..20.
fn shaped_band(mut p: FilterParams, peq: bool) -> Result<FilterParams> {
    finite(p.freq, "frequency")?;
    finite(p.q, "Q")?;
    finite(p.gain, "gain")?;
    let t = FilterType::from_raw(p.filter_type);
    if t.is_linkwitz() {
        p.qp = finite(p.qp, "Linkwitz target Q")?.clamp(0.1, 20.0);
    } else if peq {
        p.gain = quantize_gain(p.gain);
    }
    if !peq && t != FilterType::Flat && !t.is_crossover() {
        return Err(invalid(format!(
            "crossover slot of filter type {}, not a crossover type",
            p.filter_type
        )));
    }
    Ok(p)
}

/// The registry's range for `path`, clamped (the setters' clamp).
fn clamped(path: &str, v: f32) -> f32 {
    match by_path(path).map(|d| d.kind) {
        Some(Kind::Float { min, max, .. }) => v.clamp(min, max),
        _ => v,
    }
}

/// The writes that put `clip` onto `ch` on this mirror, in order, every
/// value checked and shaped as its setter shapes it.
fn plan(
    s: &DspState,
    clip: &ChannelClip,
    ch: u8,
    options: ChannelPasteOptions,
) -> Result<Vec<Step>> {
    check_version(clip.version)?;
    channel(s, ch, "channel")?;
    if let Some(m) = options.mirror {
        channel(s, m, "linked partner")?;
        if m == ch {
            return Err(invalid(format!(
                "the linked partner {m} is the channel itself"
            )));
        }
    }
    let mut steps = Vec::new();
    let peq: Vec<FilterParams> = (0..BANDS_PER_CHANNEL)
        .map(|b| shaped_band(clip.filters.get(b).copied().unwrap_or_else(flat), true))
        .collect::<Result<_>>()?;
    for target in std::iter::once(ch).chain(options.mirror) {
        for (band, p) in (0u8..).zip(&peq) {
            steps.push(Step::Peq {
                ch: target,
                band,
                p: *p,
            });
        }
    }
    if let (Some(output), Some(strip)) = (output_of(s, ch), clip.output) {
        if let Some(xo) = &clip.crossover {
            for (xband, p) in (0u8..).zip(xo.iter().take(MAX_XOVER_BANDS)) {
                steps.push(Step::Xover {
                    ch,
                    xband,
                    p: shaped_band(*p, false)?,
                });
            }
        }
        let db = quantize_gain(clamped("out.gain", finite(strip.gain_db, "output gain")?));
        let max = state::max_delay_ms_for(s);
        let ms = quantize_delay(finite(strip.delay_ms, "output delay")?).clamp(0.0, max);
        steps.push(Step::Gain { output, db });
        steps.push(Step::Delay { output, ms });
        steps.push(Step::Mute {
            output,
            muted: strip.muted,
        });
    }
    Ok(steps)
}

fn view(p: &FilterParams) -> BandView {
    BandView {
        filter_type: p.filter_type,
        bypass: p.bypass,
        freq: p.freq,
    }
}

/// Put the band writes that by themselves keep an output's high-pass first,
/// so a paste that ends with the high-pass kept never passes through a state
/// without it (the limits profile judges every write on its own).
fn order_for_highpass(p: &LimitsProfile, s: &DspState, steps: &mut [Step]) {
    let keeps = |st: &Step| {
        st.band().is_some_and(|(ch, _, b)| {
            output_of(s, ch).is_some_and(|o| p.highpass_kept(o, &[view(&b)]))
        })
    };
    // Only the bands move: they come before the output strip in a plan.
    let bands = steps.iter().take_while(|st| st.band().is_some()).count();
    if let Some(head) = steps.get_mut(..bands) {
        head.sort_by_key(|st| !keeps(st));
    }
}

fn refused(v: Violation) -> DspiError {
    DspiError::LimitExceeded {
        code: v.code.to_owned(),
        msg: v.msg,
    }
}

/// The bank of wire channel `ch` as the limits rules see it: PEQ bands
/// (wire 0..), then the crossover (wire 20..).
fn bank(s: &DspState, ch: u8) -> Vec<(u8, FilterParams)> {
    let c = usize::from(ch);
    let peq = s.filters.get(c).map(Vec::as_slice).unwrap_or_default();
    let xo = s.xover.get(c).map(Vec::as_slice).unwrap_or_default();
    (0u8..)
        .zip(peq.iter().copied())
        .chain((XOVER_BASE..).zip(xo.iter().copied()))
        .collect()
}

/// Judge the whole paste against the limits profile, write by write in
/// order, as the setters would judge each: an EQ boost, an output's
/// high-pass, its gain and its delay.
fn judge(p: &LimitsProfile, s: &DspState, steps: &[Step]) -> Result<()> {
    let mut banks: Vec<(u8, Vec<(u8, FilterParams)>)> = Vec::new();
    for st in steps {
        match *st {
            Step::Gain { output, db } => p.check_output_gain(output, db).map_err(refused)?,
            Step::Delay { output, ms } => p.check_delay(output, ms).map_err(refused)?,
            Step::Mute { .. } => {}
            Step::Peq { .. } | Step::Xover { .. } => {
                let Some((ch, band, b)) = st.band() else {
                    continue;
                };
                p.check_boost(b.filter_type, b.gain).map_err(refused)?;
                let Some(o) = output_of(s, ch) else {
                    continue;
                };
                let i = match banks.iter().position(|(c, _)| *c == ch) {
                    Some(i) => i,
                    None => {
                        banks.push((ch, bank(s, ch)));
                        banks.len().saturating_sub(1)
                    }
                };
                let Some((_, bank)) = banks.get_mut(i) else {
                    continue;
                };
                let before: Vec<BandView> = bank.iter().map(|(_, b)| view(b)).collect();
                if let Some(slot) = bank.iter_mut().find(|(n, _)| *n == band) {
                    slot.1 = b;
                }
                let after: Vec<BandView> = bank.iter().map(|(_, b)| view(b)).collect();
                p.check_highpass(o, &before, &after).map_err(refused)?;
            }
        }
    }
    Ok(())
}

// ── The clip document ──────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BandDoc {
    filter_type: u8,
    bypass: bool,
    freq: f32,
    q: f32,
    gain: f32,
    qp: f32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OutputDoc {
    gain_db: f32,
    delay_ms: f32,
    muted: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClipDoc {
    kind: String,
    version: u32,
    source_channel: u8,
    source_name: String,
    filters: Vec<BandDoc>,
    crossover: Option<Vec<BandDoc>>,
    output: Option<OutputDoc>,
}

impl From<&FilterParams> for BandDoc {
    fn from(p: &FilterParams) -> Self {
        Self {
            filter_type: p.filter_type,
            bypass: p.bypass,
            freq: p.freq,
            q: p.q,
            gain: p.gain,
            qp: p.qp,
        }
    }
}

impl From<&BandDoc> for FilterParams {
    fn from(d: &BandDoc) -> Self {
        Self {
            filter_type: d.filter_type,
            bypass: d.bypass,
            freq: d.freq,
            q: d.q,
            gain: d.gain,
            qp: d.qp,
        }
    }
}

fn finite_band(p: &FilterParams) -> Result<()> {
    for (v, what) in [
        (p.freq, "frequency"),
        (p.q, "Q"),
        (p.gain, "gain"),
        (p.qp, "qp"),
    ] {
        finite(v, what)?;
    }
    Ok(())
}

// -- WP CLIP1 --
/// A clip as a JSON document to keep: `{kind: "dspi.channelClip", version,
/// sourceChannel, sourceName, filters: [{filterType, bypass, freq, q, gain,
/// qp}], crossover: [...] | null, output: {gainDb, delayMs, muted} | null}`.
///
/// Errors: [`DspiError::InvalidArgument`] for a clip of another version or
/// with a non-finite value (JSON has no NaN).
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn channel_clip_to_json(clip: ChannelClip) -> Result<String> {
    check_version(clip.version)?;
    let mut bands = clip.filters.iter().chain(clip.crossover.iter().flatten());
    bands.try_for_each(finite_band)?;
    if let Some(o) = clip.output {
        finite(o.gain_db, "output gain")?;
        finite(o.delay_ms, "output delay")?;
    }
    let doc = ClipDoc {
        kind: CHANNEL_CLIP_KIND.to_owned(),
        version: clip.version,
        source_channel: clip.source_channel,
        source_name: clip.source_name,
        filters: clip.filters.iter().map(BandDoc::from).collect(),
        crossover: clip
            .crossover
            .as_ref()
            .map(|x| x.iter().map(BandDoc::from).collect()),
        output: clip.output.map(|o| OutputDoc {
            gain_db: o.gain_db,
            delay_ms: o.delay_ms,
            muted: o.muted,
        }),
    };
    serde_json::to_string(&doc).map_err(|e| invalid(format!("the clip could not be encoded: {e}")))
}

// -- WP CLIP1 --
/// A clip read back from its document ([`channel_clip_to_json`]).
///
/// Errors: [`DspiError::InvalidArgument`] for text that is not a channel
/// clip (too long, not JSON, another `kind`, a missing or mistyped field)
/// and for a clip of another version, older or newer, named in the message.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn channel_clip_from_json(text: String) -> Result<ChannelClip> {
    if text.len() > MAX_DOCUMENT_LEN {
        return Err(invalid(format!(
            "a channel clip of {} bytes (at most {MAX_DOCUMENT_LEN})",
            text.len()
        )));
    }
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| invalid(format!("not a channel clip: {e}")))?;
    if v.get("kind").and_then(serde_json::Value::as_str) != Some(CHANNEL_CLIP_KIND) {
        return Err(invalid(format!(
            "not a channel clip (kind is not \"{CHANNEL_CLIP_KIND}\")"
        )));
    }
    let version = v
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| invalid("a channel clip without a version".into()))?;
    check_version(u32::try_from(version).unwrap_or(u32::MAX))?;
    let doc: ClipDoc =
        serde_json::from_value(v).map_err(|e| invalid(format!("not a channel clip: {e}")))?;
    Ok(ChannelClip {
        version: doc.version,
        source_channel: doc.source_channel,
        source_name: doc.source_name,
        filters: doc.filters.iter().map(FilterParams::from).collect(),
        crossover: doc
            .crossover
            .as_ref()
            .map(|x| x.iter().map(FilterParams::from).collect()),
        output: doc.output.map(|o| ChannelClipOutput {
            gain_db: o.gain_db,
            delay_ms: o.delay_ms,
            muted: o.muted,
        }),
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_clip1;
