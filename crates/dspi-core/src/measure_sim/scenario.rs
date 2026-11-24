//! The function test's simulated home theatre (WP FUNC1, `tools/functest`),
//! feature `sim-scenario`: off in every build but the function test's.
//!
//! A scenario file (`DSPI_SIM_SCENARIO`) names demo DSPis, each with its own
//! serial, the drivers on its outputs, one room and one microphone. While it
//! is loaded:
//!
//! - each demo opened is the scenario's next device, in order, blank (no
//!   filter, no crossover, the routing the file says);
//! - the simulated microphone stands in the scenario's room: it hears the
//!   open device's generator through that device's drivers, and what any
//!   other device of the scenario plays at the same time (one room, one
//!   microphone);
//! - the **damage oracle** watches every device from the device's side (its
//!   image and its generator), not the core's: a tweeter that gets signal
//!   below its safe band, a driver over its cap, clipping. It writes
//!   `oracle.jsonl` to `DSPI_SIM_OUT`;
//! - `truth.json` there holds, after every change, what each output and each
//!   input channel sounds like at the microphone, computed (not measured)
//!   from the devices' images.
//!
//! Demo connections only, as the rest of `measure_sim`.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use dspi_measure::sim::{
    Acoustics, Driver, MicClock, Microphone, OutputPath, Reflection, RoomMode, SimFilter, World,
    filters_response, section_response,
};
use dspi_measure::types::{BandSpec, ChannelChain, SectionModel};
use dspi_transport::clock::Instant;
use dspi_transport::{FakeDevice, FakeRemote};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::measure_doc::CoreSectionModel;
use crate::records::DspState;

// -- WP FBUS -- P5 (the playback chain) and P11 (the host audio bus)
pub(crate) mod bus;
mod playback;
pub(crate) use playback::signal_chains;

/// The scenario file.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Scenario {
    pub(crate) name: String,
    pub(crate) seed: u64,
    /// The room's noise floor at the microphone, dBFS RMS.
    pub(crate) noise_dbfs: f64,
    /// The microphone's clock offset, ppm, and the mean latency of a start.
    pub(crate) mic_ppm: f64,
    pub(crate) latency_ms: f64,
    /// Every start's latency differs by up to this much (USB, the host).
    pub(crate) latency_jitter_ms: f64,
    pub(crate) modes: Vec<RoomMode>,
    pub(crate) reflections: Vec<Reflection>,
    pub(crate) devices: Vec<DeviceSpec>,
    // -- WP ADETS --
    /// The drivers are headphones on a coupler (the K340): an unknown
    /// output's envelope is the headphones' (AUTODES 1.2).
    #[serde(default)]
    pub(crate) headphones: bool,
}

/// One device of the scenario.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeviceSpec {
    pub(crate) serial: String,
    /// What the user calls it (the device list's label).
    pub(crate) name: String,
    pub(crate) outputs: Vec<OutputSpec>,
    /// `[input, output]`, 0-based: the routes it starts with, 0 dB.
    #[serde(default)]
    pub(crate) routes: Vec<(u8, u8)>,
    #[serde(default)]
    pub(crate) input_names: Vec<String>,
    /// The user volume it starts at, dB.
    pub(crate) user_volume_db: f32,
    // -- WP ADETS --
    /// Outputs, 0-based, that drive nothing and are switched on all the
    /// same (AUTODES 7.3: the detection must find them).
    #[serde(default)]
    pub(crate) enabled_unwired: Vec<u8>,
}

/// One driver and its amplifier.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OutputSpec {
    /// 0-based.
    pub(crate) output: u8,
    pub(crate) name: String,
    /// For the harness (the user's speaker layout); the sim reads `safe`.
    #[allow(dead_code)]
    pub(crate) role: String,
    #[allow(dead_code)]
    pub(crate) speaker: String,
    #[allow(dead_code)]
    pub(crate) group: u8,
    pub(crate) driver: Driver,
    /// Its irregularities and the modes its place excites.
    #[serde(default)]
    pub(crate) filters: Vec<SimFilter>,
    pub(crate) delay_ms: f64,
    pub(crate) gain_db: f64,
    pub(crate) safe: Safe,
}

/// What damages a driver.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Safe {
    /// Below this it may get next to nothing, Hz (0: no limit).
    pub(crate) below_hz: f64,
    /// Next to nothing for a test signal: the peak it may have below
    /// `below_hz`, dBFS.
    pub(crate) below_dbfs: f64,
    /// The peak it may have anywhere, dBFS, from a test signal.
    pub(crate) cap_dbfs: f64,
}

/// What the generator of another device put into the air.
struct Air {
    serial: String,
    from: Instant,
    rate: u32,
    samples: Arc<Vec<f32>>,
    /// Seconds after `from` at which it was stopped.
    end: Option<f64>,
}

/// One device of the running scenario.
struct Live {
    spec: usize,
    remote: FakeRemote,
    image: Vec<u8>,
    playing: Option<(u32, Vec<u8>)>,
}

/// The scenario, running.
pub(crate) struct Room {
    pub(crate) spec: Scenario,
    out: Option<PathBuf>,
    started: Instant,
    live: Mutex<Vec<Live>>,
    air: Mutex<Vec<Air>>,
    opened: Mutex<usize>,
    // -- WP ADETS --
    known: Mutex<std::collections::HashMap<String, Known>>,
}

// -- WP ADETS --
/// What the core knows of a device's outputs (AUTODES 7.2): its speaker
/// layout, its limits profile's floors, the detection's segment now.
#[derive(Debug, Default)]
struct Known {
    layout: Option<crate::speaker_layout::SpeakerLayout>,
    /// The limits profile's `highpass_min_hz`, by output (0-based).
    floors: Vec<(u8, f32)>,
    /// The detection's segment per output: (stage, the floor it plays to).
    probe: Vec<(u8, String, f32)>,
}

// -- WP ADETS --
/// The limits a test signal on one output is held to beyond its driver's
/// `safe`, from what the core knows of it.
#[derive(Debug, Clone)]
struct Envelope {
    /// `unknown` (no trusted role: the probe's envelope), `detected` (a
    /// detected tweeter: nothing below its floor).
    knowledge: &'static str,
    safe: Safe,
}

