#ifndef MEASURESPIKE_H
#define MEASURESPIKE_H

// -- WP P5QCAP -- Development switches for P5SPIKE (PLAN-P5 section 2, RC5g),
// hidden from --help, run instead of the app's window:
//
//   --audio-inputs <file>           the input list (QMediaDevices) as JSON, then exit
//   --measure-spike <file>          capture from the microphone and write a report:
//       --measure-spike-input <id>    the input's id (default: the system's default)
//       --measure-spike-seconds <n>   1 to 120, default 30
//
// The report (JSON) has the capture's format, every frame gap and repeat seen,
// clipped blocks, the microphone clock against the host's monotonic clock, the
// backend's lead over the reader, and the timing residuals of a walked run when
// the build has the core's measurement session.
//
// Start level only: when the session is there, the spike's setup has its
// ceiling at the start level (-60 dBFS), so the ramp never goes above it. This
// build has no session calls in the C ABI yet (P5SES, the same wave): the spike
// then plays nothing, starts no generator, and says so in the report
// ("session": "absent"). Nothing is ever played by the host (S1), and no audio
// is written to the file (S12): only counts and times.

class QApplication;
class QCommandLineParser;

namespace MeasureSpike {

// The generator's level for the spike, dBFS peak: the speakers' start level is
// -50, the headphones' -60 (S3); the spike uses the lower for every output.
constexpr double kStartLevelDb = -60.0;

void addOptions(QCommandLineParser &args);
// -1 when no spike switch was given; otherwise runs it and returns the exit code
// (0 done, 1 failed, 2 bad arguments)
int runIfAsked(const QCommandLineParser &args, QApplication &app);

} // namespace MeasureSpike

#endif // MEASURESPIKE_H
