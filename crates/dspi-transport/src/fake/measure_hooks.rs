//! What the measurement session's tests need of the simulated device (WP
//! P5SES, PLAN-P5 review RC11): set the meters (input and output peaks), play
//! what another host does (a foreign write on an output, a host volume change,
//! an input-format change, a preset load), and read the generator's staged
//! configuration, so a test can render the simulated room from exactly what
//! the session asked the generator to play.
//!
//! Every hook acts on the device's side, as a knob or another console would,
//! and queues the notification the firmware sends for it. None of them is
//! reachable from a client: they are [`FakeRemote`] methods, not requests.

use dspi_proto::generated::{notify as nfy, opcodes as op};
use dspi_proto::packets::SiggenConfig;

use super::{FakeRemote, O_GAIN, OUTPUT_LEN, UV_DB, section};

impl FakeRemote {
    /// Set the meter block (`GET_STATUS` sub-query 9): every input's peak at
    /// `input_dbfs` and each listed output's at its level, dBFS (peak, full
    /// scale 0; at or below -120 is silence). Outputs not listed read silence;
    /// no clip flags; `active_inputs` is `active_inputs`.
    pub fn measure_set_peaks(&self, input_dbfs: f32, outputs: &[(u8, f32)], active_inputs: u8) {
        let mut s = self.shared.lock();
        let n_in = usize::from(s.num_inputs());
        let n = usize::from(s.num_channels());
        let level = |db: f32| -> u16 {
            if !db.is_finite() || db <= -120.0 {
                return 0;
            }
            let lin = 10f32.powf(db.min(0.0) / 20.0);
            // In 0..=32767 by the clamp above.
            (lin * 32767.0).round().clamp(0.0, 32767.0) as u16
        };
        let mut block = Vec::with_capacity(s.status_len());
        for ch in 0..n {
            let db = if ch < n_in {
                input_dbfs
            } else {
                let o = ch.saturating_sub(n_in);
                outputs
                    .iter()
                    .find(|(out, _)| usize::from(*out) == o)
                    .map_or(-200.0, |(_, db)| *db)
            };
            block.extend_from_slice(&level(db).to_le_bytes());
        }
        // cpu0, cpu1, clip_flags, active_inputs
        block.extend_from_slice(&[10, 10, 0, 0, 0, 0, active_inputs]);
        s.status = block;
    }

    /// Another host changes output `output`'s gain to `gain_db` (a knob, a UART
    /// host): the image changes and `PARAM_CHANGED` goes out with source
    /// GPIO. False when the output is not the device's.
    pub fn measure_foreign_output_write(&self, output: u8, gain_db: f32) -> bool {
        let ok = {
            let mut s = self.shared.lock();
            match s.locate("outputs", OUTPUT_LEN, usize::from(output), O_GAIN) {
                Some(off) => s.write(off, &gain_db.to_le_bytes(), nfy::PARAM_SRC_GPIO as u8),
                None => false,
            }
        };
        self.shared.wake.notify_all();
        ok
    }

    /// The host's own USB volume moves the user volume to `db` (the OS volume
    /// slider through UAC1): the image changes and `PARAM_CHANGED` goes out
    /// with source UAC1.
    pub fn measure_host_volume(&self, db: f32) -> bool {
        let ok = {
            let mut s = self.shared.lock();
            match section("user_volume") {
                Some((base, _)) => {
                    let off = base.saturating_add(UV_DB.1);
                    s.write(off, &db.to_le_bytes(), nfy::PARAM_SRC_UAC1 as u8)
                }
                None => false,
            }
        };
        self.shared.wake.notify_all();
        ok
    }

    /// The input format changes (a new rate or channel count on the host's
    /// stream): `NOTIFY_EVT_INPUT_FORMAT` with `channels`.
    pub fn measure_input_format(&self, channels: u8) {
        self.shared
            .lock()
            .event(nfy::NOTIFY_EVT_INPUT_FORMAT, channels);
        self.shared.wake.notify_all();
    }

    /// Another console loads preset `slot` (0-based): the image is replaced,
    /// the generator stops (`SIGGEN_STOP_PRESET`), and `PRESET_LOADED` and
    /// `BULK_INVALIDATED` (source PRESET) go out, as `REQ_PRESET_LOAD` does.
    pub fn measure_preset_load_elsewhere(&self, slot: u8) {
        let _ = self.shared.lock().preset_load(u16::from(slot));
        self.shared.wake.notify_all();
    }

    /// The generator's configuration as the device holds it (staged or
    /// running), as `REQ_SIGGEN_GET_CONFIG` answers it; `None` on a device
    /// without the generator.
    pub fn measure_generator_config(&self) -> Option<SiggenConfig> {
        let mut s = self.shared.lock();
        let bytes = s.get_analysis(op::REQ_SIGGEN_GET_CONFIG, 0)??;
        drop(s);
        self.shared.wake.notify_all();
        SiggenConfig::decode(&bytes).ok()
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_p5ses {
    use crate::Transport;
    use crate::fake::FakeDevice;
    use dspi_proto::generated::opcodes as op;

    #[test]
    fn wp_p5ses_peaks_land_in_the_meter_block() {
        let mut d = FakeDevice::demo();
        let r = d.remote();
        r.measure_set_peaks(-40.0, &[(2, 0.0)], 2);
        let n = usize::from(d.remote().image()[2]);
        let b = d
            .control_in(op::REQ_GET_STATUS, 9, (n * 2 + 7) as u16)
            .unwrap();
        let n_in = usize::from(d.remote().image()[4]);
        let at = |ch: usize| u16::from_le_bytes([b[ch * 2], b[ch * 2 + 1]]);
        assert!((f32::from(at(0)) / 32767.0 - 0.01).abs() < 1e-3);
        assert_eq!(at(n_in + 2), 32767);
        assert_eq!(at(n_in + 1), 0);
        assert_eq!(b[n * 2 + 6], 2);
    }

    #[test]
    fn wp_p5ses_foreign_writes_and_events_are_queued() {
        let d = FakeDevice::demo();
        let r = d.remote();
        let mut src = d.notifications().unwrap();
        assert!(r.measure_foreign_output_write(1, -3.0));
        assert!(r.measure_host_volume(-30.0));
        r.measure_input_format(4);
        let got = crate::fake::tests_wp_t4::drain(&mut src);
        assert!(got.len() >= 3, "{got:?}");
        assert!(got.iter().any(|p| p[1] == 5 && p[4] == 4));
        assert!(!r.measure_foreign_output_write(200, 0.0));
    }

    #[test]
    fn wp_p5ses_the_generator_config_is_readable() {
        let mut d = FakeDevice::demo();
        let r = d.remote();
        let cfg = dspi_proto::packets::SiggenConfig {
            version: 1,
            signal_type: 4,
            channel_mask: 0b101,
            level_db: -40.0,
            duration_ms: 1000,
            repeat_count: 2,
            gap_ms: 300,
            p1: 20.0,
            p2: 20000.0,
            ..Default::default()
        };
        d.control_out(op::REQ_SIGGEN_SET_CONFIG, 0, &cfg.encode())
            .unwrap();
        let got = r.measure_generator_config().unwrap();
        assert_eq!(got.channel_mask, 0b101);
        assert_eq!(got.repeat_count, 2);
        r.measure_preset_load_elsewhere(0);
    }
}
