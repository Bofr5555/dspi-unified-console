#include "MeasureSpike.h"

// -- WP P5QCAP -- see MeasureSpike.h

#include "AppIdentity.h"
#include "AudioCapture.h"

#include "CoreApi.h"

#include <QApplication>
#include <QCommandLineOption>
#include <QCommandLineParser>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSaveFile>
#include <QSysInfo>
#include <QTimer>
#include <atomic>
#include <cmath>
#include <csignal>
#include <cstdio>

namespace {

const QString kInputs = QStringLiteral("audio-inputs");
const QString kSpike = QStringLiteral("measure-spike");
const QString kSpikeInput = QStringLiteral("measure-spike-input");
const QString kSpikeSeconds = QStringLiteral("measure-spike-seconds");
const QString kSpikeSetup = QStringLiteral("measure-spike-setup");
const QString kSpikeDemo = QStringLiteral("measure-spike-demo");

// Ctrl+C: the tick sees it and stops the session (one STOP_NOW)
std::atomic<bool> g_interrupted{ false };
extern "C" void onInterrupt(int) { g_interrupted.store(true); }

// One JSON reply of the C ABI (the calls write a NUL-terminated text and
// return its length)
template <typename Call> QJsonObject reply(Call call, int capacity = 1 << 18) {
    QByteArray buf(capacity, '\0');
    const uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n >= uint32_t(buf.size())) return QJsonObject{ { QStringLiteral("ok"), false } };
    return QJsonDocument::fromJson(QByteArray(buf.constData())).object();
}

// The first DSPi a USB scan finds, "" for none
QString firstSerial(FfiCore *c) {
    QVector<DeviceInfo> found(16);
    const uint32_t n = dspi_scan_devices(c, found.data(), uint32_t(found.size()));
    if (n == 0) return QString();
    const DeviceInfo &d = found[0];
    const uint32_t len = qMin<uint32_t>(d.serial_len, uint32_t(sizeof d.serial));
    return QString::fromUtf8(reinterpret_cast<const char *>(d.serial), int(len));
}

// The session's setup: the default (two outputs walked, three passes, on a
// coupler: the start level is -60 dBFS) or the caller's, at the capture's
// rate. The start level only, whatever the caller's says.
QJsonObject sessionSetup(const QString &json, int rateHz) {
    QJsonObject setup;
    if (json.trimmed().isEmpty()) {
        const auto out = [](int o) {
            return QJsonObject{ { QStringLiteral("output"), o },
                                { QStringLiteral("role"), QStringLiteral("full") },
                                { QStringLiteral("speaker"), QStringLiteral("L") } };
        };
        setup = QJsonObject{ { QStringLiteral("kind"), QStringLiteral("walked") },
                             { QStringLiteral("outputs"), QJsonArray{ out(0), out(1) } },
                             { QStringLiteral("placement"), QStringLiteral("headphones") },
                             { QStringLiteral("coupler"), QStringLiteral("flatPlate") },
                             { QStringLiteral("passes"), 3 },
                             { QStringLiteral("name"), QStringLiteral("P5SPIKE (Qt)") } };
    } else {
        setup = QJsonDocument::fromJson(json.toUtf8()).object();
    }
    setup.insert(QStringLiteral("startLevelOnly"), true);
    setup.insert(QStringLiteral("captureRateHz"), rateHz);
    return setup;
}

QJsonObject header() {
    return QJsonObject{
        { QStringLiteral("client"), QStringLiteral("qt") },
        { QStringLiteral("app"), AppIdentity::version() },
        { QStringLiteral("qt"), QString::fromLatin1(qVersion()) },
        { QStringLiteral("os"), QSysInfo::prettyProductName() },
    };
}

bool writeJson(const QString &path, const QJsonObject &doc) {
    QSaveFile f(path);
    if (!f.open(QIODevice::WriteOnly)) return false;
    f.write(QJsonDocument(doc).toJson(QJsonDocument::Indented));
    return f.commit();
}

QJsonArray inputList() {
    QJsonArray list;
    const QList<dspi_capture::InputDevice> inputs = AudioCapture::inputs();
    for (const dspi_capture::InputDevice &in : inputs) {
        list.append(QJsonObject{ { QStringLiteral("id"), QString::fromLatin1(in.id.toHex()) },
                                 { QStringLiteral("name"), in.name },
                                 { QStringLiteral("default"), in.isDefault } });
    }
    return list;
}

