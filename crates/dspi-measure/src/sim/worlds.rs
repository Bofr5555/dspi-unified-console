//! Ready-made worlds and chains (WP P5SIM): the flat world, the three-way
//! stereo speaker in a room, the two-way near-field, headphones on a coupler.

use super::render::{ClockMap, validate_world};
use super::{
    Acoustics, Driver, MicClock, Microphone, OutputPath, Reflection, RoomMode, SimFilter, World,
};
use crate::types::{BandSpec, ChannelChain, Coupler, MeasureError, ProgrammeFeed};

/// A Linkwitz-Riley 4th-order low-pass's type byte (`dspi_proto::xover`).
const LR4_LP: u8 = 34;
/// A Linkwitz-Riley 4th-order high-pass's type byte.
const LR4_HP: u8 = 35;

fn xo(filter_type: u8, freq: f32) -> BandSpec {
    BandSpec {
        filter_type,
        bypass: false,
        freq,
        q: 0.707,
        gain: 0.0,
        qp: 0.0,
    }
}

/// Ten flat PEQ bands: the free bands a fit may use.
fn free_peq() -> Vec<BandSpec> {
    vec![xo(0, 1000.0); 10]
}

/// A chain that changes nothing (no bands, 0 dB, no delay).
pub fn flat_chain() -> ChannelChain {
    ChannelChain {
        peq: Vec::new(),
        crossover: Vec::new(),
        gain_db: 0.0,
        delay_ms: 0.0,
        feed: ProgrammeFeed {
            gain_db: 0.0,
            inverted: false,
        },
    }
}

/// `n` flat chains, one per output.
pub fn flat_chains(n: usize) -> Vec<ChannelChain> {
    vec![flat_chain(); n.min(super::MAX_SIM_OUTPUTS)]
}

fn chain(crossover: Vec<BandSpec>) -> ChannelChain {
    ChannelChain {
        peq: free_peq(),
        crossover,
        ..flat_chain()
    }
}

/// The chains of [`World::three_way_room`], as the `3way-rp2350` template
/// sets them: LR4 at 300 Hz and 3 kHz, ten free PEQ bands, outputs 0 to 2 the
/// left low, mid and high, 3 to 5 the right.
pub fn three_way_chains() -> Vec<ChannelChain> {
    let side = [
        chain(vec![xo(LR4_LP, 300.0)]),
        chain(vec![xo(LR4_HP, 300.0), xo(LR4_LP, 3000.0)]),
        chain(vec![xo(LR4_HP, 3000.0)]),
    ];
    side.iter().chain(side.iter()).cloned().collect()
}

/// The chains of [`World::two_way_near_field`]: LR4 at 2.2 kHz, ten free PEQ
/// bands, output 0 the low way, 1 the high way.
pub fn two_way_chains() -> Vec<ChannelChain> {
    vec![
        chain(vec![xo(LR4_LP, 2200.0)]),
        chain(vec![xo(LR4_HP, 2200.0)]),
    ]
}

fn peak(freq: f64, q: f64, gain_db: f64) -> Option<SimFilter> {
    Some(SimFilter::Peak { freq, q, gain_db })
}

fn path(name: &str, speaker: u8, driver: Driver, delay_ms: f64, gain_db: f64) -> OutputPath {
    OutputPath {
        name: name.to_string(),
        speaker,
        driver,
        delay_ms,
        gain_db,
    }
}

impl World {
    /// `outputs` (1 to 16) ideal outputs at `rate_hz`: flat drivers with no
    /// delay, no room, a flat microphone, no noise, the capture at the same
    /// rate with no clock offset and no latency, blocks of 128, no faults. An
    /// impulse through it comes back as the impulse.
    pub fn flat(outputs: usize, rate_hz: u32) -> World {
        let n = outputs.clamp(1, super::MAX_SIM_OUTPUTS);
        World {
            name: "flat".to_string(),
            outputs: (0..n)
                .map(|i| path(&format!("out {i}"), 0, Driver::flat(), 0.0, 0.0))
                .collect(),
            acoustics: Acoustics::Free,
            mic: Microphone::flat(),
            noise_dbfs: None,
            clock: MicClock {
                rate_hz,
                ppm: 0.0,
                latency_ms: 0.0,
                block_frames: 128,
                first_frame: 0,
            },
            faults: Vec::new(),
            seed: 0,
        }
    }