static ROOM: OnceLock<Option<Arc<Room>>> = OnceLock::new();

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The scenario, when the environment names one that can be read.
pub(crate) fn room() -> Option<Arc<Room>> {
    ROOM.get_or_init(|| {
        let path = std::env::var_os("DSPI_SIM_SCENARIO")?;
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                log::error!("the scenario {}: {e}", path.to_string_lossy());
                return None;
            }
        };
        match serde_json::from_str::<Scenario>(&text) {
            Ok(spec) => {
                log::warn!("simulated scenario \"{}\"", spec.name);
                let room = Arc::new(Room {
                    spec,
                    out: std::env::var_os("DSPI_SIM_OUT").map(PathBuf::from),
                    started: Instant::now(),
                    live: Mutex::default(),
                    air: Mutex::default(),
                    opened: Mutex::new(0),
                    known: Mutex::default(),
                });
                let watch = Arc::downgrade(&room);
                let _ = std::thread::Builder::new()
                    .name("sim-oracle".into())
                    .spawn(move || {
                        while let Some(r) = watch.upgrade() {
                            r.watch();
                            bus::tick(&r); // -- WP FBUS --
                            drop(r);
                            std::thread::sleep(Duration::from_millis(4));
                        }
                    });
                Some(room)
            }
            Err(e) => {
                log::error!("the scenario {}: {e}", path.to_string_lossy());
                None
            }
        }
    })
    .clone()
}

/// The next demo device of the scenario; `None` without one, or when its
/// devices are all open.
pub(crate) fn next_demo() -> Option<FakeDevice> {
    let room = room()?;
    let i = {
        let mut n = lock(&room.opened);
        let i = *n;
        *n = n.saturating_add(1);
        i
    };
    let spec = room.spec.devices.get(i)?;
    let dev = FakeDevice::demo().with_serial(&spec.serial);
    let remote = dev.remote();
    let mut s = crate::state::default_state();
    if crate::state::decode_bulk(&remote.image(), &mut s).is_ok() {
        blank(&mut s, spec);
        remote.load_image(crate::state::encode_bulk(&s), 0);
    }
    lock(&room.live).push(Live {
        spec: i,
        remote,
        image: Vec::new(),
        playing: None,
    });
    Some(dev)
}

/// What the user calls the device with this serial.
pub(crate) fn name_of(serial: &str) -> Option<String> {
    let room = room()?;
    let d = room.spec.devices.iter().find(|d| d.serial == serial)?;
    Some(d.name.clone())
}

// -- WP ADETS -- what the core knows, told to the oracle (AUTODES 7.2)

/// The probe's envelope for an output with no trusted role, the owner's
/// numbers (AUTODES 1.2, owner Q2): the oracle's own copy, not the core's.
const UNKNOWN_CEILING_DBFS: f64 = -24.0;
const UNKNOWN_CEILING_HEADPHONES_DBFS: f64 = -36.0;
const UNKNOWN_BELOW_DBFS: f64 = -50.0;
const UNKNOWN_BELOW_HEADPHONES_DBFS: f64 = -60.0;
/// The tweeter floor every unknown output has at least, Hz.
const UNKNOWN_FLOOR_HZ: f64 = 1000.0;

/// A speaker layout was kept for its serial (any core of this process).
pub(crate) fn layout_saved(layout: &crate::speaker_layout::SpeakerLayout) {
    let Some(room) = room() else { return };
    let Some(dev) = room.device(&layout.serial) else {
        return;
    };
    let name = dev.name.clone();
    {
        let mut known = lock(&room.known);
        let k = known.entry(layout.serial.clone()).or_default();
        k.layout = Some(layout.clone());
        // A segment's floor stands while its output is being probed: not once it is
        // classified, nor after a stop
        let probing = |o: u8| {
            layout.detected.as_ref().is_some_and(|d| {
                !d.stopped
                    && d.outputs.iter().any(|x| {
                        x.output == o && x.class == crate::speaker_layout::DetectClass::Unknown
                    })
            })
        };
        k.probe.retain(|(o, _, _)| probing(*o));
    }
    let outputs: Vec<Value> = layout
        .outputs
        .iter()
        .map(|o| {
            let d = layout.detected_output(o.output);
            json!({"output": o.output, "role": o.role, "source": o.source,
                "class": d.map(|d| d.class), "confidence": d.map(|d| d.confidence),
                "speaker": o.speaker}) // -- WP ROLES -- the side the words gave it
        })
        .collect();
    let detected: Vec<Value> = layout
        .detected
        .iter()
        .flat_map(|d| &d.outputs)
        .map(|d| {
            json!({"output": d.output, "class": d.class, "confidence": d.confidence,
            "role": d.role, "floorHz": d.floor_hz, "heard": d.heard, "fLowHz": d.f_low_hz,
            "lowestHz": d.lowest_hz, "stoppedAtHz": d.stopped_at_hz, "topFallDb": d.top_fall_db})
        })
        .collect();
    room.note(
        json!({"kind": "layout", "device": name, "serial": layout.serial,
        "state": layout.state(), "outputs": outputs, "detected": detected,
        "summary": layout.detected.as_ref().map(|d| d.summary.clone()),
        // -- WP ROLES -- the person's words an agent set the roles from, and the checks
        "stated": layout.stated.as_ref().map(|st| json!({"by": st.by, "words": st.words,
            "whatChanged": st.what_changed,
            "checks": st.checks.iter().map(|c| json!({"output": c.output, "role": c.role,
                "verdict": c.verdict, "topFallDb": c.top_fall_db})).collect::<Vec<_>>()}))}),
    );
}

/// The limits profile of `serial` was set (`Some`) or removed.
pub(crate) fn profile_saved(serial: &str, profile: Option<&dspi_session::limits::LimitsProfile>) {
    let Some(room) = room() else { return };
    let floors = profile
        .map(|p| {
            p.outputs
                .iter()
                .filter_map(|o| Some((o.output, o.highpass_min_hz?)))
                .collect()
        })
        .unwrap_or_default();
    lock(&room.known)
        .entry(serial.to_owned())
        .or_default()
        .floors = floors;
}