const char *failureName(dspi_capture::Failure f) {
    switch (f) {
    case dspi_capture::Failure::None: return "none";
    case dspi_capture::Failure::NoDevice: return "no-device";
    case dspi_capture::Failure::FormatUnsupported: return "format-unsupported";
    case dspi_capture::Failure::OpenFailed: return "open-failed";
    case dspi_capture::Failure::DeviceLost: return "device-lost";
    }
    return "unknown";
}

// What the spike learns from the blocks alone
struct Stats {
    qint64 blocks = 0;
    qint64 frames = 0;
    qint64 expectedNext = 0;
    qint64 clippedBlocks = 0;
    qint64 maxLead = -1;
    QJsonArray gaps;      // {atFrame, lostFrames}
    QJsonArray repeats;   // {atFrame, repeatedFrames}
    // Least squares of the frame index at each block's end against the host's
    // monotonic clock (seconds since the first block)
    double n = 0, sx = 0, sy = 0, sxx = 0, sxy = 0;
    std::vector<std::pair<double, double>> points;

    void add(const dspi_capture::Block &b, double hostS) {
        if (blocks > 0 && b.firstFrame > expectedNext) {
            gaps.append(QJsonObject{ { QStringLiteral("atFrame"), double(expectedNext) },
                                     { QStringLiteral("lostFrames"), double(b.firstFrame - expectedNext) } });
        } else if (blocks > 0 && b.firstFrame < expectedNext) {
            repeats.append(QJsonObject{ { QStringLiteral("atFrame"), double(b.firstFrame) },
                                        { QStringLiteral("repeatedFrames"), double(expectedNext - b.firstFrame) } });
        }
        ++blocks;
        frames += b.samples.size();
        expectedNext = b.firstFrame + b.samples.size();
        if (b.clipped) ++clippedBlocks;
        maxLead = qMax(maxLead, b.sourceLeadFrames);
        const double y = double(expectedNext);
        n += 1;
        sx += hostS;
        sy += y;
        sxx += hostS * hostS;
        sxy += hostS * y;
        points.emplace_back(hostS, y);
    }

    QJsonObject clock(int rateHz) const {
        QJsonObject out{ { QStringLiteral("points"), n } };
        const double d = n * sxx - sx * sx;
        if (n < 3 || d <= 0 || rateHz <= 0) return out;
        const double slope = (n * sxy - sx * sy) / d;   // frames per host second
        const double icept = (sy - slope * sx) / n;
        double maxDev = 0;
        for (const auto &p : points) maxDev = std::max(maxDev, std::fabs(p.second - (icept + slope * p.first)));
        out.insert(QStringLiteral("framesPerHostSecond"), slope);
        out.insert(QStringLiteral("ppmVsHost"), (slope / rateHz - 1.0) * 1e6);
        out.insert(QStringLiteral("maxDeviationMs"), maxDev * 1000.0 / rateHz);
        out.insert(QStringLiteral("note"), QStringLiteral("block arrival on the GUI thread against frame count: "
                                                          "the microphone's clock against the host's, with the "
                                                          "delivery jitter in the deviation"));
        return out;
    }
};

int runInputs(const QString &path) {
    QJsonObject doc = header();
    doc.insert(QStringLiteral("kind"), QStringLiteral("dspi-audio-inputs"));
    doc.insert(QStringLiteral("inputs"), inputList());
    if (!writeJson(path, doc)) {
        std::fprintf(stderr, "audio-inputs: cannot write %s\n", qPrintable(path));
        return 1;
    }
    return 0;
}

} // namespace