    /// A stereo three-way speaker in a room, for [`three_way_chains`]:
    /// outputs 0 to 2 the left woofer, midrange and tweeter, 3 to 5 the right.
    /// The woofer's acoustic centre sits 0.25 ms and the midrange's 0.12 ms
    /// behind the tweeter's; the right speaker is 0.15 ms farther than the
    /// left. Four modes (42, 68, 95, 130 Hz) and two reflections; a typical
    /// microphone 37.5 ppm fast with 50 ms of latency at 48 kHz, noise at -85
    /// dBFS.
    pub fn three_way_room(seed: u64) -> World {
        let woofer = Driver {
            sensitivity_db: -14.0,
            highpass_hz: 38.0,
            highpass_q: 0.71,
            rolloff_hz: 3500.0,
            rolloff_q: 0.7,
            breakup: peak(4500.0, 5.0, 6.0),
            offset_ms: 0.25,
            inverted: false,
        };
        let mid = Driver {
            sensitivity_db: -15.0,
            highpass_hz: 120.0,
            highpass_q: 0.8,
            rolloff_hz: 12_000.0,
            rolloff_q: 0.7,
            breakup: peak(9000.0, 6.0, 4.0),
            offset_ms: 0.12,
            inverted: false,
        };
        let tweeter = Driver {
            sensitivity_db: -17.0,
            highpass_hz: 900.0,
            highpass_q: 0.9,
            rolloff_hz: 0.0,
            rolloff_q: 0.7,
            breakup: peak(24_000.0, 4.0, 3.0),
            offset_ms: 0.0,
            inverted: false,
        };
        let mut outputs = Vec::new();
        for (side, name, extra) in [(0u8, "L", 0.0), (1u8, "R", 0.15)] {
            let d = 8.75 + extra;
            outputs.push(path(&format!("{name} low"), side, woofer.clone(), d, 0.0));
            outputs.push(path(&format!("{name} mid"), side, mid.clone(), d, 0.0));
            outputs.push(path(&format!("{name} high"), side, tweeter.clone(), d, 0.0));
        }
        World {
            name: "three_way_room".to_string(),
            outputs,
            acoustics: Acoustics::Room {
                modes: vec![
                    RoomMode {
                        freq: 42.0,
                        gain_db: 8.0,
                        t60_s: 0.6,
                    },
                    RoomMode {
                        freq: 68.0,
                        gain_db: -6.0,
                        t60_s: 0.4,
                    },
                    RoomMode {
                        freq: 95.0,
                        gain_db: 5.0,
                        t60_s: 0.4,
                    },
                    RoomMode {
                        freq: 130.0,
                        gain_db: 4.0,
                        t60_s: 0.3,
                    },
                ],
                reflections: vec![
                    Reflection {
                        delay_ms: 2.1,
                        gain: 0.45,
                    },
                    Reflection {
                        delay_ms: 5.3,
                        gain: 0.3,
                    },
                ],
            },
            mic: Microphone::typical(),
            noise_dbfs: Some(-85.0),
            clock: MicClock {
                rate_hz: 48_000,
                ppm: 37.5,
                latency_ms: 50.0,
                block_frames: 128,
                first_frame: 1_234_567,
            },
            faults: Vec::new(),
            seed,
        }
    }

