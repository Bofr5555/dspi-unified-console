//! -- WP UB12 -- The demo as a user left it (UXP B12, owner D12): what
//! [`DspiCore::open_demo`] opens, so that a first run without a board shows
//! what a DSPi does instead of a blank device.
//!
//! A stereo two-way on outputs 1-4 of the bundled capture: USB 1 plays the left
//! woofer and tweeter, USB 2 the right ones, crossed over with LR4 at 2.2 kHz;
//! a room-mode cut on both inputs, a cut on each woofer and tweeter, the
//! tweeters 3 dB down and 0.1 ms later; the four outputs named, and their roles
//! confirmed (a speaker layout kept for the demo's serial). Outputs 5-9 play
//! nothing. The master volume and everything else are the capture's.
//!
//! The image is the device's running settings and preset slot 1's
//! ([`FakeDevice::demo_running`]); `REQ_FACTORY_RESET` still gives the
//! capture. [`DspiCore::reset_demo`] puts the example back.

use dspi_proto::enums::FilterType;
use dspi_proto::generated::opcodes as op;
use dspi_proto::value::Value;
use dspi_proto::xover::{Family, Meta};
use dspi_transport::clock::Instant;
use dspi_transport::fake::EXT_FAKE_DEMO_RESET;
use dspi_transport::{FakeDevice, Transport, TransportKind};

use crate::journal::journal;
use crate::records::{DspState, FilterParams};
use crate::speaker_layout::{RoleSource, SpeakerLayout};
use crate::{DspiCore, DspiError, Result, state};

/// The crossover frequency, Hz.
pub(crate) const CROSSOVER_HZ: f32 = 2200.0;
/// The outputs' names, outputs 1-4 (0-based 0..4).
pub(crate) const OUTPUT_NAMES: [&str; 4] = [
    "Left woofer",
    "Right woofer",
    "Left tweeter",
    "Right tweeter",
];
/// The tweeters' level, dB.
const TWEETER_GAIN_DB: f32 = -3.0;
/// The tweeters' delay, ms.
const TWEETER_DELAY_MS: f32 = 0.1;

fn band(t: FilterType, freq: f32, q: f32, gain: f32) -> FilterParams {
    band_raw(t.to_raw(), freq, q, gain)
}

fn band_raw(filter_type: u8, freq: f32, q: f32, gain: f32) -> FilterParams {
    FilterParams {
        filter_type,
        bypass: false,
        freq,
        q,
        gain,
        qp: 0.707,
    }
}

fn lr4(high_pass: bool) -> FilterParams {
    let t = Meta {
        family: Family::LinkwitzRiley,
        order: 4,
        high_pass,
    }
    .to_type();
    band_raw(t, CROSSOVER_HZ, 0.707, 0.0)
}

/// Turn the capture's settings into the example (module doc). Outputs and
/// inputs the capture lacks are left out.
pub(crate) fn tune(s: &mut DspState) {
    let n_in = usize::from(s.num_input_channels);
    let n_out = usize::from(s.num_output_channels);
    // Routing: USB 1 to the left way, USB 2 to the right one, nothing else
    for (i, row) in s.matrix_routing.iter_mut().enumerate() {
        for (o, on) in row.iter_mut().enumerate() {
            *on = i < 2 && o < 4 && o < n_out && o % 2 == i;
        }
    }
    s.matrix_gain.iter_mut().flatten().for_each(|g| *g = 0.0);
    s.matrix_invert
        .iter_mut()
        .flatten()
        .for_each(|v| *v = false);
    for o in 0..4.min(n_out) {
        let tweeter = o >= 2;
        if let Some(v) = s.output_enabled.get_mut(o) {
            *v = true;
        }
        if let Some(v) = s.output_muted.get_mut(o) {
            *v = false;
        }
        if let Some(v) = s.output_gain_db.get_mut(o) {
            *v = if tweeter { TWEETER_GAIN_DB } else { 0.0 };
        }
        let delay = if tweeter { TWEETER_DELAY_MS } else { 0.0 };
        if let Some(v) = s.output_delay_ms.get_mut(o) {
            *v = delay;
        }
        let ch = n_in.saturating_add(o);
        if let Some(v) = s.channel_delays.get_mut(ch) {
            *v = delay;
        }
        if let Some(x) = s.xover.get_mut(ch).and_then(|x| x.first_mut()) {
            *x = lr4(tweeter);
        }
        // A woofer's box bump, a tweeter's peak
        if let Some(f) = s.filters.get_mut(ch).and_then(|f| f.first_mut()) {
            *f = if tweeter {
                band(FilterType::Peaking, 4200.0, 2.0, -1.5)
            } else {
                band(FilterType::Peaking, 180.0, 2.0, -2.5)
            };
        }
        if let (Some(name), Some(n)) = (s.channel_names.get_mut(ch), OUTPUT_NAMES.get(o)) {
            *name = (*n).to_owned();
        }
    }
    // The room's mode, on what both ways play
    for i in 0..2.min(n_in) {
        if let Some(f) = s.filters.get_mut(i).and_then(|f| f.first_mut()) {
            *f = band(FilterType::Peaking, 63.0, 4.0, -4.0);
        }
    }
}

