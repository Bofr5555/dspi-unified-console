#include "MeasureSpike.h"

// -- WP P5QCAP -- see MeasureSpike.h

#include "AppIdentity.h"
#include "AudioCapture.h"

#include <QApplication>
#include <QCommandLineOption>
#include <QCommandLineParser>
#include <QElapsedTimer>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSaveFile>
#include <QSysInfo>
#include <QTimer>
#include <cmath>
#include <cstdio>

namespace {

const QString kInputs = QStringLiteral("audio-inputs");
const QString kSpike = QStringLiteral("measure-spike");
const QString kSpikeInput = QStringLiteral("measure-spike-input");
const QString kSpikeSeconds = QStringLiteral("measure-spike-seconds");

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

int runSpike(const QString &path, const QByteArray &inputId, int seconds, QApplication &app) {
    AudioCapture capture;
    Stats stats;
    QElapsedTimer host;
    QJsonObject doc = header();
    doc.insert(QStringLiteral("kind"), QStringLiteral("dspi-measure-spike"));
    doc.insert(QStringLiteral("startLevelDb"), MeasureSpike::kStartLevelDb);
    doc.insert(QStringLiteral("inputs"), inputList());
    // The core's measurement session is not in this build's C ABI (P5SES): no
    // generator is started, no residuals are computed
    doc.insert(QStringLiteral("session"), QStringLiteral("absent"));
    doc.insert(QStringLiteral("sessionNote"),
               QStringLiteral("this build has no measurement session (P5SES): the generator was not started; "
                              "capture statistics only"));
    doc.insert(QStringLiteral("residuals"), QJsonArray());
    int code = 0;
    QString failure;

    QObject::connect(&capture, &AudioCapture::started, &app, [&](int rateHz, int channels, const QString &name) {
        doc.insert(QStringLiteral("input"), QJsonObject{ { QStringLiteral("id"), QString::fromLatin1(capture.deviceId().toHex()) },
                                                         { QStringLiteral("name"), name } });
        doc.insert(QStringLiteral("format"), QJsonObject{ { QStringLiteral("rateHz"), rateHz },
                                                          { QStringLiteral("channels"), channels } });
        std::printf("measure-spike: capturing %d Hz, %d channel(s) from \"%s\" for %d s; nothing is played\n", rateHz,
                    channels, qPrintable(name), seconds);
        std::fflush(stdout);
        QTimer::singleShot(seconds * 1000, &capture, &AudioCapture::stop);
    });
    QObject::connect(&capture, &AudioCapture::block, &app, [&](const dspi_capture::Block &b) {
        if (!host.isValid()) host.start();
        stats.add(b, double(host.nsecsElapsed()) / 1e9);
    });
    QObject::connect(&capture, &AudioCapture::failed, &app, [&](dspi_capture::Failure f, const QString &detail) {
        failure = QString::fromLatin1(failureName(f)) + QStringLiteral(": ") + detail;
        code = 1;
    });
    QObject::connect(&capture, &AudioCapture::stopped, &app, [&] { app.quit(); });

    if (!capture.start(inputId)) return 1;
    app.exec();

    const int rate = doc.value(QStringLiteral("format")).toObject().value(QStringLiteral("rateHz")).toInt();
    doc.insert(QStringLiteral("blocks"), double(stats.blocks));
    doc.insert(QStringLiteral("frames"), double(stats.frames));
    doc.insert(QStringLiteral("gaps"), stats.gaps);
    doc.insert(QStringLiteral("repeats"), stats.repeats);
    doc.insert(QStringLiteral("clippedBlocks"), double(stats.clippedBlocks));
    doc.insert(QStringLiteral("maxSourceLeadFrames"), double(stats.maxLead));
    doc.insert(QStringLiteral("clock"), stats.clock(rate));
    if (!failure.isEmpty()) doc.insert(QStringLiteral("failure"), failure);
    if (!writeJson(path, doc)) {
        std::fprintf(stderr, "measure-spike: cannot write %s\n", qPrintable(path));
        return 1;
    }
    std::printf("measure-spike: %lld blocks, %lld gap(s), %lld repeat(s), %lld clipped block(s) -> %s\n",
                static_cast<long long>(stats.blocks), static_cast<long long>(stats.gaps.size()),
                static_cast<long long>(stats.repeats.size()), static_cast<long long>(stats.clippedBlocks),
                qPrintable(path));
    if (!failure.isEmpty()) std::fprintf(stderr, "measure-spike: %s\n", qPrintable(failure));
    return code;
}

} // namespace

namespace MeasureSpike {

void addOptions(QCommandLineParser &args) {
    // Development switches: not translated, not in --help
    const QList<QCommandLineOption> options{
        QCommandLineOption(kInputs, QStringLiteral("Write the audio input list to <file> and exit."),
                           QStringLiteral("file")),
        QCommandLineOption(kSpike, QStringLiteral("Measurement spike: capture and write a report to <file>."),
                           QStringLiteral("file")),
        QCommandLineOption(kSpikeInput, QStringLiteral("The spike's audio input id (hex, from --audio-inputs)."),
                           QStringLiteral("id")),
        QCommandLineOption(kSpikeSeconds, QStringLiteral("The spike's capture length, 1 to 120 s."),
                           QStringLiteral("seconds"), QStringLiteral("30")),
    };
    for (QCommandLineOption o : options) {
        o.setFlags(QCommandLineOption::HiddenFromHelp);
        args.addOption(o);
    }
}

int runIfAsked(const QCommandLineParser &args, QApplication &app) {
    if (args.isSet(kInputs)) return runInputs(args.value(kInputs));
    if (!args.isSet(kSpike)) return -1;
    bool ok = false;
    const int seconds = args.value(kSpikeSeconds).toInt(&ok);
    if (!ok || seconds < 1 || seconds > 120) {
        std::fprintf(stderr, "measure-spike: --measure-spike-seconds is 1 to 120\n");
        return 2;
    }
    const QByteArray id = QByteArray::fromHex(args.value(kSpikeInput).toLatin1());
    return runSpike(args.value(kSpike), id, seconds, app);
}

} // namespace MeasureSpike