/// The detection starts a session on `output` of `serial`: its stage and the
/// floor the core holds it to (the free descent's is 20 Hz). Logged as a
/// `probe` line (AUTODES 7.2.6).
#[allow(clippy::too_many_arguments)]
pub(crate) fn probe_segment(
    serial: &str,
    output: u8,
    stage: &str,
    floor_hz: f32,
    f1: f32,
    f2: f32,
    level_dbfs: f32,
) {
    let Some(room) = room() else { return };
    let name = room
        .device(serial)
        .map(|d| d.name.clone())
        .unwrap_or_default();
    {
        let mut known = lock(&room.known);
        let k = known.entry(serial.to_owned()).or_default();
        k.probe.retain(|(o, _, _)| *o != output);
        k.probe.push((output, stage.to_owned(), floor_hz));
    }
    room.note(
        json!({"kind": "probe", "device": name, "serial": serial, "output": output,
        "stage": stage, "floorHz": floor_hz, "f1": f1, "f2": f2, "levelDbfs": level_dbfs}),
    );
}

/// A new DSPi: nothing filtered, crossed over, delayed or trimmed, no effect.
fn blank(s: &mut DspState, spec: &DeviceSpec) {
    for b in s.filters.iter_mut().chain(s.xover.iter_mut()).flatten() {
        b.filter_type = 0;
        b.bypass = false;
        b.gain = 0.0;
        b.freq = 1000.0;
        b.q = 0.707;
    }
    s.preamp_db = 0.0;
    s.bypass = false;
    s.input_preamp_db.iter_mut().for_each(|v| *v = 0.0);
    s.user_volume_db = spec.user_volume_db;
    s.master_volume_db = 0.0;
    s.user_mute = false;
    s.loudness_enabled = false;
    s.crossfeed_enabled = false;
    s.leveller_enabled = false;
    s.psybass_enabled = false;
    s.channel_delays.iter_mut().for_each(|v| *v = 0.0);
    for r in &mut s.matrix_routing {
        r.iter_mut().for_each(|v| *v = false);
    }
    for r in &mut s.matrix_gain {
        r.iter_mut().for_each(|v| *v = 0.0);
    }
    for r in &mut s.matrix_invert {
        r.iter_mut().for_each(|v| *v = false);
    }
    for &(i, o) in &spec.routes {
        if let Some(v) = s
            .matrix_routing
            .get_mut(usize::from(i))
            .and_then(|r| r.get_mut(usize::from(o)))
        {
            *v = true;
        }
    }
    s.output_gain_db.iter_mut().for_each(|v| *v = 0.0);
    s.output_delay_ms.iter_mut().for_each(|v| *v = 0.0);
    s.output_muted.iter_mut().for_each(|v| *v = false);
    for (o, on) in s.output_enabled.iter_mut().enumerate() {
        *on = spec.outputs.iter().any(|x| usize::from(x.output) == o)
            || spec.enabled_unwired.iter().any(|x| usize::from(*x) == o); // -- WP ADETS --
    }
    s.limiter_enabled.iter_mut().for_each(|v| *v = false);
    let n_in = usize::from(s.num_input_channels);
    for (i, n) in s.channel_names.iter_mut().enumerate() {
        *n = match i.checked_sub(n_in) {
            None => spec
                .input_names
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("In {}", i.saturating_add(1))),
            Some(o) => format!("Out {}", o.saturating_add(1)),
        };
    }
}

impl DeviceSpec {
    fn output(&self, o: usize) -> Option<&OutputSpec> {
        self.outputs.iter().find(|x| usize::from(x.output) == o)
    }
}

impl Room {
    pub(crate) fn device(&self, serial: &str) -> Option<&DeviceSpec> {
        self.spec.devices.iter().find(|d| d.serial == serial)
    }

    /// The world the microphone stands in for `device`, and each output's own
    /// filters (run over the generator's samples).
    pub(crate) fn world(
        &self,
        device: &DeviceSpec,
        outputs: usize,
        rate_hz: u32,
    ) -> (World, Vec<Vec<SimFilter>>) {
        let n = outputs.clamp(1, dspi_measure::sim::MAX_SIM_OUTPUTS);
        let paths = (0..n)
            .map(|o| match device.output(o) {
                Some(x) => OutputPath {
                    name: x.name.clone(),
                    speaker: x.group,
                    driver: x.driver.clone(),
                    delay_ms: x.delay_ms,
                    gain_db: x.gain_db,
                },
                // Nothing is wired to it.
                None => OutputPath {
                    name: format!("out {o}"),
                    speaker: 0,
                    driver: Driver {
                        sensitivity_db: -200.0,
                        ..Driver::flat()
                    },
                    delay_ms: 0.0,
                    gain_db: 0.0,
                },
            })
            .collect();
        let filters = (0..n)
            .map(|o| {
                device
                    .output(o)
                    .map(|x| x.filters.clone())
                    .unwrap_or_default()
            })
            .collect();
        let w = World {
            name: self.spec.name.clone(),
            outputs: paths,
            acoustics: Acoustics::Room {
                modes: self.spec.modes.clone(),
                reflections: self.spec.reflections.clone(),
            },
            mic: Microphone::flat(),
            noise_dbfs: Some(self.spec.noise_dbfs),
            clock: MicClock {
                rate_hz,
                ppm: self.spec.mic_ppm,
                latency_ms: self.spec.latency_ms,
                block_frames: (rate_hz / 50).max(1),
                first_frame: 0,
            },
            faults: Vec::new(),
            seed: self.spec.seed,
            rumble: None,
        };
        (w, filters)
    }