/// The demo device opened as the example, or the plain capture when its
/// image cannot be decoded (never with the bundled one).
pub(crate) fn device() -> FakeDevice {
    let image = FakeDevice::demo().remote().image();
    let mut s = state::default_state();
    match state::decode_bulk(&image, &mut s) {
        Ok(()) => {
            tune(&mut s);
            FakeDevice::demo_running(state::encode_bulk(&s))
        }
        Err(e) => {
            log::warn!("the demo's example could not be built: {e}");
            FakeDevice::demo()
        }
    }
}

impl DspiCore {
    /// The example's roles for the open demo, as the user confirmed them:
    /// kept for its serial in place of any layout kept before (the demo's
    /// settings start afresh each time it opens, and so do its roles). Built
    /// from the filters as [`DspiCore::speaker_layout_propose`] suggests
    /// them, so the two cannot disagree. Not the user's call
    /// ([`DspiCore::speaker_layout_set`]): an agent may open a demo too.
    pub(crate) fn keep_demo_layout(&self) -> Result<()> {
        let proposal = self.speaker_layout_propose()?;
        let layout = SpeakerLayout {
            outputs: proposal
                .outputs
                .into_iter()
                .map(|mut o| {
                    o.source = RoleSource::User;
                    o
                })
                .collect(),
            confirmed_by_user: true,
            updated_ms: crate::events::now_ms(),
            notes: Vec::new(),
            ..proposal
        };
        self.speaker_layouts.save(&layout)
    }

    /// Open the demo device as the bundled capture left it, without the
    /// example (no filters, USB 1 and 2 on outputs 1 and 2, nothing named, no
    /// speaker layout: one kept for its serial is forgotten): for tests of a
    /// new board. Replaces any open device.
    pub fn open_demo_blank(&self) -> Result<()> {
        let demo = FakeDevice::demo();
        let serial = demo.descriptor().serial.clone();
        self.attach_probed(Box::new(demo), serial.clone(), 0)?;
        self.speaker_layouts.forget(&serial);
        Ok(())
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl DspiCore {
    /// "Reset demo": the open demo device runs the settings it opened with
    /// again (the example, for [`DspiCore::open_demo`]'s) as preset slot 1's
    /// and the active slot, the mirror is read again with nothing unsaved,
    /// and the example's speaker layout is kept again. The device stays
    /// open. The history is a preset load's ("Preset 1 loaded"): what came
    /// before it is gone with the state, as after a factory reset.
    ///
    /// Errors: [`DspiError::Disconnected`] with no device;
    /// [`DspiError::Unsupported`], with no traffic, on anything but a
    /// simulated device; the refusals of every write (read-only, an
    /// installer's lock, another actor) and the transfer errors of the read.
    pub fn reset_demo(&self) -> Result<()> {
        let began = Instant::now();
        let sent = {
            let mut guard = self.lock();
            let sent = guard.writable_conn().and_then(|mut w| {
                if w.conn().caps.kind != TransportKind::Fake {
                    return Err(DspiError::Unsupported);
                }
                // A whole image, as a preset load (`factory_reset_with`)
                w.conn().fresh_image = false;
                w.conn().echo.whole_image_sent(Instant::now());
                let sent = w
                    .conn()
                    .transport()
                    .control_out(op::REQ_EXTENSION, EXT_FAKE_DEMO_RESET, &[])
                    .map_err(DspiError::from);
                w.conn().echo.whole_image_done(Instant::now());
                sent
            });
            sent.map(|()| journal(&mut guard, "preset.load", &[0], Value::Trigger))
        };
        let result = sent.and_then(|()| {
            self.fetch_all()?;
            self.capture_baseline();
            Ok(())
        });
        self.whole_image_ended(began);
        result?;
        if is_example(&self.get_state()) {
            self.keep_demo_layout()?;
        }
        Ok(())
    }
}

/// The open demo runs the example (its outputs carry the example's names):
/// only then does a reset bring the example's roles back.
fn is_example(s: &DspState) -> bool {
    let n_in = usize::from(s.num_input_channels);
    OUTPUT_NAMES.iter().enumerate().all(|(o, n)| {
        s.channel_names
            .get(n_in.saturating_add(o))
            .is_some_and(|x| x == n)
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::needless_range_loop)]
mod tests_wp_ub12;
