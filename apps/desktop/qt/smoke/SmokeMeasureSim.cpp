// -- WP P5DEMO -- The spike on the demo DSPi with its simulated microphone
// (dspi_measure_sim_set): P5SIM's three-way speaker in a room answers the
// generator, so the run reaches a timing verdict (review P5-B noted that a
// WAV of noise, which does not answer the generator, ends without one). No
// capture is opened and nothing is played: the microphone is arithmetic in
// the core, fed from dspi_measure_tick.
#include "SmokeDriver.h"

#include "MeasureSpike.h"

#include <QCoreApplication>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>

namespace {

QJsonObject readJson(const QString &path) {
    QFile f(path);
    return f.open(QIODevice::ReadOnly) ? QJsonDocument::fromJson(f.readAll()).object() : QJsonObject();
}

void simSection(SmokeDriver &d) {
    const QString dir = d.outDir();

    // In this process: the spike's own run, the default setup (two outputs
    // walked, three passes, on a coupler, the start level only).
    MeasureSpike::Options o;
    o.path = dir + QStringLiteral("/p5demo-spike-sim.json");
    o.seconds = 120;
    o.demo = true;
    o.simMic = true;
    o.simSeed = 7;
    const int code = MeasureSpike::run(o);
    const QJsonObject doc = readJson(o.path);
    const QJsonObject st = doc.value(QStringLiteral("state")).toObject();
    const QJsonObject timing = doc.value(QStringLiteral("timing")).toObject();
    const QJsonObject sim = doc.value(QStringLiteral("simulatedMicrophone")).toObject();
    d.check(QStringLiteral("p5demo: the simulated microphone is on for the demo DSPi, labelled"),
            sim.value(QStringLiteral("on")).toBool() &&
                sim.value(QStringLiteral("label")).toString() == QLatin1String("Simulated microphone") &&
                doc.value(QStringLiteral("input")).toObject().value(QStringLiteral("name")).toString() ==
                    QLatin1String("Simulated microphone"));
    d.check(QStringLiteral("p5demo: the spike's run is done with a timing verdict, exit 0"),
            code == 0 && st.value(QStringLiteral("phase")).toString() == QLatin1String("done") &&
                doc.value(QStringLiteral("verdict")).toString() == QLatin1String("timing reliable"));
    d.check(QStringLiteral("p5demo: the timing has the clock ratio and six cycle residuals within the bound"),
            timing.value(QStringLiteral("reliable")).toBool() &&
                timing.value(QStringLiteral("cycleResidualsUs")).toArray().size() == 6 &&
                timing.value(QStringLiteral("maxResidualUs")).toDouble() <=
                    timing.value(QStringLiteral("residualBoundUs")).toDouble() &&
                qAbs(timing.value(QStringLiteral("ratioPpm")).toDouble() - 37.5) < 5.0);
    d.check(QStringLiteral("p5demo: never above -60 dBFS, no frame fault, no capture opened, the generator idle"),
            doc.value(QStringLiteral("maxLevelDb")).toDouble() == -60.0 &&
                st.value(QStringLiteral("frameFaults")).toInt(-1) == 0 &&
                doc.value(QStringLiteral("blocks")).toDouble() == 0.0 &&
                doc.value(QStringLiteral("generatorAfter")).toObject().value(QStringLiteral("state")).toInt(-1) == 0 &&
                doc.value(QStringLiteral("result")).toObject().value(QStringLiteral("meta")).toObject()
                        .value(QStringLiteral("kind")).toString() == QLatin1String("measurement"));
    if (code != 0) qWarning("p5demo: spike report: %s", QJsonDocument(doc).toJson(QJsonDocument::Compact).constData());

    // As a launch: the switch alone (no WAV, no input) runs it end to end.
    const QString path = dir + QStringLiteral("/p5demo-spike-sim-cli.json");
    QProcess p;
    p.setProcessChannelMode(QProcess::ForwardedChannels);
    p.start(QCoreApplication::applicationFilePath(),
            { QStringLiteral("--measure-spike"), path, QStringLiteral("--measure-spike-sim"),
              QStringLiteral("--measure-spike-seconds"), QStringLiteral("120") });
    const bool ran = p.waitForFinished(150000) && p.exitStatus() == QProcess::NormalExit && p.exitCode() == 0;
    const QJsonObject cli = readJson(path);
    d.check(QStringLiteral("p5demo: --measure-spike-sim runs the demo with its simulated microphone to a verdict"),
            ran && cli.value(QStringLiteral("device")).toString() == QLatin1String("demo") &&
                cli.value(QStringLiteral("verdict")).toString() == QLatin1String("timing reliable"));
}

const SmokeDriver::Section sim("p5demo-sim", 206, &simSection);

} // namespace