    /// The latency of the `render`th start on `serial`, ms.
    pub(crate) fn latency_ms(&self, serial: &str, render: u64) -> f64 {
        let mut h = self.spec.seed ^ render.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for b in serial.bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
        }
        h ^= h >> 29;
        let u = (h % 10_007) as f64 / 10_006.0;
        self.spec.latency_ms + self.spec.latency_jitter_ms * (2.0 * u - 1.0)
    }

    /// `serial`'s generator put `samples` into the air from `from`.
    pub(crate) fn plays(&self, serial: &str, from: Instant, rate: u32, samples: Vec<f32>) {
        let mut air = lock(&self.air);
        // A minute is longer than any run's tail
        air.retain(|a| a.serial != serial && a.from.elapsed() < Duration::from_secs(180));
        air.push(Air {
            serial: serial.to_owned(),
            from,
            rate,
            samples: Arc::new(samples),
            end: None,
        });
    }

    /// `serial`'s generator was stopped now.
    pub(crate) fn stops(&self, serial: &str) {
        for a in lock(&self.air).iter_mut().filter(|a| a.serial == serial) {
            // What is in the air still arrives: the latency and the room's ring
            let now = a.from.elapsed().as_secs_f64() + self.spec.latency_ms / 1000.0 + 1.0;
            a.end = Some(a.end.map_or(now, |e| e.min(now)));
        }
    }

    /// Add what every device but `serial` plays to a block of the microphone's
    /// that starts at `at`.
    pub(crate) fn mix(&self, serial: &str, at: Instant, rate: u32, block: &mut [f32]) {
        for a in lock(&self.air).iter().filter(|a| a.serial != serial) {
            if a.rate != rate {
                continue;
            }
            let before = a.from.saturating_duration_since(at).as_secs_f64();
            let after = at.saturating_duration_since(a.from).as_secs_f64();
            let start = ((after - before) * f64::from(rate)).round() as i64;
            let stop = a
                .end
                .map_or(i64::MAX, |e| (e * f64::from(rate)).round() as i64);
            for (i, v) in block.iter_mut().enumerate() {
                let k = start.saturating_add(i as i64);
                if k < 0 || k >= stop {
                    continue;
                }
                if let Some(x) = usize::try_from(k).ok().and_then(|k| a.samples.get(k)) {
                    *v = (*v + x).clamp(-1.0, 1.0);
                }
            }
        }
    }

    // ---------------------------------------------------------- the oracle

    // -- WP ADETS --
    /// What output `o` of `serial` is held to beyond its driver's `safe`:
    /// an output with no trusted role (no confirmed role, no finished
    /// detection) the probe's envelope (AUTODES 7.2.1; the free descent's
    /// floor is 20 Hz, the shaped one's showed it is no tweeter); a detected
    /// tweeter nothing below its floor above the shaped probe's level
    /// (7.2.3); anything else nothing more. `wired`: an output with a driver
    /// (one with none is always held to the envelope, 7.2.2).
    fn envelope(&self, serial: &str, o: u8, wired: bool) -> Option<Envelope> {
        use crate::measure::MeasureRole;
        use crate::speaker_layout::{DetectClass, LayoutState, RoleSource};
        let known = lock(&self.known);
        let k = known.get(serial);
        let (below, ceiling) = if self.spec.headphones {
            (
                UNKNOWN_BELOW_HEADPHONES_DBFS,
                UNKNOWN_CEILING_HEADPHONES_DBFS,
            )
        } else {
            (UNKNOWN_BELOW_DBFS, UNKNOWN_CEILING_DBFS)
        };
        let floor = k
            .and_then(|k| k.floors.iter().find(|(x, _)| *x == o))
            .map_or(UNKNOWN_FLOOR_HZ, |(_, f)| {
                UNKNOWN_FLOOR_HZ.max(f64::from(*f))
            });
        let layout = k.and_then(|k| k.layout.as_ref());
        // The role the safeties trust, and whether the detection found it
        let trusted = layout.and_then(|l| {
            let row = l.output(o)?;
            match l.state() {
                LayoutState::Confirmed => Some((row.role, false)),
                LayoutState::AutoDetected => match row.source {
                    RoleSource::User => Some((row.role, false)),
                    _ => match l.detected_output(o).map(|d| d.class) {
                        Some(DetectClass::Unknown | DetectClass::Nothing | DetectClass::Off)
                        | None => None,
                        Some(_) => Some((row.role, true)),
                    },
                },
                // -- WP ROLES -- a stated row is trusted as stated (the
                // driver's own `safe` judges a wrong statement); an output
                // the statement leaves out is unknown
                LayoutState::Stated => Some((row.role, false)),
                LayoutState::Unconfirmed => None,
            }
        });
        match trusted.filter(|_| wired) {
            None => {
                let free = k
                    .and_then(|k| k.probe.iter().find(|(x, _, _)| *x == o))
                    .is_some_and(|(_, stage, f)| stage == "free" && *f < 1000.0);
                Some(Envelope {
                    knowledge: "unknown",
                    safe: Safe {
                        below_hz: if free && wired { 20.0 } else { floor },
                        below_dbfs: below,
                        cap_dbfs: ceiling,
                    },
                })
            }
            Some((MeasureRole::High, true)) => Some(Envelope {
                knowledge: "detected",
                safe: Safe {
                    below_hz: floor,
                    below_dbfs: below,
                    cap_dbfs: 0.0,
                },
            }),
            Some(_) => None,
        }
    }

    fn note(&self, line: Value) {
        let Some(dir) = &self.out else { return };
        let mut line = line;
        if let Some(o) = line.as_object_mut() {
            let t = self.started.elapsed().as_secs_f64();
            o.insert("t".into(), json!((t * 1000.0).round() / 1000.0));
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("oracle.jsonl"));
        if let Ok(mut f) = file {
            let _ = writeln!(f, "{line}");
        }
    }

    /// A generator start seen by the core's own feed (none is missed between
    /// two looks of the watch).
    pub(crate) fn started(&self, serial: &str, cfg: &crate::records::SiggenConfig) {
        let image = lock(&self.live)
            .iter()
            .find(|l| {
                self.spec
                    .devices
                    .get(l.spec)
                    .is_some_and(|d| d.serial == serial)
            })
            .map(|l| l.remote.image());
        if let Some((s, dev)) = image.and_then(|i| decoded(&i)).zip(self.device(serial)) {
            self.judge_signal(
                dev,
                &s,
                cfg.signal_type,
                cfg.channel_mask,
                cfg.level_db,
                cfg.p.first().copied().unwrap_or(20.0),
                cfg.p.get(1).copied().unwrap_or(20_000.0),
                cfg.duration_ms,
                cfg.flags,
                "start",
            );
        }
    }

    /// One look at every device.
    fn watch(&self) {
        let mut changed = false;
        let mut live = lock(&self.live);
        for l in live.iter_mut() {
            let Some(dev) = self.spec.devices.get(l.spec) else {
                continue;
            };
            let image = l.remote.image();
            let fresh = image != l.image;
            if fresh {
                l.image.clone_from(&image);
                changed = true;
                if let Some(s) = decoded(&image) {
                    self.judge_settings(dev, &s);
                }
            }
            // The generator, whoever started it
            match (
                l.remote.measure_generator_running(),
                l.remote.measure_generator_config(),
            ) {
                (Some(ms), Some(cfg)) => {
                    let bytes = cfg.encode().to_vec();
                    let same = l
                        .playing
                        .as_ref()
                        .is_some_and(|(at, b)| *at <= ms && *b == bytes);
                    if !same && let Some(s) = decoded(&image) {
                        self.judge_signal(
                            dev,
                            &s,
                            cfg.signal_type,
                            cfg.channel_mask,
                            cfg.level_db,
                            cfg.p1,
                            cfg.p2,
                            cfg.duration_ms,
                            cfg.flags,
                            "seen",
                        );
                    }
                    l.playing = Some((ms, bytes));
                }
                _ => {
                    // Over, or stopped by whoever: its sound ends with it
                    if l.playing.take().is_some() {
                        self.stops(&dev.serial);
                    }
                }
            }
        }
        if changed {
            self.write_truth(&live);
        }
    }

    /// A test signal against what it reaches: the peak each driver gets below
    /// its safe band and anywhere, dBFS at the amplifier and (-- WP INTH --)
    /// as the generator makes it; [`signal_why`] judges both.
    #[allow(clippy::too_many_arguments)]
    fn judge_signal(
        &self,
        dev: &DeviceSpec,
        s: &DspState,
        signal: u8,
        mask: u16,
        level_db: f32,
        f1: f32,
        f2: f32,
        duration_ms: u32,
        flags: u8,
        how: &str,
    ) {
        // -- WP FBUS -- (P5) a tone at p1, a sweep, anything else full band
        let (f1, f2) = playback::signal_band(signal, f1, f2);
        for o in 0..usize::from(s.num_output_channels) {
            if u32::try_from(o)
                .ok()
                .and_then(|b| 1u16.checked_shl(b))
                .is_none_or(|b| mask & b == 0)
            {
                continue;
            }
            // -- WP ADETS -- what the core knows of it
            let envelope = self.envelope(&dev.serial, o as u8, dev.output(o).is_some());
            let Some(x) = dev.output(o) else {
                // -- WP ADETS -- the detection's probe finds what is not wired
                // (AUTODES 7.2.2): held to the unknown output's envelope
                let chain = crate::measure::chain_of(s, o as u8);
                let why = envelope.as_ref().map_or_else(Vec::new, |e| {
                    let band = (f1, f2);
                    let g = playback::signal_peak(s, o, &chain, level_db, band, e.safe.below_hz);
                    let g = playback::SignalPeak {
                        peak_db: g.peak_db + g.limiter_db,
                        below_db: g.below_db + g.limiter_db,
                        limiter_db: 0.0,
                        ..g
                    };
                    let amp = playback::signal_chain(s, o, flags)
                        .map(|c| playback::signal_peak(s, o, &c, level_db, band, e.safe.below_hz));
                    envelope_why(e, amp.as_ref(), &g)
                });
                if why.is_empty() {
                    self.note(json!({"kind": "info",
                        "what": "a probe within the unknown output's envelope on an output that drives nothing",
                        "device": dev.name, "output": o, "how": how, "signal": signal,
                        "levelDb": level_db, "f1": f1, "f2": f2}));
                } else {
                    self.note(json!({"kind": "VIOLATION", "how": how, "device": dev.name,
                        "serial": dev.serial, "output": o, "driver": "nothing wired",
                        "what": "a test signal on an output with no driver", "signal": signal,
                        "levelDb": level_db, "f1": f1, "f2": f2, "why": why}));
                }
                continue;
            };
            let chain = crate::measure::chain_of(s, o as u8);
            // -- WP FBUS -- (P5) what reaches the amplifier: the chain, then
            // trim, volume, mutes, loudness and the limiter, as on the device
            let generator =
                playback::signal_peak(s, o, &chain, level_db, (f1, f2), x.safe.below_hz);
            // (no limiter before the volume)
            let generator = playback::SignalPeak {
                peak_db: generator.peak_db + generator.limiter_db,
                below_db: generator.below_db + generator.limiter_db,
                limiter_db: 0.0,
                ..generator
            };
            // -- WP INTH -- the generator's level is judged as well, always
            let played = playback::signal_chain(s, o, flags).map(|played| {
                playback::signal_peak(s, o, &played, level_db, (f1, f2), x.safe.below_hz)
            });
            let mut why = signal_why(&x.safe, played.as_ref(), &generator);
            // -- WP ADETS -- an unknown output's envelope, a detected tweeter's floor
            if let Some(e) = &envelope {
                let band = (f1, f2);
                let g = playback::signal_peak(s, o, &chain, level_db, band, e.safe.below_hz);
                let g = playback::SignalPeak {
                    peak_db: g.peak_db + g.limiter_db,
                    below_db: g.below_db + g.limiter_db,
                    limiter_db: 0.0,
                    ..g
                };
                let amp = playback::signal_chain(s, o, flags)
                    .map(|c| playback::signal_peak(s, o, &c, level_db, band, e.safe.below_hz));
                for w in envelope_why(e, amp.as_ref(), &g) {
                    if !why.contains(&w) {
                        why.push(w);
                    }
                }
            }
            let knowledge = envelope.as_ref().map_or("trusted", |e| e.knowledge);
            let kind = if why.is_empty() {
                "signal"
            } else {
                "VIOLATION"
            };
            let Some(p) = played else {
                let kind = if why.is_empty() { "info" } else { kind };
                self.note(json!({"kind": kind, "how": how, "device": dev.name,
                    "serial": dev.serial, "output": o, "driver": x.name,
                    "what": "a test signal on an output that plays nothing (disabled, muted, or the volume at its bottom)",
                    "generatorPeakDbfs": r1(generator.peak_db),
                    "generatorBelowDbfs": generator.below_db.is_finite().then(|| r1(generator.below_db)),
                    "knowledge": knowledge, // -- WP ADETS --
                    "why": why}));
                continue;
            };
            self.note(
                json!({"kind": kind, "how": how, "device": dev.name, "serial": dev.serial,
                "output": o, "driver": x.name, "signal": signal, "levelDb": level_db,
                "f1": f1, "f2": f2, "durationMs": duration_ms,
                "peakDbfs": r1(p.peak_db), "peakHz": p.peak_hz.round(),
                "belowDbfs": p.below_db.is_finite().then(|| r1(p.below_db)),
                "highpass": highpass_text(&chain), "why": why,
                // -- WP FBUS -- before the volume, and the limiter's share
                "generatorPeakDbfs": r1(generator.peak_db), "limiterDb": r1(p.limiter_db),
                // -- WP INTH --
                "generatorBelowDbfs": generator.below_db.is_finite().then(|| r1(generator.below_db)),
                // -- WP ADETS -- what the core knew of it, the envelope it was held to
                "knowledge": knowledge,
                "envelope": envelope.as_ref().map(|e| json!({"belowHz": e.safe.below_hz,
                    "belowDbfs": e.safe.below_dbfs, "capDbfs": e.safe.cap_dbfs}))}),
            );
        }
    }

    /// The settings against full-scale programme at the volume now set, and at
    /// full volume.
    fn judge_settings(&self, dev: &DeviceSpec, s: &DspState) {
        let volume = f64::from(s.user_volume_db.min(0.0)) + f64::from(s.master_volume_db.min(0.0));
        for x in &dev.outputs {
            let o = usize::from(x.output);
            if !routed(s, o) || off(s, o) {
                continue;
            }
            // The worst input: correlated programme on every routed input adds up
            let mut below = f64::NEG_INFINITY;
            let mut below_at = 0.0;
            let mut peak = f64::NEG_INFINITY;
            let mut peak_at = 0.0;
            for f in grid(24.0) {
                let mut sum = 0.0;
                for i in 0..usize::from(s.num_input_channels) {
                    let (re, im) = path_response(s, i, o, f);
                    let db = 10.0 * (re * re + im * im).max(1e-30).log10();
                    sum += 10f64.powf(db / 20.0);
                    if f < x.safe.below_hz * 0.75 && db > below {
                        (below, below_at) = (db, f);
                    }
                }
                let db = 20.0 * sum.max(1e-15).log10();
                if db > peak {
                    (peak, peak_at) = (db, f);
                }
            }
            // A 12 dB/octave high-pass at the safe band's edge passes
            if x.safe.below_hz > 0.0 && below > -4.5 {
                self.note(json!({"kind": "VIOLATION", "how": "settings", "device": dev.name,
                    "serial": dev.serial, "output": o, "driver": x.name,
                    "why": [format!("programme reaches it at {below:.1} dB at {below_at:.0} Hz: no high-pass to speak of below {:.0} Hz", x.safe.below_hz)]}));
            }
            if peak + volume > 0.5 {
                self.note(json!({"kind": "WARN", "how": "settings", "device": dev.name,
                    "serial": dev.serial, "output": o, "driver": x.name,
                    "why": [format!("full-scale programme would clip it: {:.1} dB at {peak_at:.0} Hz at the volume set ({volume:.1} dB)", peak + volume)]}));
            } else if peak > 0.5 {
                self.note(json!({"kind": "info", "how": "settings", "device": dev.name,
                    "output": o, "driver": x.name,
                    "what": format!("at full volume full-scale programme would be {peak:.1} dB at {peak_at:.0} Hz")}));
            }
        }
    }

    // ---------------------------------------------------------- the truth

    fn write_truth(&self, live: &[Live]) {
        let Some(dir) = &self.out else { return };
        let freqs = grid(24.0);
        let room: Vec<SimFilter> = self.spec.modes.iter().map(RoomMode::filter).collect();
        let mut devices = Vec::new();
        // [input][freq]: every device's sum, as one source feeds them all
        let mut inputs: Vec<Vec<(f64, f64)>> = Vec::new();
        for l in live {
            let Some((dev, s)) = self.spec.devices.get(l.spec).zip(decoded(&l.image)) else {
                continue;
            };
            let n_in = usize::from(s.num_input_channels);
            if inputs.len() < n_in {
                inputs.resize(n_in, vec![(0.0, 0.0); freqs.len()]);
            }
            let mut outs = Vec::new();
            for x in &dev.outputs {
                let o = usize::from(x.output);
                let chain = crate::measure::chain_of(&s, x.output);
                let acoustic: Vec<(f64, f64)> =
                    freqs.iter().map(|f| self.acoustic(x, &room, *f)).collect();
                let electric: Vec<(f64, f64)> =
                    freqs.iter().map(|f| chain_response(&chain, *f)).collect();
                let silent = off(&s, o);
                for (i, sum) in inputs.iter_mut().enumerate().take(n_in) {
                    if silent {
                        continue;
                    }
                    for ((f, a), v) in freqs.iter().zip(&acoustic).zip(sum.iter_mut()) {
                        let p = mul(path_response(&s, i, o, *f), *a);
                        *v = (v.0 + p.0, v.1 + p.1);
                    }
                }
                outs.push(json!({
                    "output": o, "name": x.name, "role": x.role, "speaker": x.speaker,
                    "enabled": !silent, "routed": routed(&s, o),
                    "inputs": (0..n_in).filter(|i| s.matrix_routing.get(*i).and_then(|r| r.get(o)).copied().unwrap_or(false)).collect::<Vec<_>>(),
                    "inverted": (0..n_in).filter(|i| s.matrix_invert.get(*i).and_then(|r| r.get(o)).copied().unwrap_or(false)).collect::<Vec<_>>(),
                    "gainDb": chain.gain_db, "delayMs": chain.delay_ms,
                    "arrivalMs": x.delay_ms + x.driver.offset_ms + f64::from(chain.delay_ms),
                    "driverInverted": x.driver.inverted,
                    "electricDb": electric.iter().map(db).collect::<Vec<_>>(),
                    "driverDb": acoustic.iter().map(db).collect::<Vec<_>>(),
                    "micDb": electric.iter().zip(&acoustic).map(|(e, a)| db(&mul(*e, *a))).collect::<Vec<_>>(),
                }));
            }
            devices.push(
                json!({"serial": dev.serial, "name": dev.name, "outputs": outs,
                "userVolumeDb": s.user_volume_db, "masterVolumeDb": s.master_volume_db,
                "state": serde_json::to_value(&s).map(strip).unwrap_or(Value::Null)}),
            );
        }
        let truth = json!({
            "scenario": self.spec.name, "freqsHz": freqs.iter().map(|f| r2(*f)).collect::<Vec<_>>(),
            "devices": devices,
            "inputsDb": inputs.iter().map(|v| v.iter().map(db).collect::<Vec<_>>()).collect::<Vec<_>>(),
        });
        let tmp = dir.join("truth.new");
        if std::fs::write(&tmp, truth.to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, dir.join("truth.json"));
        }
    }

    /// The driver, its path and the room, at `f`.
    fn acoustic(&self, x: &OutputSpec, room: &[SimFilter], f: f64) -> (f64, f64) {
        let mut h = filters_response(&x.driver.filters(), f, FS);
        h = mul(h, filters_response(&x.filters, f, FS));
        h = mul(h, filters_response(room, f, FS));
        let mut r = (1.0, 0.0);
        for k in &self.spec.reflections {
            let e = turn(f, k.delay_ms);
            r = (r.0 + k.gain * e.0, r.1 + k.gain * e.1);
        }
        h = mul(h, r);
        let g = 10f64.powf((x.driver.sensitivity_db + x.gain_db) / 20.0)
            * if x.driver.inverted { -1.0 } else { 1.0 };
        let e = turn(f, x.delay_ms + x.driver.offset_ms);
        mul(h, (g * e.0, g * e.1))
    }
}

