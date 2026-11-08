//! The channel clipboard: the Console's `copyChannelParams` /
//! `pasteChannelParams`.
//!
//! Moved here from the TUI (`dspi-tui/src/screens/clipboard.rs`, WP S5 of
//! docs/core-merge/PLAN.md) so the TUI, the desktop core and the mobile
//! bindings copy and paste a channel the same way.
//!
//! A copy ([`copy_channel`]) takes a snapshot of what makes a channel sound
//! the way it does: its PEQ bank, and for an output its crossover bank and
//! its gain, delay and mute (for an input, its trim). A paste is planned as a
//! list of typed [`PasteWrite`]s ([`paste_plan`]); a GUI applies them with
//! [`apply_paste`], and the TUI renders each as the command line that does the
//! same, so its echo line shows exactly what was done.
//!
//! What a GUI must supply that is not on the device (the name it shows for a
//! channel the device leaves unnamed) comes through [`ClipboardContext`].

#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
#![warn(missing_docs)]

use dspi_proto::FilterType;
use dspi_proto::registry;
use dspi_proto::value::{EqParamPacket, Value};

use crate::fast::DEFAULT_QP;
use crate::probe::Capabilities;
use crate::{DeviceState, Outcome, Session, WriteError};

/// The first crossover band, in the firmware's band numbering (20 to 23).
pub const FIRST_CROSSOVER_BAND: u8 = 20;

/// How many crossover bands an output has.
pub const CROSSOVER_BANDS: usize = 4;

/// What a GUI supplies to the clipboard beyond the device state.
///
/// For Rust callers (the TUI implements it to name channels with the
/// Console's descriptors). The bindings (UniFFI, the C ABI) do not implement
/// it across the boundary: they copy with the [`DeviceState`] context and
/// set [`ChannelClipboard::source`] to the name their GUI shows.
pub trait ClipboardContext {
    /// The device state a copy reads.
    fn state(&self) -> &DeviceState;

    /// The name a copy records as its source, for the "Copied ..." line.
    ///
    /// The default is the device's own name for the channel, or `in.N` /
    /// `out.N` when the device leaves it unnamed; the TUI overrides it with
    /// the Console's descriptors ("FL", "Sub", ...).
    fn channel_name(&self, channel: usize) -> String {
        let state = self.state();
        let name = state.channel_name(channel);
        if !name.is_empty() {
            return name;
        }
        let ni = usize::from(state.caps.num_inputs);
        match channel.checked_sub(ni) {
            None => format!("in.{}", channel.saturating_add(1)),
            Some(o) => format!("out.{}", o.saturating_add(1)),
        }
    }
}

/// The device state alone is a context: unnamed channels read `in.N`/`out.N`.
impl ClipboardContext for DeviceState {
    fn state(&self) -> &DeviceState {
        self
    }
}

/// What a copy captured. The source name is kept so the paste can say where
/// it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelClipboard {
    /// The source channel's name, from [`ClipboardContext::channel_name`].
    pub source: String,
    /// The PEQ bank, band 0 first.
    pub bands: Vec<EqParamPacket>,
    /// Outputs only: the crossover bank travels between outputs and is left
    /// alone when the clipboard came from an input.
    pub crossover: Option<Vec<EqParamPacket>>,
    /// Inputs only.
    pub preamp_db: Option<f32>,
    /// Outputs only: gain, delay, mute.
    pub output: Option<(f32, f32, bool)>,
}

impl ChannelClipboard {
    /// What the echo line says after a copy.
    pub fn copied_message(&self) -> String {
        format!("Copied {} parameters", self.source)
    }
}

/// One write a paste makes, in the order it makes them.
#[derive(Debug, Clone, PartialEq)]
pub enum PasteWrite {
    /// A whole EQ band; the packet's `channel` and `band` are the target's.
    Band(EqParamPacket),
    /// An input's trim (`pre`).
    Preamp {
        /// The input, 0-based.
        input: usize,
        /// Trim in dB.
        db: f32,
    },
    /// An output's gain (`out.gain`).
    OutputGain {
        /// The output, 0-based (not the unified channel).
        output: usize,
        /// Gain in dB.
        db: f32,
    },
    /// An output's delay (`out.delay`).
    OutputDelay {
        /// The output, 0-based.
        output: usize,
        /// Delay in milliseconds.
        ms: f32,
    },
    /// An output's mute (`out.mute`).
    OutputMute {
        /// The output, 0-based.
        output: usize,
        /// Whether it is muted.
        mute: bool,
    },
}