    /// A two-way speaker measured near-field, for [`two_way_chains`]: output
    /// 0 the woofer, 1 the tweeter (0.08 ms ahead), no room, the microphone 20
    /// ppm slow with 30 ms of latency, noise at -90 dBFS.
    pub fn two_way_near_field(seed: u64) -> World {
        let woofer = Driver {
            sensitivity_db: -6.0,
            highpass_hz: 50.0,
            highpass_q: 0.7,
            rolloff_hz: 4000.0,
            rolloff_q: 0.7,
            breakup: peak(5500.0, 4.0, 5.0),
            offset_ms: 0.08,
            inverted: false,
        };
        let tweeter = Driver {
            sensitivity_db: -8.0,
            highpass_hz: 1100.0,
            highpass_q: 1.0,
            rolloff_hz: 0.0,
            rolloff_q: 0.7,
            breakup: None,
            offset_ms: 0.0,
            inverted: false,
        };
        World {
            name: "two_way_near_field".to_string(),
            outputs: vec![
                path("low", 0, woofer, 0.3, 0.0),
                path("high", 0, tweeter, 0.3, 0.0),
            ],
            acoustics: Acoustics::Free,
            mic: Microphone::typical(),
            noise_dbfs: Some(-90.0),
            clock: MicClock {
                rate_hz: 48_000,
                ppm: -20.0,
                latency_ms: 30.0,
                block_frames: 128,
                first_frame: 0,
            },
            faults: Vec::new(),
            seed,
        }
    }

    /// Headphones on a coupler: output 0 the left side, 1 the right, flat
    /// chains ([`flat_chains`]`(2)`). Five reseats with seal leaks of 0, 35,
    /// 12, 60 and 20 Hz (reseat 0 rendered; change `Acoustics::Coupler`'s
    /// `reseat`), the microphone 12 ppm fast with 20 ms of latency at 48 kHz,
    /// noise at -95 dBFS.
    pub fn headphones(coupler: Coupler, seed: u64) -> World {
        let side = |sens: f64, breakup: f64| Driver {
            sensitivity_db: sens,
            highpass_hz: 12.0,
            highpass_q: 0.6,
            rolloff_hz: 18_000.0,
            rolloff_q: 0.7,
            breakup: peak(breakup, 3.0, 3.0),
            offset_ms: 0.0,
            inverted: false,
        };
        let name = match coupler {
            Coupler::Iec60318_4 => "headphones_711",
            Coupler::FlatPlate => "headphones_flat_plate",
            Coupler::Hats => "headphones_hats",
        };
        World {
            name: name.to_string(),
            outputs: vec![
                path("left", 0, side(-20.0, 6000.0), 0.05, 0.0),
                path("right", 1, side(-20.5, 6300.0), 0.05, 0.0),
            ],
            acoustics: Acoustics::Coupler {
                coupler,
                leak_hz: vec![0.0, 35.0, 12.0, 60.0, 20.0],
                reseat: 0,
            },
            mic: Microphone::typical(),
            noise_dbfs: Some(-95.0),
            clock: MicClock {
                rate_hz: 48_000,
                ppm: 12.0,
                latency_ms: 20.0,
                block_frames: 128,
                first_frame: 0,
            },
            faults: Vec::new(),
            seed,
        }
    }

    /// The ground truth of the capture clock: the reported frame index
    /// (`MicClock::first_frame` included) at which device time `device_frame`
    /// is sampled, ignoring step slips but with the clock offset, the latency
    /// and any wander. An output's sound reaches the microphone
    /// [`World::acoustic_delay_ms`] later.
    ///
    /// Errors: [`MeasureError::Invalid`] for a world out of range, a device
    /// rate outside 8 to 192 kHz or a non-finite frame.
    pub fn capture_position(
        &self,
        device_frame: f64,
        device_rate_hz: u32,
    ) -> Result<f64, MeasureError> {
        validate_world(self)?;
        if !(8000..=192_000).contains(&device_rate_hz) || !device_frame.is_finite() {
            return Err(MeasureError::Invalid("bad position".into()));
        }
        let k = ClockMap::new(self, f64::from(device_rate_hz)).inverse(device_frame);
        let r = k + self.clock.first_frame as f64;
        if r.is_finite() {
            Ok(r)
        } else {
            Err(MeasureError::Invalid("bad position".into()))
        }
    }

    /// An output's acoustic delay, ms: its driver's offset plus its path's
    /// time of flight (the device chain's own delay comes on top). `None` for
    /// an output the world lacks.
    pub fn acoustic_delay_ms(&self, output: usize) -> Option<f64> {
        self.outputs
            .get(output)
            .map(|p| p.driver.offset_ms + p.delay_ms)
    }
}