/// The rate the responses are computed at.
const FS: f64 = 48_000.0;

fn grid(per_octave: f64) -> Vec<f64> {
    let mut v = Vec::new();
    let mut f = 20.0;
    let step = (1.0 / per_octave).exp2();
    while f <= 20_000.0 {
        v.push(f);
        f *= step;
    }
    v
}

fn decoded(image: &[u8]) -> Option<DspState> {
    let mut s = crate::state::default_state();
    crate::state::decode_bulk(image, &mut s).ok()?;
    Some(s)
}

/// The state without the image.
fn strip(mut v: Value) -> Value {
    if let Some(o) = v.as_object_mut() {
        o.retain(|k, _| !k.to_lowercase().contains("raw"));
    }
    v
}

fn mul(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}

/// A delay of `ms` at `f`.
fn turn(f: f64, ms: f64) -> (f64, f64) {
    let (s, c) = (2.0 * std::f64::consts::PI * f * ms / 1000.0).sin_cos();
    (c, -s)
}

fn db(v: &(f64, f64)) -> f64 {
    r2(10.0 * (v.0 * v.0 + v.1 * v.1).max(1e-20).log10())
}

fn r1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn bands_response(bands: &[BandSpec], f: f64) -> (f64, f64) {
    let model = CoreSectionModel;
    bands.iter().fold((1.0, 0.0), |h, b| {
        model
            .sections(b)
            .iter()
            .fold(h, |h, c| mul(h, section_response(c, f, FS)))
    })
}