namespace MeasureSpike {

int run(const Options &o) {
    AudioCapture capture;
    Stats stats;
    QElapsedTimer host;
    QElapsedTimer wall;
    QEventLoop loop;
    QTimer ticker;
    QTimer limit;
    limit.setSingleShot(true);
    QJsonObject doc = header();
    doc.insert(QStringLiteral("kind"), QStringLiteral("dspi-measure-spike"));
    doc.insert(QStringLiteral("startLevelDb"), kStartLevelDb);
    doc.insert(QStringLiteral("inputs"), inputList());
    doc.insert(QStringLiteral("residuals"), QJsonArray());
    QString failure;
    bool captureFailed = false;

    // The DSPi: the demo, or the first on USB. Without one the spike is the
    // capture statistics alone.
    FfiCore *core = dspi_core_new();
    bool device = false;
    if (core && o.demo) {
        // The demo's volume is above what a measurement starts at (S2); it is
        // a simulated device, so the spike lowers it. A real DSPi is never
        // written to: its preflight says what the owner must change.
        device = dspi_open_demo(core) && dspi_fetch_all(core) && dspi_set_user_volume(core, -20.0f);
        doc.insert(QStringLiteral("device"), QStringLiteral("demo"));
    } else if (core) {
        const QString serial = firstSerial(core);
        device = !serial.isEmpty() && dspi_select_device(core, serial.toUtf8().constData()) && dspi_fetch_all(core);
        if (device) doc.insert(QStringLiteral("device"), serial);
    }
    doc.insert(QStringLiteral("session"), device ? QStringLiteral("not-started") : QStringLiteral("no-device"));
    if (!device)
        doc.insert(QStringLiteral("sessionNote"),
                   QStringLiteral("no DSPi was opened: the generator was not started; capture statistics only"));

    bool running = false;    // dspi_measure_start succeeded and the session has not ended
    bool done = false;       // the session reached `done`
    QString stoppedBy;
    QString lastPhase;
    QJsonArray phases;
    double maxLevelDb = -1000.0;
    int pushRefusals = 0;

    const auto note = [&](const QJsonObject &st) {
        const QString phase = st.value(QStringLiteral("phase")).toString();
        if (st.contains(QStringLiteral("levelDb")) && st.value(QStringLiteral("levelDb")).isDouble())
            maxLevelDb = qMax(maxLevelDb, st.value(QStringLiteral("levelDb")).toDouble());
        if (phase.isEmpty() || phase == lastPhase) return;
        lastPhase = phase;
        phases.append(QJsonObject{ { QStringLiteral("phase"), phase },
                                   { QStringLiteral("atMs"), double(wall.elapsed()) },
                                   { QStringLiteral("frames"), st.value(QStringLiteral("frames")) } });
        std::printf("measure-spike: %s\n", qPrintable(phase));
        std::fflush(stdout);
    };
    // The session's end: its state, timing and document into the report
    const auto finish = [&] {
        if (!running) return;
        running = false;
        ticker.stop();
        const QJsonObject st = reply([&](char *b, uint32_t l) { return dspi_measure_state(core, b, l); });
        note(st);
        done = st.value(QStringLiteral("phase")).toString() == QLatin1String("done");
        doc.insert(QStringLiteral("session"), st.value(QStringLiteral("phase")).toString());
        doc.insert(QStringLiteral("state"), st);
        const QJsonObject timing = st.value(QStringLiteral("timing")).toObject();
        if (!timing.isEmpty()) {
            doc.insert(QStringLiteral("timing"), timing);
            doc.insert(QStringLiteral("residuals"), timing.value(QStringLiteral("cycleResidualsUs")));
            doc.insert(QStringLiteral("verdict"), timing.value(QStringLiteral("reliable")).toBool()
                                                       ? QStringLiteral("timing reliable")
                                                       : QStringLiteral("timing unreliable"));
        } else {
            const QJsonObject issue = st.value(QStringLiteral("issue")).toObject();
            doc.insert(QStringLiteral("verdict"),
                       QStringLiteral("no timing: ") + st.value(QStringLiteral("phase")).toString() +
                           QStringLiteral(" ") + issue.value(QStringLiteral("key")).toString() + QStringLiteral(" ") +
                           issue.value(QStringLiteral("message")).toString());
        }
        if (st.value(QStringLiteral("resultReady")).toBool()) {
            const QJsonObject result =
                reply([&](char *b, uint32_t l) { return dspi_measure_result(core, b, l); }, 1 << 22);
            doc.insert(QStringLiteral("result"), result);   // derived curves only (S12)
        }
        SiggenStatus sg{};
        if (dspi_siggen_get_status(core, &sg))
            doc.insert(QStringLiteral("generatorAfter"), QJsonObject{ { QStringLiteral("state"), int(sg.state) },
                                                                      { QStringLiteral("stopReason"), int(sg.stop_reason) },
                                                                      { QStringLiteral("cyclesDone"), int(sg.cycles_done) } });
        if (capture.isRunning()) capture.stop();
        else loop.quit();
    };
    // Stop: one dspi_measure_stop (STOP_NOW where the generator plays)
    const auto stop = [&](const QString &why) {
        if (!running) return;
        stoppedBy = why;
        note(reply([&](char *b, uint32_t l) { return dspi_measure_stop(core, b, l); }));
        finish();
    };

    QObject::connect(&capture, &AudioCapture::started, &loop, [&](int rateHz, int channels, const QString &name) {
        doc.insert(QStringLiteral("input"), QJsonObject{ { QStringLiteral("id"), QString::fromLatin1(capture.deviceId().toHex()) },
                                                         { QStringLiteral("name"), name } });
        doc.insert(QStringLiteral("format"), QJsonObject{ { QStringLiteral("rateHz"), rateHz },
                                                          { QStringLiteral("channels"), channels } });
        std::printf("measure-spike: capturing %d Hz, %d channel(s) from \"%s\", at most %d s; the host plays nothing\n",
                    rateHz, channels, qPrintable(name), o.seconds);
        std::fflush(stdout);
        limit.start(o.seconds * 1000);
        if (!device) return;
        const QJsonObject setup = sessionSetup(o.setupJson, rateHz);
        const QByteArray setupText = QJsonDocument(setup).toJson(QJsonDocument::Compact);
        doc.insert(QStringLiteral("setup"), setup);
        const QJsonObject pre =
            reply([&](char *b, uint32_t l) { return dspi_measure_preflight(core, setupText.constData(), b, l); });
        doc.insert(QStringLiteral("preflight"), pre);
        if (!pre.value(QStringLiteral("ok")).toBool() || !pre.value(QStringLiteral("clear")).toBool()) {
            doc.insert(QStringLiteral("session"), QStringLiteral("refused"));
            failure = QStringLiteral("the preflight is not clear: see \"preflight\"");
            std::fprintf(stderr, "measure-spike: %s\n",
                         QJsonDocument(pre).toJson(QJsonDocument::Compact).constData());
            capture.stop();
            return;
        }
        const QJsonObject st =
            reply([&](char *b, uint32_t l) { return dspi_measure_start(core, setupText.constData(), b, l); });
        if (!st.value(QStringLiteral("ok")).toBool()) {
            doc.insert(QStringLiteral("session"), QStringLiteral("refused"));
            doc.insert(QStringLiteral("start"), st);
            failure = QStringLiteral("the session did not start: ") + st.value(QStringLiteral("error")).toString();
            capture.stop();
            return;
        }
        running = true;
        wall.start();
        note(st);
        ticker.start(50);
    });
    QObject::connect(&capture, &AudioCapture::block, &loop, [&](const dspi_capture::Block &b) {
        if (!host.isValid()) host.start();
        stats.add(b, double(host.nsecsElapsed()) / 1e9);
        if (!running) return;
        const QJsonObject st = reply([&](char *buf, uint32_t l) {
            return dspi_measure_capture_push(core, uint64_t(b.firstFrame), b.samples.constData(),
                                             uint32_t(b.samples.size()), uint32_t(b.rateHz), b.clipped, buf, l);
        });
        if (!st.value(QStringLiteral("ok")).toBool()) ++pushRefusals;
    });
    QObject::connect(&ticker, &QTimer::timeout, &loop, [&] {
        if (!running) return;
        if (g_interrupted.load()) {
            stop(QStringLiteral("interrupted"));
            return;
        }
        const QJsonObject st = reply([&](char *b, uint32_t l) { return dspi_measure_tick(core, b, l); });
        note(st);
        if (!st.value(QStringLiteral("active")).toBool()) finish();
    });
    QObject::connect(&limit, &QTimer::timeout, &loop, [&] {
        if (running) stop(QStringLiteral("time-limit"));
        else capture.stop();
    });
    QObject::connect(&capture, &AudioCapture::failed, &loop, [&](dspi_capture::Failure f, const QString &detail) {
        failure = QString::fromLatin1(failureName(f)) + QStringLiteral(": ") + detail;
        captureFailed = true;
        stop(QStringLiteral("capture-failed"));
    });
    QObject::connect(&capture, &AudioCapture::stopped, &loop, [&] {
        stop(QStringLiteral("capture-stopped"));
        loop.quit();
    });

    const auto previous = std::signal(SIGINT, onInterrupt);
    const bool opened = o.source ? capture.startWith(o.source, o.inputId) : capture.start(o.inputId);
    if (opened) loop.exec();
    else if (failure.isEmpty()) failure = QStringLiteral("the capture did not start");
    std::signal(SIGINT, previous);
    // Whatever ended the loop, the session does not outlive it
    if (core) {
        if (running) stop(QStringLiteral("ended"));
        dspi_disconnect(core);
        dspi_core_free(core);
    }

    const int rate = doc.value(QStringLiteral("format")).toObject().value(QStringLiteral("rateHz")).toInt();
    doc.insert(QStringLiteral("blocks"), double(stats.blocks));
    doc.insert(QStringLiteral("frames"), double(stats.frames));
    doc.insert(QStringLiteral("gaps"), stats.gaps);
    doc.insert(QStringLiteral("repeats"), stats.repeats);
    doc.insert(QStringLiteral("clippedBlocks"), double(stats.clippedBlocks));
    doc.insert(QStringLiteral("maxSourceLeadFrames"), double(stats.maxLead));
    doc.insert(QStringLiteral("clock"), stats.clock(rate));
    doc.insert(QStringLiteral("phases"), phases);
    doc.insert(QStringLiteral("pushRefusals"), pushRefusals);
    if (maxLevelDb > -999.0) doc.insert(QStringLiteral("maxLevelDb"), maxLevelDb);
    if (!stoppedBy.isEmpty()) doc.insert(QStringLiteral("stoppedBy"), stoppedBy);
    if (!failure.isEmpty()) doc.insert(QStringLiteral("failure"), failure);
    if (!writeJson(o.path, doc)) {
        std::fprintf(stderr, "measure-spike: cannot write %s\n", qPrintable(o.path));
        return 1;
    }
    std::printf("measure-spike: %lld blocks, %lld gap(s), %lld repeat(s), %lld clipped block(s); session: %s; %s -> %s\n",
                static_cast<long long>(stats.blocks), static_cast<long long>(stats.gaps.size()),
                static_cast<long long>(stats.repeats.size()), static_cast<long long>(stats.clippedBlocks),
                qPrintable(doc.value(QStringLiteral("session")).toString()),
                qPrintable(doc.value(QStringLiteral("verdict")).toString()), qPrintable(o.path));
    if (!failure.isEmpty()) std::fprintf(stderr, "measure-spike: %s\n", qPrintable(failure));
    if (captureFailed || !failure.isEmpty()) return 1;
    return (!device || done) ? 0 : 1;
}

void addOptions(QCommandLineParser &args) {
    // Development switches: not translated, not in --help
    const QList<QCommandLineOption> options{
        QCommandLineOption(kInputs, QStringLiteral("Write the audio input list to <file> and exit."),
                           QStringLiteral("file")),
        QCommandLineOption(kSpike, QStringLiteral("Measurement spike: a start-level-only run, its report to <file>."),
                           QStringLiteral("file")),
        QCommandLineOption(kSpikeInput, QStringLiteral("The spike's audio input id (hex, from --audio-inputs)."),
                           QStringLiteral("id")),
        QCommandLineOption(kSpikeSeconds, QStringLiteral("The spike's time limit, 1 to 300 s."),
                           QStringLiteral("seconds"), QStringLiteral("120")),
        QCommandLineOption(kSpikeSetup, QStringLiteral("The spike's session setup (JSON file)."),
                           QStringLiteral("file")),
        QCommandLineOption(kSpikeDemo, QStringLiteral("The spike measures the demo DSPi.")),
    };
    for (QCommandLineOption o : options) {
        o.setFlags(QCommandLineOption::HiddenFromHelp);
        args.addOption(o);
    }
}

int runIfAsked(const QCommandLineParser &args, QApplication &) {
    if (args.isSet(kInputs)) return runInputs(args.value(kInputs));
    if (!args.isSet(kSpike)) return -1;
    bool ok = false;
    Options o;
    o.path = args.value(kSpike);
    o.seconds = args.value(kSpikeSeconds).toInt(&ok);
    if (!ok || o.seconds < 1 || o.seconds > 300) {
        std::fprintf(stderr, "measure-spike: --measure-spike-seconds is 1 to 300\n");
        return 2;
    }
    o.inputId = QByteArray::fromHex(args.value(kSpikeInput).toLatin1());
    o.demo = args.isSet(kSpikeDemo);
    if (args.isSet(kSpikeSetup)) {
        QFile f(args.value(kSpikeSetup));
        if (!f.open(QIODevice::ReadOnly) || !QJsonDocument::fromJson(f.readAll()).isObject()) {
            std::fprintf(stderr, "measure-spike: --measure-spike-setup is not a JSON object\n");
            return 2;
        }
        f.seek(0);
        o.setupJson = QString::fromUtf8(f.readAll());
    }
    return run(o);
}

} // namespace MeasureSpike
