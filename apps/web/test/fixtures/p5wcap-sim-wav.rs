// WP P5WCAP: how test/fixtures/p5wcap-sim.wav (the Chromium e2e fake microphone) was rendered with
// P5SIM. A throwaway crate outside the workspace: Cargo.toml `[package] name = "p5wcap-simwav"`,
// `[workspace]`, `[dependencies] dspi-measure = { path = "<repo>/crates/dspi-measure", features = ["sim"] }`,
// the workspace Cargo.lock copied beside it; in the build container:
//   ci/run.sh 'cd /work/simwav && cargo run --offline -q -- /work/p5wcap-sim.wav'
// It printed "65408 frames at 44100 Hz, peak 0.2637997".
use dspi_measure::sim::{render, test_sweep, ProtoModel, Stimulus, World, two_way_chains};
use dspi_measure::wav::WavFormat;

fn main() {
    let out = std::env::args().nth(1).expect("output path");
    // The DSPi at 48 kHz, the microphone at 44.1 kHz: the rate Chromium's fake microphone runs
    // at, so its file is passed through unresampled.
    let fs = 48_000;
    let mut w = World::two_way_near_field(5);
    w.clock.rate_hz = 44_100;
    let sweep = test_sweep(fs, 20.0, 20_000.0, 0.5, 5.0, 0.5).unwrap();
    let stim = Stimulus::walked(&sweep, &[0, 1], 1, 12_000, 4_800, 4_800).unwrap();
    let c = render(&w, &stim, &two_way_chains(), &ProtoModel, fs).unwrap();
    let peak = c.samples.iter().fold(0f32, |m, v| m.max(v.abs()));
    std::fs::write(&out, c.to_wav(WavFormat::Pcm16).unwrap()).unwrap();
    println!("{} frames at {} Hz, peak {peak}", c.samples.len(), c.rate_hz);
}