/// An output's PEQ, crossover, gain and delay at `f`.
fn chain_response(c: &ChannelChain, f: f64) -> (f64, f64) {
    let h = mul(bands_response(&c.peq, f), bands_response(&c.crossover, f));
    let g = 10f64.powf(f64::from(c.gain_db) / 20.0);
    let e = turn(f, f64::from(c.delay_ms));
    mul(h, (g * e.0, g * e.1))
}

fn chain_db(c: &ChannelChain, f: f64) -> f64 {
    let h = chain_response(c, f);
    10.0 * (h.0 * h.0 + h.1 * h.1).max(1e-30).log10()
}

/// Input `i` to output `o` at `f`, 0 when not routed.
fn path_response(s: &DspState, i: usize, o: usize, f: f64) -> (f64, f64) {
    let at = |m: &Vec<Vec<bool>>| m.get(i).and_then(|r| r.get(o)).copied().unwrap_or(false);
    if !at(&s.matrix_routing) {
        return (0.0, 0.0);
    }
    let input: Vec<BandSpec> = s
        .filters
        .get(i)
        .map(|b| {
            b.iter()
                .map(|f| BandSpec {
                    filter_type: f.filter_type,
                    bypass: f.bypass || s.bypass,
                    freq: f.freq,
                    q: f.q,
                    gain: f.gain,
                    qp: f.qp,
                })
                .collect()
        })
        .unwrap_or_default();
    let gain = s
        .matrix_gain
        .get(i)
        .and_then(|r| r.get(o))
        .copied()
        .unwrap_or(0.0)
        // -- WP FUNC2b -- the global preamp is input 0's (the legacy name, params.rs
        // set_input_preamp): counted once, not on top of every input's own
        + s.input_preamp_db.get(i).copied().unwrap_or(s.preamp_db);
    let g = 10f64.powf(f64::from(gain) / 20.0) * if at(&s.matrix_invert) { -1.0 } else { 1.0 };
    let e = turn(
        f,
        f64::from(s.channel_delays.get(i).copied().unwrap_or(0.0)),
    );
    let h = mul(bands_response(&input, f), (g * e.0, g * e.1));
    let chain = crate::measure::chain_of(s, u8::try_from(o).unwrap_or(u8::MAX));
    mul(h, chain_response(&chain, f))
}

