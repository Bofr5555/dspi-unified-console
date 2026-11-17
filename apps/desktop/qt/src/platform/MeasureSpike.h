#ifndef MEASURESPIKE_H
#define MEASURESPIKE_H

// -- WP P5QCAP -- Development switches for P5SPIKE (PLAN-P5 section 2, RC5g),
// hidden from --help, run instead of the app's window:
//
//   --audio-inputs <file>           the input list (QMediaDevices) as JSON, then exit
//   --measure-spike <file>          a walked run at the start level only, its report to <file>:
//       --measure-spike-input <id>    the input's id (default: the system's default)
//       --measure-spike-seconds <n>   the longest the spike runs before it stops itself,
//                                     1 to 300, default 120
//       --measure-spike-setup <file>  the session's setup (MeasureSetup as JSON) in place of
//                                     the default; startLevelOnly is always forced on
//       --measure-spike-demo          the demo DSPi (simulated; its volume set to -20 dB)
//                                     in place of the first DSPi on USB
//       --measure-spike-sim           -- WP P5DEMO -- the demo DSPi with its simulated
//                                     microphone (P5SIM's room answers the generator): no
//                                     capture is opened, the run reaches a timing verdict
//
// -- Review P5-B (decision a) -- The spike runs the core's measurement session
// (P5SES) through the C ABI: the first DSPi on USB (or the demo), the preflight
// (every blocker in the report; the spike changes nothing on a real DSPi, so
// lower the volume to -12 dB or below first), dspi_measure_start with
// startLevelOnly, every capture block into dspi_measure_capture_push with its
// frame index, dspi_measure_tick every 50 ms, and at the end the state, the
// timing (the clock ratio, every cycle's residual, the verdict) and the
// measurement document. Ctrl+C, the time limit, a capture failure: one
// dspi_measure_stop (STOP_NOW).
//
// The report (JSON) also has the capture's format, every frame gap and repeat
// seen, clipped blocks, the microphone clock against the host's monotonic
// clock and the backend's lead over the reader. With no DSPi found it is the
// capture statistics alone ("session": "no-device").
//
// Start level only: the session's ceiling is its start level (-60 dBFS on the
// default headphone setup, -50 dBFS for a speaker placement), so the ramp
// never goes above it. Nothing is ever played by the host (S1), and no audio
// is written to the file (S12): only counts, times and derived curves.

#include "AudioCapture.h"

#include <QByteArray>
#include <QString>

class QApplication;
class QCommandLineParser;

namespace MeasureSpike {

// The generator's level of the default setup, dBFS peak: the headphones' start
// level (S3); a speaker placement's is -50.
constexpr double kStartLevelDb = -60.0;
// -- WP P5DEMO -- the simulated microphone's capture rate
constexpr int kSimRateHz = 48000;

struct Options {
    QString path;                 // the report
    QByteArray inputId;           // empty: the default input
    int seconds = 120;            // the spike stops itself after this long
    bool demo = false;            // the demo DSPi in place of USB
    QString setupJson;            // empty: the default setup
    bool simMic = false;          // -- WP P5DEMO -- the demo's simulated microphone (implies demo)
    quint64 simSeed = 1;          // -- WP P5DEMO -- its world's seed
    dspi_capture::SourceFactory source;   // the capture's test seam (smoke tests)
};

void addOptions(QCommandLineParser &args);
// -1 when no spike switch was given; otherwise runs it and returns the exit code
// (0 the run completed, 1 it did not, 2 bad arguments)
int runIfAsked(const QCommandLineParser &args, QApplication &app);
// The spike itself, on its own event loop (0 the run completed, 1 it did not)
int run(const Options &options);

} // namespace MeasureSpike

#endif // MEASURESPIKE_H