/// A paste that stopped partway, or (with `done == 0` and no outcomes) one
/// [`validate_paste`] refused before the first write.
#[derive(Debug, thiserror::Error)]
#[error("paste stopped after {done} of {total} writes: {source}")]
pub struct PasteError {
    /// How many writes went through before the failure.
    pub done: usize,
    /// How many the paste had to make.
    pub total: usize,
    /// What each write that went through did, in plan order (`done` of them),
    /// so a GUI knows what on the destination has changed.
    pub outcomes: Vec<Outcome>,
    /// The write that failed.
    #[source]
    pub source: WriteError,
}

/// Crossovers are addressable from wire format V11 (bands 20 to 23).
pub fn supports_crossover(state: &DeviceState) -> bool {
    state.caps.wire_format >= 11
}

/// Per-band bypass shipped in firmware 1.1.4 (`DSPViewModel.firmwareSupports
/// BandBypass`); older firmware has no bypass, so a paste sends it clear.
pub fn supports_band_bypass(caps: &Capabilities) -> bool {
    firmware_triple(&caps.firmware) >= (1, 1, 4)
}

/// A firmware version string as a comparable triple; an unparseable version
/// reads as `(0, 0, 0)`, the Console's conservative default. The same
/// parse as the TUI's `firmware_triple`.
fn firmware_triple(version: &str) -> (u32, u32, u32) {
    let mut parts = version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// A band as a paste writes it onto `channel`/`band`: the bypass flag only
/// where the firmware has one and the band is not flat, as the TUI's `eq`
/// lines do.
fn retarget(p: &EqParamPacket, channel: u8, band: u8, bypass_ok: bool) -> EqParamPacket {
    EqParamPacket {
        channel,
        band,
        bypass: p.bypass && p.filter_type != FilterType::Flat && bypass_ok,
        ..*p
    }
}

/// A band reset to its default (flat) state, as the Console's Clear does.
pub fn cleared_band(channel: u8, band: u8) -> EqParamPacket {
    EqParamPacket {
        channel,
        band,
        filter_type: FilterType::Flat,
        bypass: false,
        freq: 1000.0,
        q: 0.707,
        gain_db: 0.0,
        qp: None,
    }
}

/// Take a channel's parameters (unified channel space: inputs, then outputs).
pub fn copy_channel(cx: &impl ClipboardContext, channel: usize) -> ChannelClipboard {
    let state = cx.state();
    let ni = usize::from(state.caps.num_inputs);
    // `Some` for an output, as its 0-based output index.
    let output = channel.checked_sub(ni);
    let ch = channel_u8(channel);
    ChannelClipboard {
        source: cx.channel_name(channel),
        bands: state.bands(ch),
        crossover: output.map(|_| state.xover_bands(ch)),
        preamp_db: output.is_none().then(|| state.preamp_db(channel)),
        output: output.map(|o| {
            let o = state.output(o);
            (o.gain_db, o.delay_ms, o.mute)
        }),
    }
}

/// The writes that put a clipboard onto `channel`, and onto its linked
/// partner `mirror` when it has one.
///
/// Every band of the bank is written, the ones the clipboard does not reach
/// cleared, so the destination is the source rather than a merge of the two.
/// The trim goes to every input target; the crossover bank (when the firmware
/// has one) and the output strip go to `channel` alone, when it is an output:
/// they do not follow onto `mirror`, as they never did in the TUI.
///
/// A band keeps its bypass flag only on firmware with band bypass
/// ([`supports_band_bypass`]) and when it is not flat, so a clipboard taken
/// on one device pastes cleanly onto older firmware.
pub fn paste_plan(
    clip: &ChannelClipboard,
    state: &DeviceState,
    channel: usize,
    mirror: Option<usize>,
) -> Vec<PasteWrite> {
    let ni = usize::from(state.caps.num_inputs);
    let bypass_ok = supports_band_bypass(&state.caps);
    let mut out = Vec::new();

    for target in std::iter::once(channel).chain(mirror) {
        let ch = channel_u8(target);
        for band in 0..state.caps.max_bands {
            let p = clip
                .bands
                .get(usize::from(band))
                .copied()
                .unwrap_or_else(|| cleared_band(ch, band));
            out.push(PasteWrite::Band(retarget(&p, ch, band, bypass_ok)));
        }
        if let Some(db) = clip.preamp_db
            && target < ni
        {
            out.push(PasteWrite::Preamp { input: target, db });
        }
    }

    if let Some(o) = channel.checked_sub(ni) {
        let ch = channel_u8(channel);
        if let Some(xo) = &clip.crossover
            && supports_crossover(state)
        {
            for (band, p) in (FIRST_CROSSOVER_BAND..).zip(xo.iter().take(CROSSOVER_BANDS)) {
                out.push(PasteWrite::Band(retarget(p, ch, band, bypass_ok)));
            }
        }
        if let Some((gain, delay, mute)) = clip.output {
            out.push(PasteWrite::OutputGain {
                output: o,
                db: gain,
            });
            out.push(PasteWrite::OutputDelay {
                output: o,
                ms: delay,
            });
            out.push(PasteWrite::OutputMute { output: o, mute });
        }
    }
    out
}

/// The registry path, indices and value of a non-band write.
fn param_write(w: &PasteWrite) -> Option<(&'static str, u8, Value)> {
    match *w {
        PasteWrite::Band(_) => None,
        PasteWrite::Preamp { input, db } => Some(("pre", channel_u8(input), Value::Float(db))),
        PasteWrite::OutputGain { output, db } => {
            Some(("out.gain", channel_u8(output), Value::Float(db)))
        }
        PasteWrite::OutputDelay { output, ms } => {
            Some(("out.delay", channel_u8(output), Value::Float(ms)))
        }
        PasteWrite::OutputMute { output, mute } => {
            Some(("out.mute", channel_u8(output), Value::Bool(mute)))
        }
    }
}

/// Check every write of a paste against the session without making any
/// ([`Session::check`]): targets, ranges and platform limits. A band is
/// checked field by field as the registry's rows on its channel and band,
/// taking the fields its type uses (`registry::eq_field_fits`): `eq.freq`,
/// `eq.q` (except for a crossover type, whose Q the firmware derives) and
/// `eq.gain`, or for a Linkwitz Transform `eq.fp` and `eq.qp` instead of
/// `eq.gain`. Returns the index of the first bad write with its error.
pub fn validate_paste(session: &Session, plan: &[PasteWrite]) -> Result<(), (usize, WriteError)> {
    for (i, w) in plan.iter().enumerate() {
        let r = match (w, param_write(w)) {
            (PasteWrite::Band(p), _) => check_band(session, p),
            (_, Some((path, index, value))) => session.check(path, &[index], &value),
            // `param_write` answers every non-band write.
            (_, None) => Ok(()),
        };
        r.map_err(|e| (i, e))?;
    }
    Ok(())
}

// -- WP CG35 -- (review c6v: a Linkwitz band's fp is not a gain)
/// [`validate_paste`] for one band: each field its type takes, by the
/// registry's row.
fn check_band(session: &Session, p: &EqParamPacket) -> Result<(), WriteError> {
    let t = p.filter_type;
    let fields = [
        ("eq.freq", p.freq),
        ("eq.q", p.q),
        ("eq.gain", p.gain_db),
        ("eq.fp", p.gain_db),
        ("eq.qp", p.qp.unwrap_or(DEFAULT_QP)),
    ];
    let at = [p.channel, p.band];
    for (path, v) in fields {
        let derived_q = path == "eq.q" && dspi_proto::xover::meta(t.to_raw()).is_some();
        if derived_q || registry::eq_field_fits(path, p.band, t).is_err() {
            continue;
        }
        session.check(path, &at, &Value::Float(v))?;
    }
    Ok(())
}

/// Make a paste's writes on the device, in order.
///
/// The whole plan is checked first ([`validate_paste`]), so a bad value or
/// target is refused before anything is written. After that the writes stop
/// at the first that fails (the device or the transport), and the
/// [`PasteError`] carries the outcomes of those that went through. Each band
/// is one whole-band write (bypass flag included); the other writes go
/// through [`Session::write`], so they are read back like any other.
pub fn apply_paste(session: &mut Session, plan: &[PasteWrite]) -> Result<Vec<Outcome>, PasteError> {
    if let Err((_, source)) = validate_paste(session, plan) {
        return Err(PasteError {
            done: 0,
            total: plan.len(),
            outcomes: Vec::new(),
            source,
        });
    }
    let mut outcomes = Vec::with_capacity(plan.len());
    for w in plan {
        let result = match (w, param_write(w)) {
            (PasteWrite::Band(p), _) => session.write_band(p),
            (_, Some((path, index, value))) => session.write(path, &[index], value),
            // `param_write` answers every non-band write.
            (_, None) => continue,
        };
        match result {
            Ok(o) => outcomes.push(o),
            Err(source) => {
                return Err(PasteError {
                    done: outcomes.len(),
                    total: plan.len(),
                    outcomes,
                    source,
                });
            }
        }
    }
    Ok(outcomes)
}

/// A channel index as the wire carries it. Channels are fewer than 256 (the
/// device reports their count in a byte); anything larger saturates, which
/// the device then refuses as out of range rather than aliasing a channel.
fn channel_u8(channel: usize) -> u8 {
    u8::try_from(channel).unwrap_or(u8::MAX)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_s5 {
    use super::*;
    use crate::{probe, read_bulk};
    use dspi_transport::FakeDevice;

    /// The demo device, its state, and a session over it.
    pub(super) fn demo() -> (DeviceState, Session) {
        let mut d = FakeDevice::demo();
        let caps = probe(&mut d).unwrap();
        let bulk = read_bulk(&mut d).unwrap();
        let state = DeviceState::new(caps.clone(), bulk);
        (state, Session::new(Box::new(d), caps).unwrap())
    }

    #[test]
    fn wp_s5_a_copy_takes_what_the_channel_kind_carries() {
        let (s, _) = demo();
        let ni = usize::from(s.caps.num_inputs);
        let input = copy_channel(&s, 0);
        assert_eq!(input.bands.len(), usize::from(s.caps.max_bands));
        assert!(input.crossover.is_none());
        assert!(input.preamp_db.is_some());
        assert!(input.output.is_none());

        let output = copy_channel(&s, ni);
        assert_eq!(output.crossover.as_ref().unwrap().len(), CROSSOVER_BANDS);
        assert!(output.output.is_some());
        assert!(output.preamp_db.is_none());
        assert_eq!(
            output.copied_message(),
            format!("Copied {} parameters", output.source)
        );
    }

    /// Past the device's name table the state answers empty, and the default
    /// context falls back to the channel's token.
    #[test]
    fn wp_s5_the_default_name_falls_back_to_the_channel_token() {
        let (s, _) = demo();
        let ni = usize::from(s.caps.num_inputs);
        assert_eq!(
            ClipboardContext::channel_name(&s, 20),
            format!("out.{}", 21 - ni)
        );
    }

    #[test]
    fn wp_s5_a_plan_retargets_every_band_and_clears_the_rest() {
        let (s, _) = demo();
        let mut clip = copy_channel(&s, 0);
        clip.bands.truncate(2);
        let plan = paste_plan(&clip, &s, 1, Some(2));
        let bands: Vec<_> = plan
            .iter()
            .filter_map(|w| match w {
                PasteWrite::Band(p) => Some(*p),
                _ => None,
            })
            .collect();
        assert_eq!(bands.len(), 2 * usize::from(s.caps.max_bands));
        assert!(bands.iter().all(|p| p.channel == 1 || p.channel == 2));
        assert_eq!(bands[2], cleared_band(1, 2));
        let pre = plan
            .iter()
            .filter(|w| matches!(w, PasteWrite::Preamp { .. }))
            .count();
        assert_eq!(pre, 2, "the trim goes to both inputs");
    }

    #[test]
    fn wp_s5_a_paste_onto_an_output_lands_on_the_device() {
        let (s, mut session) = demo();
        let ni = usize::from(s.caps.num_inputs);
        let (src, dst) = (ni, ni + 1);
        let mut clip = copy_channel(&s, src);
        // Make the source distinctive, so the paste is visible.
        clip.bands[0] = EqParamPacket {
            channel: src as u8,
            band: 0,
            filter_type: FilterType::Peaking,
            bypass: false,
            freq: 1234.0,
            q: 2.0,
            gain_db: -4.5,
            qp: None,
        };
        clip.output = Some((-7.0, 1.5, true));

        let plan = paste_plan(&clip, &s, dst, None);
        let outcomes = apply_paste(&mut session, &plan).unwrap();
        assert_eq!(outcomes.len(), plan.len());

        let after = DeviceState::new(s.caps.clone(), session.snapshot().unwrap());
        let b = after.bands(dst as u8)[0];
        assert_eq!(b.filter_type, FilterType::Peaking);
        assert!((b.freq - 1234.0).abs() < 0.01);
        assert!((b.gain_db + 4.5).abs() < 0.01);
        let o = after.output(1);
        assert!((o.gain_db + 7.0).abs() < 0.01);
        assert!((o.delay_ms - 1.5).abs() < 0.01);
        assert!(o.mute);
        let mut xo = clip.crossover.clone().unwrap();
        for p in &mut xo {
            p.channel = dst as u8;
        }
        assert_eq!(after.xover_bands(dst as u8), xo);
    }

    /// A bad value anywhere in the plan is refused before the first write,
    /// so the destination is untouched.
    #[test]
    fn wp_s5_a_bad_value_is_refused_before_anything_is_written() {
        let (s, mut session) = demo();
        let mut clip = copy_channel(&s, 0);
        clip.bands[0].gain_db = 6.0;
        clip.bands[0].filter_type = FilterType::Peaking;
        let mut plan = paste_plan(&clip, &s, 1, None);
        // An out-of-range trim, last in the plan.
        plan.push(PasteWrite::Preamp { input: 1, db: 99.0 });
        let (i, _) = validate_paste(&session, &plan).unwrap_err();
        assert_eq!(i, plan.len() - 1);

        let e = apply_paste(&mut session, &plan).unwrap_err();
        assert_eq!((e.done, e.total), (0, plan.len()));
        assert!(e.outcomes.is_empty());
        assert!(matches!(e.source, WriteError::Value(_)), "{e:?}");
        assert!(e.to_string().starts_with("paste stopped after 0"));
        let after = DeviceState::new(s.caps.clone(), session.snapshot().unwrap());
        assert_eq!(after.bands(1), s.bands(1), "nothing was written");
    }

    /// A band past the device's channels is refused up front too.
    #[test]
    fn wp_s5_a_bad_target_is_refused_up_front() {
        let (s, session) = demo();
        let plan = [PasteWrite::Band(cleared_band(200, 0))];
        let (i, e) = validate_paste(&session, &plan).unwrap_err();
        assert_eq!(i, 0);
        assert!(matches!(e, WriteError::BadTarget(..)), "{e:?}");
        let ok = paste_plan(&copy_channel(&s, 0), &s, 1, Some(2));
        assert!(validate_paste(&session, &ok).is_ok());
    }

    /// A transport failure partway carries the outcomes so far.
    #[test]
    fn wp_s5_a_failed_write_carries_what_went_through() {
        use dspi_proto::generated::opcodes as op;
        use dspi_transport::MockTransport;
        use dspi_transport::mock::Reply;

        let (s, _) = demo();
        let clip = copy_channel(&s, 0);
        let plan = paste_plan(&clip, &s, 1, None);
        // Every band write fails on the wire; the plan itself is valid.
        let t = MockTransport::new().reply(op::REQ_SET_EQ_PARAM, Reply::Disconnect);
        let mut session = Session::new(Box::new(t), s.caps.clone()).unwrap();
        let e = apply_paste(&mut session, &plan).unwrap_err();
        assert_eq!(e.done, 0);
        assert_eq!(e.outcomes.len(), e.done);
        assert!(matches!(e.source, WriteError::Transport(_)), "{e:?}");
    }

    /// Bypass travels only where the firmware has it, and never on a flat
    /// band, as the TUI's `eq` lines do.
    #[test]
    fn wp_s5_bypass_is_gated_by_firmware_and_type() {
        let (mut s, _) = demo();
        assert!(supports_band_bypass(&s.caps));
        let mut clip = copy_channel(&s, 0);
        clip.bands[0].filter_type = FilterType::Peaking;
        clip.bands[0].bypass = true;
        clip.bands[1].filter_type = FilterType::Flat;
        clip.bands[1].bypass = true;
        let bypass = |plan: &[PasteWrite], i: usize| match &plan[i] {
            PasteWrite::Band(p) => p.bypass,
            w => panic!("{w:?}"),
        };
        let plan = paste_plan(&clip, &s, 1, None);
        assert!(bypass(&plan, 0));
        assert!(!bypass(&plan, 1), "a flat band is never bypassed");

        s.caps.firmware = "1.1.3".into();
        assert!(!supports_band_bypass(&s.caps));
        let plan = paste_plan(&clip, &s, 1, None);
        assert!(!bypass(&plan, 0), "older firmware has no bypass");
        s.caps.firmware = "1.1.4 beta 1".into();
        assert!(supports_band_bypass(&s.caps));
        s.caps.firmware = "garbage".into();
        assert!(!supports_band_bypass(&s.caps));
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_cg35 {
    use super::tests_wp_s5::demo;
    use super::*;

    /// A Linkwitz Transform on band 0 of the copy: f0 40 Hz, Q0 0.5, target
    /// `fp` Hz (the gain field) and Qp `qp`.
    fn linkwitz_clip(s: &DeviceState, fp: f32, qp: f32) -> ChannelClipboard {
        let mut clip = copy_channel(s, 0);
        clip.bands[0] = EqParamPacket {
            channel: 0,
            band: 0,
            filter_type: FilterType::LinkwitzTransform,
            bypass: false,
            freq: 40.0,
            q: 0.5,
            gain_db: fp,
            qp: Some(qp),
        };
        clip
    }

    /// Review c6v: a pasted Linkwitz band's gain field is fp in Hz, checked
    /// as `eq.fp` (10..14400) and its Qp as `eq.qp`, not as `eq.gain`'s
    /// +-24 dB, so a valid transform (fp 25 Hz) pastes and lands whole.
    #[test]
    fn wp_cg35_a_valid_linkwitz_band_pastes() {
        let (s, mut session) = demo();
        let plan = paste_plan(&linkwitz_clip(&s, 25.0, 0.6), &s, 1, None);
        assert!(validate_paste(&session, &plan).is_ok());
        apply_paste(&mut session, &plan).unwrap();
        let after = DeviceState::new(s.caps.clone(), session.snapshot().unwrap());
        let b = after.bands(1)[0];
        assert_eq!(b.filter_type, FilterType::LinkwitzTransform);
        assert!((b.freq - 40.0).abs() < 0.01 && (b.gain_db - 25.0).abs() < 0.01);
        assert!((b.qp.unwrap() - 0.6).abs() < 0.01, "{b:?}");
    }

    /// An out-of-range fp or Qp is refused before anything is written, as is
    /// a bell's gain past +-24 dB (unchanged).
    #[test]
    fn wp_cg35_an_invalid_linkwitz_band_is_refused() {
        let (s, session) = demo();
        for (fp, qp) in [(5.0, 0.6), (20_000.0, 0.6), (25.0, 30.0)] {
            let plan = paste_plan(&linkwitz_clip(&s, fp, qp), &s, 1, None);
            let (i, e) = validate_paste(&session, &plan).unwrap_err();
            assert_eq!(i, 0, "fp {fp} qp {qp}");
            assert!(matches!(e, WriteError::Value(_)), "{e:?}");
        }
        let mut clip = linkwitz_clip(&s, 25.0, 0.6);
        clip.bands[0].filter_type = FilterType::Peaking;
        let plan = paste_plan(&clip, &s, 1, None);
        assert!(matches!(
            validate_paste(&session, &plan),
            Err((0, WriteError::Value(_)))
        ));
    }
}