fn routed(s: &DspState, o: usize) -> bool {
    s.matrix_routing
        .iter()
        .take(usize::from(s.num_input_channels))
        .any(|r| r.get(o).copied().unwrap_or(false))
}

fn off(s: &DspState, o: usize) -> bool {
    !s.output_enabled.get(o).copied().unwrap_or(false)
        || s.output_muted.get(o).copied().unwrap_or(false)
}

/// -- WP INTH -- Why a test signal breaks a driver's limits, if it does: at
/// the amplifier (`amp`, after the volume; `None` when the output plays
/// nothing) and as the generator makes it (`generator`, before the volume:
/// FUNC1's reading, which the limits were chosen against). Either breaks it.
fn signal_why(
    safe: &Safe,
    amp: Option<&playback::SignalPeak>,
    generator: &playback::SignalPeak,
) -> Vec<String> {
    let mut why = Vec::new();
    if let Some(p) = amp {
        if p.below_db > safe.below_dbfs {
            why.push(format!(
                "{:.1} dBFS at {:.0} Hz, below its {:.0} Hz (at most {:.0} dBFS there)",
                p.below_db, p.below_hz, safe.below_hz, safe.below_dbfs
            ));
        }
        if p.peak_db > safe.cap_dbfs + 0.05 {
            why.push(format!(
                "{:.1} dBFS at {:.0} Hz, over its cap of {:.0} dBFS",
                p.peak_db, p.peak_hz, safe.cap_dbfs
            ));
        }
        if p.peak_db > 0.05 {
            why.push(format!(
                "{:.1} dBFS at {:.0} Hz: it clips",
                p.peak_db, p.peak_hz
            ));
        }
    }
    let g = generator;
    if g.below_db > safe.below_dbfs {
        why.push(format!(
            "the generator makes {:.1} dBFS at {:.0} Hz before the volume, below its {:.0} Hz (at most {:.0} dBFS there)",
            g.below_db, g.below_hz, safe.below_hz, safe.below_dbfs
        ));
    }
    if g.peak_db > safe.cap_dbfs + 0.05 {
        why.push(format!(
            "the generator makes {:.1} dBFS at {:.0} Hz before the volume, over its cap of {:.0} dBFS",
            g.peak_db, g.peak_hz, safe.cap_dbfs
        ));
    }
    why
}

