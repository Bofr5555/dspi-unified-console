//! The function test's simulated host audio (WP FBUS, FSTORY 3S.2 P11):
//! while a scenario runs (`DSPI_SIM_SCENARIO`), `dspi_get_host_audio` reads
//! the bus file the harness and the sound-source stand-ins write
//! (dspi-core's `sim_bus_host_view`) instead of the computer's own audio,
//! and a demo unit the bus names counts as an output of this computer.
//!
//! Feature `sim-bus`: on in dspi-capi's `sim-scenario` only, which no shipped
//! build turns on.

use std::sync::Arc;

use super::{HostAudio, HostAudioSource, HostOs, HostOutput, HostStream};

/// What the reading's `audio_system` ends with: the screens and the agent
/// see that it is simulated, as the simulated microphone's label says.
pub const SIM_MARK: &str = " (simulated)";

/// The bus while it has a file, else `real`.
pub struct SimBus {
    /// The computer's own reader (no bus file yet).
    pub real: Arc<dyn HostAudioSource>,
}

impl HostAudioSource for SimBus {
    fn read(&self) -> HostAudio {
        dspi_core::sim_bus_host_view().map_or_else(|| self.real.read(), from_sim)
    }
}

/// A scenario runs in this process: the bus may answer.
pub fn loaded() -> bool {
    std::env::var_os("DSPI_SIM_SCENARIO").is_some()
}

/// `audio` came from the bus.
pub fn is_sim(audio: &HostAudio) -> bool {
    audio.audio_system.ends_with(SIM_MARK)
}

/// The bus's view in the reader's terms.
pub fn from_sim(v: dspi_core::SimHostAudio) -> HostAudio {
    let os = match v.os.as_str() {
        "windows" => HostOs::Windows,
        "macos" => HostOs::MacOs,
        "linux" => HostOs::Linux,
        _ => HostOs::Other,
    };
    HostAudio {
        os,
        audio_system: format!("{}{SIM_MARK}", v.audio_system),
        unknown: None,
        outputs: v
            .outputs
            .into_iter()
            .map(|o| HostOutput {
                id: o.id,
                name: o.name,
                serial: o.serial,
                is_default: Some(o.is_default),
                channels: Some(o.channels),
                rate_hz: Some(o.rate_hz),
                running: Some(o.running),
                volume_pct: Some(o.volume_pct as f32),
                muted: Some(o.muted),
                hardware_volume: Some(o.hardware_volume),
                streams: o
                    .streams
                    .into_iter()
                    .map(|s| HostStream {
                        app: s.app,
                        title: s.title,
                        channels: Some(s.channels),
                        volume_pct: Some(s.volume_pct as f32),
                        muted: Some(s.muted),
                        playing: Some(s.playing),
                    })
                    .collect(),
            })
            .collect(),
        streams_known: true,
        streams_note: None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_fbus {
    use super::*;
    use crate::registry::DeviceRow;
    use dspi_core::{
        DeviceId, DeviceLiveness, DeviceTransport, OpenDevice, SimHostAudio, SimHostOutput,
        SimHostStream,
    };

    fn sim() -> HostAudio {
        from_sim(SimHostAudio {
            os: "linux".into(),
            audio_system: "PipeWire 1.6".into(),
            outputs: vec![
                SimHostOutput {
                    id: "fronts".into(),
                    name: "DSPi Fronts (USB)".into(),
                    serial: "Raspberry_Pi_DSPi_FBUS-FRONTS".into(),
                    is_default: true,
                    channels: 8,
                    rate_hz: 48_000,
                    running: true,
                    volume_pct: 50.0,
                    muted: false,
                    hardware_volume: true,
                    streams: vec![SimHostStream {
                        app: "Tidal".into(),
                        title: "Easy Jazz".into(),
                        channels: 2,
                        volume_pct: 100.0,
                        muted: false,
                        playing: true,
                    }],
                },
                SimHostOutput {
                    name: "Laptop speakers".into(),
                    channels: 2,
                    ..SimHostOutput::default()
                },
            ],
        })
    }

    fn row(n: u32, serial: &str) -> DeviceRow {
        DeviceRow {
            open: OpenDevice {
                id: DeviceId::demo(n),
                label: format!("Demo {n}"),
                transport: DeviceTransport::Demo,
                serial: serial.into(),
                state: DeviceLiveness::Live,
                extra: false,
            },
            firmware: "1.1.6".into(),
            active: false,
            hub: None,
        }
    }

    #[test]
    fn wp_fbus_the_bus_shows_a_demo_unit_as_this_computers_output() {
        let audio = sim();
        assert!(is_sim(&audio));
        assert_eq!(audio.audio_system, "PipeWire 1.6 (simulated)");
        let rows = [row(1, "FBUS-FRONTS"), row(2, "FBUS-OTHER")];
        let v = crate::tools::host::report(&audio, &rows);
        let d = &v["devices"][0];
        assert_eq!(d["output_device"], "yes", "{v}");
        assert_eq!(d["matched_by"], "serial");
        assert_eq!(d["is_default_output"], true);
        assert_eq!(d["os_volume_drives_user_volume"], true);
        assert_eq!(d["streams"][0]["app_name"], "Tidal");
        assert_eq!(v["devices"][1]["output_device"], "no");
        assert_eq!(v["other_outputs"][0]["output_name"], "Laptop speakers");
        assert_eq!(v["default_output"]["is_dspi"], true);
        // A real reading still says no for a demo unit
        let mut real = sim();
        real.audio_system = "PipeWire 1.6".into();
        let v = crate::tools::host::report(&real, &rows);
        assert_eq!(v["devices"][0]["output_device"], "no");
    }
}