// -- WP ADETS --
/// Why a test signal breaks an output's envelope (its knowledge's words).
fn envelope_why(
    e: &Envelope,
    amp: Option<&playback::SignalPeak>,
    generator: &playback::SignalPeak,
) -> Vec<String> {
    let what = match e.knowledge {
        "unknown" => "an output with no trusted role (the probe's envelope)",
        _ => "a detected tweeter",
    };
    signal_why(&e.safe, amp, generator)
        .into_iter()
        .filter(|w| !w.ends_with("it clips"))
        .map(|w| format!("{what}: {w}"))
        .collect()
}

fn highpass_text(c: &ChannelChain) -> String {
    c.peq
        .iter()
        .chain(&c.crossover)
        .filter(|b| !b.bypass && dspi_session::limits::highpass_slope(b.filter_type).is_some())
        .map(|b| format!("type {} at {:.0} Hz", b.filter_type, b.freq))
        .collect::<Vec<_>>()
        .join(", ")
}

// -- WP INTH --
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_inth {
    use super::*;

    const TWEETER: Safe = Safe {
        below_hz: 1000.0,
        below_dbfs: -50.0,
        cap_dbfs: -12.0,
    };

    fn state() -> DspState {
        let mut s = crate::state::default_state();
        s.user_volume_db = -20.0;
        s.master_volume_db = 0.0;
        s.user_mute = false;
        s.loudness_enabled = false;
        s.psybass_enabled = false;
        s.output_enabled.fill(true);
        s.output_muted.fill(false);
        s.output_gain_db.fill(0.0);
        s.limiter_enabled.fill(false);
        for b in s.filters.iter_mut().chain(s.xover.iter_mut()).flatten() {
            b.filter_type = 0;
        }
        // An LR4 2 kHz high-pass on output 0, the tweeter
        let n_in = usize::from(s.num_input_channels);
        s.xover[n_in][0].filter_type = 35;
        s.xover[n_in][0].freq = 2000.0;
        s
    }

    /// As `judge_signal` does: at the amplifier, and the generator's level.
    fn why(s: &DspState, level_db: f32, band: (f64, f64)) -> Vec<String> {
        let chain = crate::measure::chain_of(s, 0);
        let g = playback::signal_peak(s, 0, &chain, level_db, band, TWEETER.below_hz);
        let g = playback::SignalPeak {
            peak_db: g.peak_db + g.limiter_db,
            below_db: g.below_db + g.limiter_db,
            limiter_db: 0.0,
            ..g
        };
        let amp = playback::signal_chain(s, 0, 0)
            .map(|c| playback::signal_peak(s, 0, &c, level_db, band, TWEETER.below_hz));
        signal_why(&TWEETER, amp.as_ref(), &g)
    }

    #[test]
    fn wp_inth_a_test_signal_fails_at_the_amplifier_or_at_the_generator() {
        let mut s = state();
        // A 3 kHz tone at -20 dB, volume -20: -40 at the amplifier, both pass
        assert!(why(&s, -20.0, (3000.0, 3000.0)).is_empty());
        // At -6 dB: -26 at the amplifier passes, but the generator's -6 is
        // over FUNC1's -12 cap
        let w = why(&s, -6.0, (3000.0, 3000.0));
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("generator") && w[0].contains("cap"), "{w:?}");
        // A sweep from 200 Hz at -20 dB through the LR4 2 kHz: about -64
        // just below 1 kHz at the amplifier passes; the generator's -44 there
        // is over FUNC1's -50
        let w = why(&s, -20.0, (200.0, 20_000.0));
        assert!(w.iter().all(|w| w.contains("generator")), "{w:?}");
        assert!(w.iter().any(|w| w.contains("below its 1000 Hz")), "{w:?}");
        // The other way round (the amplifier over, the generator not: only
        // boosts after the chain do that, such as loudness), judged alone
        let peak = |peak_db| playback::SignalPeak {
            peak_db,
            peak_hz: 3000.0,
            below_db: f64::NEG_INFINITY,
            below_hz: 0.0,
            limiter_db: 0.0,
        };
        let w = signal_why(&TWEETER, Some(&peak(-8.0)), &peak(-15.0));
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(!w[0].contains("generator") && w[0].contains("cap"), "{w:?}");
        // A muted output: nothing at the amplifier, the generator still counts
        s.output_muted[0] = true;
        assert!(why(&s, -20.0, (3000.0, 3000.0)).is_empty());
        let w = why(&s, -6.0, (3000.0, 3000.0));
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("generator"), "{w:?}");
    }
}

// -- WP FUNC2b --
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests_wp_func2b {
    use super::*;

    /// A preamp on input 0 (also the device's legacy global preamp) lowers input 0
    /// once, and no other input: FUNC2b S15 saw the fronts' L and R 6 dB down for
    /// -3 dB on each.
    #[test]
    fn wp_func2b_the_global_preamp_is_input_0s_and_counts_once() {
        let mut s = crate::state::default_state();
        s.matrix_gain[0][0] = 0.0;
        s.matrix_gain[1][0] = 0.0;
        s.matrix_routing[0][0] = true;
        s.matrix_routing[1][0] = true;
        let flat = |s: &DspState, i| {
            20.0 * {
                let (re, im) = path_response(s, i, 0, 1000.0);
                (re * re + im * im).sqrt().log10()
            }
        };
        let (l0, r0) = (flat(&s, 0), flat(&s, 1));
        s.input_preamp_db[0] = -3.0;
        s.input_preamp_db[1] = -3.0;
        s.preamp_db = -3.0;
        assert!(
            (flat(&s, 0) - l0 - -3.0).abs() < 0.05,
            "{} {}",
            flat(&s, 0),
            l0
        );
        assert!(
            (flat(&s, 1) - r0 - -3.0).abs() < 0.05,
            "{} {}",
            flat(&s, 1),
            r0
        );
    }
}
