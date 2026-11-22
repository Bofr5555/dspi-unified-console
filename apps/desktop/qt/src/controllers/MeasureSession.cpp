// -- WP P5QUI -- Tools > Measure…, one device's (MeasureSession.h).
#include "MeasureSession.h"

#include "AudioCapture.h"
#include "ChannelListModel.h"
#include "CoreWorker.h"
#include "DeviceSession.h"
#include "FilesController.h"
#include "HistoryController.h"
#include "LinkController.h"
#include "LinkSession.h"
#include "MeasureController.h"
#include "NoticeCenter.h"
#include "SessionModel.h"
#include "SpeakersController.h"
#include "TuneSession.h"

#include "CoreApi.h"

#include <QCoreApplication>
#include <QDateTime>
#include <QDir>
#include <QElapsedTimer>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QLocale>
#include <QRegularExpression>
#include <QMetaObject>
#include <QSaveFile>
#include <QStandardPaths>
#include <QThread>
#include <QTimer>
#include <algorithm>
#include <cmath>

namespace {

// The simulated microphone's world (the demo's three-way room), and its rate
constexpr quint64 kSimSeed = 1;
constexpr int kSimRateHz = 48000;
// The longest reply a mutating call may give (a call that changes something
// is never called twice to fit its answer): a proposal with its curves, a
// measurement document (1 MiB at most) with room
constexpr int kOnceBytes = 8 << 20;
// The graph's span and how many points a curve keeps for drawing
constexpr double kGraphMinHz = 20.0;
constexpr double kGraphMaxHz = 20000.0;
constexpr int kGraphPoints = 480;
// The files a person may open: the core bounds them again
constexpr qint64 kMaxFileBytes = 4 << 20;

QString str(const QJsonObject &o, const char *key) { return o.value(QLatin1String(key)).toString(); }

bool isOk(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }

QByteArray compact(const QJsonObject &o) { return QJsonDocument(o).toJson(QJsonDocument::Compact); }

// Reads a text file of at most kMaxFileBytes; "" and `error` set otherwise
QByteArray readText(const QString &path, QString *error) {
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly)) {
        *error = QCoreApplication::translate("MeasureController", "%1 could not be read.").arg(QFileInfo(path).fileName());
        return {};
    }
    if (f.size() > kMaxFileBytes) {
        *error = QCoreApplication::translate("MeasureController", "%1 is too large to be a measurement file.")
                     .arg(QFileInfo(path).fileName());
        return {};
    }
    return f.readAll();
}

bool writeText(const QString &path, const QByteArray &text) {
    QSaveFile f(path);
    if (!f.open(QIODevice::WriteOnly)) return false;
    f.write(text);
    return f.commit();
}

// The graph's x (0 to 1) of a frequency
double xOf(double hz) { return std::log(hz / kGraphMinHz) / std::log(kGraphMaxHz / kGraphMinHz); }

// A frequency in words: "950 Hz", "1.02 kHz"
QString hzText(double f) {
    if (!std::isfinite(f)) return QStringLiteral("-");
    if (f >= 1000) return QCoreApplication::translate("MeasureController", "%1 kHz").arg(f / 1000.0, 0, 'f', f >= 10000 ? 1 : 2);
    return QCoreApplication::translate("MeasureController", "%1 Hz").arg(qRound(f));
}

QString roleWords(const QString &role) {
    if (role == QLatin1String("full")) return QCoreApplication::translate("MeasureController", "Full range");
    if (role == QLatin1String("sub")) return QCoreApplication::translate("MeasureController", "Subwoofer");
    if (role == QLatin1String("low")) return QCoreApplication::translate("MeasureController", "Low (woofer)");
    if (role == QLatin1String("mid")) return QCoreApplication::translate("MeasureController", "Mid");
    if (role == QLatin1String("high")) return QCoreApplication::translate("MeasureController", "High (tweeter)");
    return role;
}

QString placementWords(const QString &p) {
    if (p == QLatin1String("listeningPosition")) return QCoreApplication::translate("MeasureController", "Listening position");
    if (p == QLatin1String("multiPosition")) return QCoreApplication::translate("MeasureController", "Several positions");
    if (p == QLatin1String("nearField")) return QCoreApplication::translate("MeasureController", "Near-field");
    if (p == QLatin1String("headphones")) return QCoreApplication::translate("MeasureController", "Headphones on a coupler");
    return p;
}

QString couplerWords(const QString &c) {
    if (c == QLatin1String("iec60318_4")) return QCoreApplication::translate("MeasureController", "IEC 60318-4 ear simulator (711)");
    if (c == QLatin1String("flatPlate")) return QCoreApplication::translate("MeasureController", "Flat-plate coupler");
    if (c == QLatin1String("hats")) return QCoreApplication::translate("MeasureController", "HATS or another coupler with pinna");
    return c;
}

// The range of positions (several positions) or reseats (headphones); {0, 0} for none
std::pair<int, int> countRange(const QString &p) {
    if (p == QLatin1String("multiPosition")) return { 3, 9 };
    if (p == QLatin1String("headphones")) return { 1, 9 };
    return { 0, 0 };
}

// A call that changes something, never called twice to fit its answer:
// once, with room for any answer (an answer past it reads as none)
QByteArray once(const std::function<uint32_t(char *, uint32_t)> &call, int bytes = kOnceBytes) {
    QByteArray buf(bytes, '\0');
    const uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    buf.truncate(int(n) - 1);
    return buf;
}

// The state replies of the pushes and the ticks: a tick or a push is never
// repeated (a repeated block is a frame fault, S5)
constexpr int kStateBytes = 1 << 18;

// -- WP ROLEQ -- A speaker layout's (or a proposal's) outputs into the setup
// rows: role, speaker and group; `choose` also ticks exactly its outputs.
// Whether any row is in it.
bool layoutInto(QVariantList &rows, const QJsonArray &outs, bool choose) {
    bool any = false;
    for (QVariant &v : rows) {
        QVariantMap m = v.toMap();
        if (choose) m.insert(QStringLiteral("on"), false);
        for (const QJsonValue &x : outs) {
            const QJsonObject p = x.toObject();
            if (p.value(QLatin1String("output")).toInt() != m.value(QStringLiteral("output")).toInt()) continue;
            if (choose) m.insert(QStringLiteral("on"), true);
            m.insert(QStringLiteral("role"), str(p, "role"));
            m.insert(QStringLiteral("speaker"), str(p, "speaker"));
            m.insert(QStringLiteral("group"), p.value(QLatin1String("group")).toInt());
            any = true;
        }
        v = m;
    }
    return any;
}

} // namespace

// ---- The measurement loop --------------------------------------------------------------

void MeasureLoop::begin() {
    m_timer = new QTimer(this);
    m_timer->setInterval(kTickMs);
    connect(m_timer, &QTimer::timeout, this, &MeasureLoop::tick);
    m_timer->start();
}

void MeasureLoop::push(const dspi_capture::Block &b) {
    // -- WP ATQT -- the several-units session takes every block (it runs alone)
    if (system.load()) {
        const QByteArray st = once([&](char *buf, uint32_t l) {
            return dspi_measure_system_capture_push(m_core, uint64_t(b.firstFrame), b.samples.constData(),
                                                    uint32_t(b.samples.size()), uint32_t(b.rateHz), b.clipped, buf, l);
        }, kStateBytes);
        if (!isOk(MeasureSession::object(st))) emit pushRefused(st);
        return;
    }
    if (!session.load() && !live.load() && !tune.load() && !rolloff.load()) {   // -- WP MCP5 -- the search too
        // -- WP P5QUI2 -- the passive RTA's blocks (never beside a run: the
        // core ends the passive RTA when one starts)
        if (!passive.load()) return;
        const QByteArray st = once([&](char *buf, uint32_t l) {
            return dspi_measure_rta_passive_push(m_core, b.samples.constData(), uint32_t(b.samples.size()), uint32_t(b.rateHz), buf, l);
        }, kStateBytes);
        if (!isOk(MeasureSession::object(st))) emit pushRefused(st);
        return;
    }
    const QByteArray st = once([&](char *buf, uint32_t l) {
        return dspi_measure_capture_push(m_core, uint64_t(b.firstFrame), b.samples.constData(), uint32_t(b.samples.size()),
                                         uint32_t(b.rateHz), b.clipped, buf, l);
    }, kStateBytes);
    if (!isOk(MeasureSession::object(st))) emit pushRefused(st);
}

// Emits a state when it changed in more than its progress, or every 200 ms
// while only that moves
bool MeasureLoop::due(QByteArray *last, qint64 *at, const QByteArray &now, const char *key) {
    if (now == *last) return false;
    const QJsonObject a = MeasureSession::object(*last), b = MeasureSession::object(now);
    const QLatin1String k(key);
    const bool same = a.value(k) == b.value(k) && a.value(QLatin1String("active")) == b.value(QLatin1String("active")) &&
                      a.value(QLatin1String("running")) == b.value(QLatin1String("running")) &&
                      a.value(QLatin1String("issue")) == b.value(QLatin1String("issue"));
    const qint64 t = QDateTime::currentMSecsSinceEpoch();
    if (same && t - *at < 200) return false;
    *last = now;
    *at = t;
    return true;
}

void MeasureLoop::tick() {
    // -- WP ATQT -- the several-units session: its tick drives the hub's side
    if (system.load()) {
        const QByteArray st = once([&](char *b, uint32_t l) { return dspi_measure_system_tick(m_core, b, l); }, kStateBytes);
        const bool active = MeasureSession::object(st).value(QLatin1String("active")).toBool();
        if (!active) system.store(false);
        if (due(&m_lastSystem, &m_systemAt, st, "phase") || !active) emit systemState(st);
    }
    // -- WP P5QUI2 -- the passive RTA's frame, every 250 ms (a read)
    if (passive.load() && ++m_passiveTicks % 5 == 0) {
        const QByteArray f = MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_rta_passive_frame(m_core, b, l); }, 1 << 18);
        const QJsonObject st = MeasureSession::object(f).value(QLatin1String("state")).toObject();
        if (!st.value(QLatin1String("active")).toBool()) passive.store(false);
        emit passiveFrame(f);
    }
    if (tune.load()) {
        const QByteArray st = once([&](char *b, uint32_t l) { return dspi_measure_tune_tick(m_core, b, l); }, kStateBytes);
        const QJsonObject o = MeasureSession::object(st);
        const QString stage = str(o, "stage");
        // The guard runs while applied; the runs while they play
        const bool wanted = o.value(QLatin1String("running")).toBool() || stage == QLatin1String("sumCheck") ||
                            stage == QLatin1String("applied") || stage == QLatin1String("verifying") ||
                            stage == QLatin1String("verified");
        if (!wanted) tune.store(false);
        if (due(&m_lastTune, &m_tuneAt, st, "stage") || !wanted) emit tuneState(st);
    }
    if (live.load()) {
        const QByteArray st = once([&](char *b, uint32_t l) { return dspi_measure_live_tick(m_core, b, l); }, kStateBytes);
        const QJsonObject o = MeasureSession::object(st);
        const bool active = o.value(QLatin1String("active")).toBool();
        if (!active) live.store(false);
        if (due(&m_lastLive, &m_liveAt, st, "seq") || !active) emit liveState(st);
        const QByteArray key = QByteArray::number(o.value(QLatin1String("seq")).toInt()) + ',' +
                               QJsonDocument(QJsonArray{ o.value(QLatin1String("pending")) }).toJson(QJsonDocument::Compact) +
                               ',' + (active ? "1" : "0") + ',' + QByteArray::number(o.value(QLatin1String("runs")).toInt());
        if (key != m_liveKey || m_frameWanted) {
            m_liveKey = key;
            m_frameWanted = false;
            emit liveFrame(MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_live_frame(m_core, b, l); },
                                                    1 << 19));
        }
    } else if (m_frameWanted) {
        m_frameWanted = false;
        emit liveFrame(MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_live_frame(m_core, b, l); }, 1 << 19));
    }
    // -- WP MCP5 -- the low-end search: its tick drives its segments' sessions
    if (rolloff.load()) {
        const QByteArray st = once([&](char *b, uint32_t l) { return dspi_measure_rolloff_tick(m_core, b, l); }, kStateBytes);
        const bool running = MeasureSession::object(st).value(QLatin1String("running")).toBool();
        if (!running) rolloff.store(false);
        if (due(&m_lastRolloff, &m_rolloffAt, st, "segment") || !running) emit rolloffState(st);
    }
    if (session.load() && !live.load()) {
        // The tune's runs are the tune tick's; this reads the session's state
        const bool tuneRuns = tune.load();
        const QByteArray st = once([&](char *b, uint32_t l) {
            return tuneRuns ? dspi_measure_state(m_core, b, l) : dspi_measure_tick(m_core, b, l);
        }, kStateBytes);
        const bool active = MeasureSession::object(st).value(QLatin1String("active")).toBool();
        if (!active && !tuneRuns) session.store(false);
        if (due(&m_lastSession, &m_sessionAt, st, "phase") || !active) emit sessionState(st);
    }
}

void MeasureLoop::finish() {
    if (m_timer) m_timer->stop();
    // The session's own stops: one STOP_NOW where the generator plays
    if (live.exchange(false)) MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_live_stop(m_core, b, l); });
    if (tune.exchange(false)) MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_tune_stop(m_core, b, l); });
    if (rolloff.exchange(false)) MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_rolloff_stop(m_core, b, l); });   // -- WP MCP5 --
    if (session.exchange(false)) MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_stop(m_core, b, l); });
    if (passive.exchange(false)) MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_rta_passive_stop(m_core, b, l); });
    // -- WP ATQT -- the several-units session: one system.measure.abort
    if (system.exchange(false)) MeasureSession::reply([&](char *b, uint32_t l) { return dspi_measure_system_stop(m_core, b, l); });
}

// ---- Helpers ---------------------------------------------------------------------------

QByteArray MeasureSession::reply(const std::function<uint32_t(char *, uint32_t)> &call, int first) {
    QByteArray buf(first, '\0');
    uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0) return {};
    if (n > uint32_t(buf.size())) {
        // Reads only: a bigger buffer and once more
        if (n > uint32_t(64 << 20)) return {};
        buf = QByteArray(int(n), '\0');
        n = call(buf.data(), uint32_t(buf.size()));
        if (n == 0 || n > uint32_t(buf.size())) return {};
    }
    buf.truncate(int(n) - 1);
    return buf;
}

QJsonObject MeasureSession::object(const QByteArray &text) { return QJsonDocument::fromJson(text).object(); }

QString MeasureSession::failureText(const QJsonObject &reply) {
    if (reply.isEmpty()) return tr("The core did not answer.");
    const QString key = str(reply, "noticeKey");
    QString error = str(reply, "error");
    auto *notices = Registrar::instance<NoticeCenter>();
    // -- WP P5QUI2 -- a refusal that names its notice ("measure.tune.x: why",
    // the measurement's and the tune's refusals): that notice's title when the
    // table has a row for it, else the core's words without the key. Any such
    // key, so a new refusal needs no code here.
    // Review P5-E: after the error's own prefix ("invalid argument: measure.x:
    // why" is what the core says; without it this never matched), and only a
    // key the table has: any other text stays the core's, whole.
    static const QRegularExpression keyed(QStringLiteral("^(?:[a-z][a-z ]*: )?([a-z][a-z0-9_]*(?:\\.[a-z0-9_]+)+): (.+)$"),
                                          QRegularExpression::DotMatchesEverythingOption);
    if (const QRegularExpressionMatch m = keyed.match(error); m.hasMatch()) {
        QString why = m.captured(2).trimmed();
        if (!why.isEmpty()) why[0] = why[0].toUpper();
        const QString title = notices ? notices->row(m.captured(1)).value(QStringLiteral("title")).toString() : QString();
        //: A refusal of the core: %1 its notice's title, %2 the core's words
        if (!title.isEmpty()) return tr("%1: %2").arg(title, why);
    }
    if (notices) {
        const QString title = notices->row(key).value(QStringLiteral("title")).toString();
        if (!title.isEmpty() && !key.startsWith(QLatin1String("core."))) return error.isEmpty() ? title : tr("%1: %2").arg(title, error);
    }
    return error.isEmpty() ? tr("The call was refused.") : error;
}

QString MeasureSession::issueWords(const QJsonObject &i) const {
    if (i.isEmpty()) return {};
    const QString key = str(i, "key");
    QString msg = str(i, "message");
    if (!msg.isEmpty()) msg[0] = msg[0].toUpper();
    const QJsonValue out = i.value(QLatin1String("output"));
    const QString where = out.isDouble() ? outputName(out.toInt()) + QStringLiteral(": ") : QString();
    QString title;
    if (auto *notices = Registrar::instance<NoticeCenter>()) title = notices->row(key).value(QStringLiteral("title")).toString();
    //: A measurement's issue: %1 its title, %2 the output it is about, %3 the core's words
    if (!title.isEmpty()) return tr("%1. %2%3").arg(title, where, msg);
    return where + (msg.isEmpty() ? key : msg);
}

QString MeasureSession::outputName(int output) const {
    for (const QVariant &v : m_outputs) {
        const QVariantMap m = v.toMap();
        if (m.value(QStringLiteral("output")).toInt() == output) return m.value(QStringLiteral("label")).toString();
    }
    return tr("Output %1").arg(output + 1);
}

QVariantMap MeasureSession::series(const QJsonObject &c, const QString &kind, const QString &label, int colour) {
    const QJsonArray f = c.value(QLatin1String("freqsHz")).toArray();
    const QJsonArray m = c.value(QLatin1String("magDb")).toArray();
    const QJsonArray v = c.value(QLatin1String("valid")).toArray();
    const QJsonArray s = c.value(QLatin1String("spreadDb")).toArray();
    // The points in the graph's span, then every k-th of them
    QList<int> in;
    for (int i = 0; i < f.size() && i < m.size(); ++i) {
        const double hz = f.at(i).toDouble(), db = m.at(i).toDouble(NAN);
        if (hz >= kGraphMinHz && hz <= kGraphMaxHz && std::isfinite(db)) in.append(i);
    }
    const int step = std::max(1, int(std::ceil(double(in.size()) / kGraphPoints)));
    QVariantList pts, spread;
    double lo = INFINITY, hi = -INFINITY;
    for (int k = 0; k < in.size(); k += step) {
        const int i = in.at(k);
        const double db = m.at(i).toDouble();
        // A point is trusted when every point it stands for is
        bool ok = true;
        for (int j = k; j < std::min(int(in.size()), k + step); ++j)
            if (v.size() > in.at(j) && !v.at(in.at(j)).toBool(true)) ok = false;
        pts << xOf(f.at(i).toDouble()) << db << (ok ? 1 : 0);
        if (ok) {
            lo = std::min(lo, db);
            hi = std::max(hi, db);
        }
        if (s.size() > i) {
            const double w = std::fabs(s.at(i).toDouble()) / 2;
            spread << xOf(f.at(i).toDouble()) << db - w << db + w;
        }
    }
    QVariantMap out{ { QStringLiteral("kind"), kind },
                     { QStringLiteral("label"), label },
                     { QStringLiteral("colour"), colour },
                     { QStringLiteral("points"), pts } };
    if (!spread.isEmpty()) out.insert(QStringLiteral("spread"), spread);
    if (std::isfinite(lo)) {
        out.insert(QStringLiteral("lo"), lo);
        out.insert(QStringLiteral("hi"), hi);
    }
    return out;
}

QVariantMap MeasureSession::graph(const QVariantList &list, const QString &unit) {
    // The dB range that shows every trusted point: 6 dB steps, at least 36 dB
    double lo = INFINITY, hi = -INFINITY;
    for (const QVariant &v : list) {
        const QVariantMap m = v.toMap();
        if (m.value(QStringLiteral("kind")).toString() == QLatin1String("compare")) continue;
        if (!m.contains(QStringLiteral("lo"))) continue;
        lo = std::min(lo, m.value(QStringLiteral("lo")).toDouble());
        hi = std::max(hi, m.value(QStringLiteral("hi")).toDouble());
    }
    if (!std::isfinite(lo)) {
        for (const QVariant &v : list) {
            const QVariantMap m = v.toMap();
            if (!m.contains(QStringLiteral("lo"))) continue;
            lo = std::min(lo, m.value(QStringLiteral("lo")).toDouble());
            hi = std::max(hi, m.value(QStringLiteral("hi")).toDouble());
        }
    }
    double a = -30, b = 6;
    if (std::isfinite(lo)) {
        a = std::floor(lo / 6) * 6 - 6;
        b = std::ceil(hi / 6) * 6 + 6;
        if (b - a < 36) {
            const double mid = (a + b) / 2;
            a = std::floor((mid - 18) / 6) * 6;
            b = a + 36;
        }
    }
    return { { QStringLiteral("series"), list }, { QStringLiteral("yMin"), a }, { QStringLiteral("yMax"), b }, { QStringLiteral("unit"), unit } };
}

// ---- Life ------------------------------------------------------------------------------

MeasureSession::MeasureSession(DeviceSession *session, MeasureController *owner)
    : QObject(owner), m_session(session), m_owner(owner) {
    m_capture = new AudioCapture(this);
    if (FfiCore *h = session->handle()) {
        // -- WP MDQT2c -- a device's own thread; the controller's child, since
        // it may outlast this while its last call returns
        m_thread = new QThread(owner);
        m_thread->setObjectName(QStringLiteral("measure"));
        m_loop = new MeasureLoop(h);
        m_loop->moveToThread(m_thread);
        connect(m_thread, &QThread::started, m_loop, &MeasureLoop::begin);
        connect(m_thread, &QThread::finished, m_loop, &QObject::deleteLater);
        connect(m_loop, &MeasureLoop::sessionState, this, &MeasureSession::onSessionState);
        connect(m_loop, &MeasureLoop::liveState, this, &MeasureSession::onLiveState);
        connect(m_loop, &MeasureLoop::liveFrame, this, &MeasureSession::onLiveFrame);
        connect(m_loop, &MeasureLoop::pushRefused, this, [this](const QByteArray &json) { fail(failureText(object(json))); });
        connect(m_loop, &MeasureLoop::passiveFrame, this, &MeasureSession::onPassiveFrame);
        connect(m_loop, &MeasureLoop::rolloffState, this, &MeasureSession::onRolloffState);   // -- WP MCP5 --
        // The blocks go from the capture straight to the loop, in order
        connect(m_capture, &AudioCapture::block, m_loop, &MeasureLoop::push);
        m_thread->start();
    }
    connect(m_capture, &AudioCapture::started, this, [this](int rateHz, int, const QString &) {
        if (auto done = std::exchange(m_micOpening, nullptr)) done(true, rateHz);
        emit micChanged();
        setRunning();
    });
    connect(m_capture, &AudioCapture::failed, this, [this](dspi_capture::Failure f, const QString &) {
        QString why;
        switch (f) {
        case dspi_capture::Failure::NoDevice: why = tr("The microphone is not connected."); break;
        case dspi_capture::Failure::FormatUnsupported: why = tr("The microphone's audio format is not supported."); break;
        case dspi_capture::Failure::OpenFailed: why = tr("The microphone could not be opened."); break;
        case dspi_capture::Failure::DeviceLost: why = tr("The microphone went away: the measurement was stopped."); break;
        case dspi_capture::Failure::None: break;
        }
        if (!why.isEmpty()) fail(why);
        // -- WP P5QUI2 -- answered after AudioCapture's own `failed` returns:
        // the answer closes the capture (closeMic), and a stop from inside
        // `failed` deletes the reader thread its finish() then touches
        QMetaObject::invokeMethod(this, [this] {
            if (auto done = std::exchange(m_micOpening, nullptr)) done(false, 0);
            // A lost microphone ends what it fed (the core aborts on the gap too)
            if (running()) stop();
        }, Qt::QueuedConnection);
    });
    connect(m_capture, &AudioCapture::stopped, this, [this] {
        emit micChanged();
        setRunning();
    });
    connect(m_capture, &AudioCapture::inputsChanged, this, &MeasureSession::refreshInputs);
    connect(session->model(), &SessionModel::applied, this, &MeasureSession::onSnapshot);

    // The documents live under the app's data folder
    const QByteArray dir = (QStandardPaths::writableLocation(QStandardPaths::AppDataLocation) + QStringLiteral("/measurements")).toUtf8();
    session->worker()->post([dir](FfiCore *c) {
        QDir().mkpath(QString::fromUtf8(dir));
        reply([&](char *b, uint32_t l) { return dspi_measure_set_dir(c, dir.constData(), b, l); }, 1024);
    });
    QMetaObject::invokeMethod(this, &MeasureSession::refreshInputs, Qt::QueuedConnection);
    // Review P5-D: the hub's role decides who may play (playBlocker)
    QMetaObject::invokeMethod(this, [this] {
        if (LinkSession *l = link()) connect(l, &LinkSession::statusChanged, this, &MeasureSession::deviceChanged);
    }, Qt::QueuedConnection);
    m_tune = new TuneSession(this);
    onSnapshot();
}

// -- WP MDQT2c -- The device is closed (or the app quits, after shutdown()): the
// thread ends by itself once its last call returns; nothing waits for it here
MeasureSession::~MeasureSession() {
    m_micOpening = nullptr;
    if (m_session) m_session->setJob(QStringLiteral("MeasureController"), QString());
    disconnect(m_capture, nullptr, this, nullptr);
    if (m_capture->isRunning()) m_capture->stop();
    if (m_shutDown || !m_thread) return;
    m_shutDown = true;
    for (std::atomic<bool> *f : { &m_loop->live, &m_loop->tune, &m_loop->session, &m_loop->passive, &m_loop->system, &m_loop->rolloff }) f->store(false);
    connect(m_thread, &QThread::finished, m_thread, &QObject::deleteLater);
    m_thread->quit();
}

// -- WP MDQT2c -- Before the cores go (main.cpp shuts them down after the event
// loop): whatever this started is stopped first
void MeasureSession::shutdown() {
    if (m_shutDown) return;
    m_shutDown = true;
    m_micOpening = nullptr;
    if (m_capture && m_capture->isRunning()) m_capture->stop();
    if (m_thread) {
        if (m_loop && m_session && m_session->handle())
            QMetaObject::invokeMethod(m_loop, &MeasureLoop::finish, Qt::BlockingQueuedConnection);
        m_thread->quit();
        m_thread->wait();
    }
    m_loop = nullptr;
}

// ---- Which device (-- WP MDQT2c --) ----------------------------------------------------

SessionModel *MeasureSession::model() const { return m_session ? m_session->model() : nullptr; }

LinkSession *MeasureSession::link() const {
    auto *links = Registrar::instance<LinkController>();
    return links && m_session ? links->linkOf(m_session) : nullptr;
}

bool MeasureSession::isActive() const { return m_owner->session() == this; }

bool MeasureSession::capturing() const { return m_micOpening || (m_capture && m_capture->isRunning()); }

bool MeasureSession::underWay() const {
    return running() || capturing() || m_starting || m_liveStarting || m_passiveStarting;
}

void MeasureSession::leaving() {
    // What has an end runs on; what plays or listens until it is stopped does not
    const bool live = liveActive() || m_liveStarting, passive = m_passiveOn || m_passiveStarting;
    if (!live && !passive) return;
    ++m_stopEpoch;   // a start still on its way is withdrawn
    if (liveActive()) liveStop();
    if (m_passiveOn) passiveStop();
    closeMic();
}

void MeasureSession::entered() {
    refreshAccess();
    refreshInputs();
    reloadDocuments();
}

void MeasureSession::stopForClose() {
    if (gone()) return;
    if (underWay()) {
        ++m_stopEpoch;   // a start still on its way is withdrawn
        if (m_systemStop) m_systemStop();
        if (m_loop) {
            const bool live = m_loop->live.exchange(false), passive = m_loop->passive.exchange(false), system = m_loop->system.exchange(false);
            const bool rolloff = m_loop->rolloff.exchange(false);   // -- WP MCP5 --
            m_loop->session.store(false);
            const bool tuneRuns = m_tuneRunning;
            // On the worker, where the close (or the next device) is posted next
            m_session->worker()->post([live, passive, system, tuneRuns, rolloff](FfiCore *c) {
                if (rolloff) reply([&](char *b, uint32_t l) { return dspi_measure_rolloff_stop(c, b, l); });   // -- WP MCP5 --
                if (system) reply([&](char *b, uint32_t l) { return dspi_measure_system_stop(c, b, l); });
                if (passive) reply([&](char *b, uint32_t l) { return dspi_measure_rta_passive_stop(c, b, l); });
                if (live) reply([&](char *b, uint32_t l) { return dspi_measure_live_stop(c, b, l); });
                if (tuneRuns) reply([&](char *b, uint32_t l) { return dspi_measure_tune_stop(c, b, l); });
                reply([&](char *b, uint32_t l) { return dspi_measure_stop(c, b, l); });
            });
            m_session->drainFirst();
        }
        closeMic();
    }
    m_tune->stopForClose();
}

void MeasureSession::takeLayout(const QJsonObject &layout) {
    if (layout.isEmpty()) return;
    QVariantList rows = m_setupOutputs;
    layoutInto(rows, layout.value(QLatin1String("outputs")).toArray(), false);
    if (rows == m_setupOutputs) return;
    m_setupOutputs = rows;
    m_preflightDone = false;
    emit setupChanged();
    emit preflightChanged();
}

void MeasureSession::adoptInput(const QString &choice) {
    // The demo's simulated microphone stays the demo's choice
    if (m_micChoice == QLatin1String("sim") || m_micChoice == choice) return;
    m_micChoice = choice;
    emit micChanged();
}

void MeasureSession::adoptCalibration(const QByteArray &document, const QString &summary) {
    m_micDocument = document;
    m_micSummary = summary;
    emit micChanged();
}

void MeasureSession::setCalibration(const QByteArray &document, const QString &summary) {
    adoptCalibration(document, summary);
    emit calibrationChosen();
}

void MeasureSession::onLoop(QObject *ctx, std::function<QByteArray(FfiCore *)> job,
                               std::function<void(const QByteArray &)> done) {
    if (!m_loop || m_shutDown) return;
    track(1);
    QPointer<QObject> guard(ctx);
    QPointer<MeasureSession> self(this);
    MeasureLoop *loop = m_loop;
    QMetaObject::invokeMethod(m_loop, [loop, job = std::move(job), done = std::move(done), guard, self]() mutable {
        const QByteArray r = job(loop->core());
        QMetaObject::invokeMethod(qApp, [r, done = std::move(done), guard, self]() {
            if (!self) return;   // -- WP MDQT2c -- its device is closed
            self->track(-1);
            if (guard && done) done(r);
        }, Qt::QueuedConnection);
    });
}

void MeasureSession::track(int delta) {
    const bool was = busy();
    m_busy = std::max(0, m_busy + delta);
    if (was != busy()) emit busyChanged();
}

void MeasureSession::fail(const QString &text) {
    m_error = text;
    m_said.clear();
    emit errorChanged();
    if (!isActive()) m_owner->report(this, text, true);   // -- WP MDQT2c --
}

void MeasureSession::say(const QString &text) {
    m_said = text;
    m_error.clear();
    emit errorChanged();
}

void MeasureSession::clearError() {
    m_error.clear();
    m_said.clear();
    emit errorChanged();
}

void MeasureSession::storeCall(const QString &op, std::function<QByteArray(FfiCore *)> job,
                                  std::function<void(const QByteArray &)> done) {
    if (gone()) return;   // -- WP MDQT2c --
    track(1);
    m_session->worker()->call(this, std::move(job), [this, op, done = std::move(done)](QByteArray r) {
        track(-1);
        const QJsonObject o = object(r);
        // A document's text is no JSON reply with `ok`; a failure always is
        const bool failed = !o.isEmpty() && o.contains(QLatin1String("ok")) && !isOk(o);
        if (failed) fail(failureText(o));
        else if (r.isEmpty()) fail(tr("The core did not answer."));
        if (done && !failed && !r.isEmpty()) done(r);
        emit stored(op, !failed && !r.isEmpty());
    });
}

// ---- The device ------------------------------------------------------------------------

bool MeasureSession::connected() const { return m_session && model()->connected(); }

QString MeasureSession::playBlocker() const {
    if (!connected()) return tr("No device connected.");
    if (model()->readOnly())
        return tr("This connection is read-only: measuring plays the DSPi's generator, so only the passive microphone "
                  "RTA, the documents and saved measurements are offered.");
    // -- WP P5QUI2 -- the client read-only mode (the core refuses every write)
    if (m_clientReadOnly)
        return tr("The read-only mode is on: measuring plays the DSPi's generator, so only the passive microphone RTA, "
                  "the documents and saved measurements are offered.");
    // Review P5-D: a Viewer through a hub may not play the generator (the
    // core refuses it, S11): said here, the tabs that play are not offered
    if (LinkSession *l = link(); l && l->inSession() && l->role() == QLatin1String("viewer"))
        return tr("This connection has the hub's Viewer role: measuring needs Control. Only the passive microphone RTA, "
                  "the documents and saved measurements are offered.");
    return {};
}

// -- WP P5QUI2 -- the client read-only mode, read on the core worker (the
// core's lock may wait for a call in flight)
void MeasureSession::refreshAccess() {
    if (gone()) return;   // -- WP MDQT2c --
    m_session->worker()->call(this, [](FfiCore *c) { return dspi_client_read_only(c); }, [this](bool on) {
        if (on == m_clientReadOnly) return;
        m_clientReadOnly = on;
        emit deviceChanged();
    });
}

void MeasureSession::onSnapshot() {
    if (gone()) return;   // -- WP MDQT2c --
    QVariantList outs;
    ChannelListModel *ch = &m_channels;
    ch->update(model()->snapshot(), model()->platformName());
    for (int r = 0; r < ch->rowCount(); ++r) {
        if (ch->get(r, QStringLiteral("kind")).toString() != QLatin1String("output")) continue;
        outs.append(QVariantMap{ { QStringLiteral("output"), ch->get(r, QStringLiteral("number")) },
                                 { QStringLiteral("label"), ch->get(r, QStringLiteral("label")) } });
    }
    const QString serial = connected() ? model()->serial() : QString();
    const bool changedDevice = serial != m_serial;
    m_serial = serial;
    // -- WP MDQT2c -- another device on this session (Classic replaces it): the
    // placement chosen for the last one is not assumed for it (headphones are
    // measured much more quietly)
    if (!serial.isEmpty() && serial != m_lastSerial) {
        if (!m_lastSerial.isEmpty()) setPlacement(QString());
        m_lastSerial = serial;
        emit deviceSwitched();
    }
    if (outs != m_outputs || changedDevice) {
        m_outputs = outs;
        // The setup keeps its choices for outputs still there
        QVariantList kept;
        for (const QVariant &o : outs) {
            const int n = o.toMap().value(QStringLiteral("output")).toInt();
            QVariantMap row{ { QStringLiteral("output"), n },
                             { QStringLiteral("label"), o.toMap().value(QStringLiteral("label")) },
                             { QStringLiteral("on"), false },
                             { QStringLiteral("role"), QStringLiteral("full") },
                             { QStringLiteral("speaker"), QStringLiteral("L") },
                             { QStringLiteral("group"), 0 } };
            for (const QVariant &old : std::as_const(m_setupOutputs))
                if (old.toMap().value(QStringLiteral("output")).toInt() == n) {
                    row = old.toMap();
                    row.insert(QStringLiteral("label"), o.toMap().value(QStringLiteral("label")));
                }
            kept.append(row);
        }
        m_setupOutputs = kept;
        emit setupChanged();
    }
    emit deviceChanged();
    refreshAccess();
    if (changedDevice) {
        refreshSim();
        if (connected()) fillFromLayout();   // -- WP ROLEQ --
        reloadDocuments();
    }
}

void MeasureSession::refreshSim() {
    if (!m_loop) return;
    onLoop(this, [](FfiCore *c) { return reply([&](char *b, uint32_t l) { return dspi_measure_sim_info(c, b, l); }, 4096); },
           [this](const QByteArray &r) {
               const QJsonObject o = object(r);
               m_simAvailable = o.value(QLatin1String("available")).toBool();
               m_simOn = o.value(QLatin1String("on")).toBool();
               m_simLabel = str(o, "label");
               // The demo measures with its simulated microphone unless the person chose
               if (m_simAvailable && !m_micChosen) m_micChoice = QStringLiteral("sim");
               if (!m_simAvailable && m_micChoice == QLatin1String("sim")) m_micChoice.clear();
               emit micChanged();
           });
}

// ---- The microphone --------------------------------------------------------------------

void MeasureSession::setMicChoice(const QString &choice) {
    m_micChosen = true;
    if (choice == m_micChoice) return;
    m_micChoice = choice;
    emit micChanged();
    if (choice != QLatin1String("sim")) emit inputChosen(choice);   // -- WP MDQT2c --
}

bool MeasureSession::micOpen() const { return (m_capture && m_capture->isRunning()) || m_simOn; }

void MeasureSession::refreshInputs() {
    QVariantList list;
    for (const dspi_capture::InputDevice &in : AudioCapture::inputs())
        list.append(QVariantMap{ { QStringLiteral("id"), QString::fromLatin1(in.id.toHex()) },
                                 { QStringLiteral("name"), in.name },
                                 { QStringLiteral("isDefault"), in.isDefault } });
    if (list == m_inputs) return;
    m_inputs = list;
    emit micChanged();
}

void MeasureSession::openMic(std::function<void(bool, int)> done) {
    if (m_micChoice == QLatin1String("sim")) {
        if (m_capture->isRunning()) m_capture->stop();
        onLoop(this, [](FfiCore *c) { return reply([&](char *b, uint32_t l) { return dspi_measure_sim_set(c, true, kSimSeed, b, l); }, 4096); },
               [this, done](const QByteArray &r) {
                   const QJsonObject o = object(r);
                   m_simOn = o.value(QLatin1String("on")).toBool();
                   emit micChanged();
                   if (!isOk(o) || !m_simOn) fail(failureText(o));
                   done(isOk(o) && m_simOn, kSimRateHz);
               });
        return;
    }
    openCapture(std::move(done));
}

// -- WP P5QUI2 -- the chosen input (the system's default when the simulated
// microphone is chosen), the simulated one off first
void MeasureSession::openCapture(std::function<void(bool, int)> done) {
    // -- WP MDQT2c -- one microphone: a run on another device has it
    if (MeasureSession *other = m_owner->capturing(this)) {
        //: %1 is another open device
        fail(tr("The microphone is in use on %1. Stop what runs there first.").arg(other->session()->title()));
        done(false, 0);
        return;
    }
    const auto capture = [this, done] {
        if (m_capture->isRunning()) m_capture->stop();
        m_micOpening = done;
        const QByteArray id = m_micChoice == QLatin1String("sim") ? QByteArray() : QByteArray::fromHex(m_micChoice.toLatin1());
        if (!(m_captureSource ? m_capture->startWith(m_captureSource, id) : m_capture->start(id))) {
            m_micOpening = nullptr;
            fail(tr("The microphone could not be opened."));
            done(false, 0);
        }
    };
    if (!m_simOn) {
        capture();
        return;
    }
    onLoop(this, [](FfiCore *c) { return reply([&](char *b, uint32_t l) { return dspi_measure_sim_set(c, false, 0, b, l); }, 4096); },
           [this, capture](const QByteArray &) {
               m_simOn = false;
               emit micChanged();
               capture();
           });
}

void MeasureSession::closeMic() {
    m_micOpening = nullptr;
    if (m_capture->isRunning()) m_capture->stop();
    emit micChanged();
    setRunning();
}

void MeasureSession::chooseMicFile() {
    FilesController::pickFile(false, tr("Import a Microphone Calibration"),
                              tr("Calibration files (*.txt *.cal *.frd *.csv);;All files (*)"), QString(), this,
                              [this](const QString &path) { importMicFile(path); });
}

void MeasureSession::importMicFile(const QString &path) {
    QString error;
    const QByteArray text = readText(path, &error);
    if (!error.isEmpty()) {
        fail(error);
        emit stored(QStringLiteral("mic"), false);
        return;
    }
    const QByteArray file = QFileInfo(path).fileName().toUtf8();
    storeCall(QStringLiteral("mic"),
              [text, file](FfiCore *) { return once([&](char *b, uint32_t l) { return dspi_mic_import(nullptr, file.constData(), text.constData(), b, l); }); },
              [this](const QByteArray &doc) {
                  const QJsonObject d = object(doc);
                  const QJsonObject m = d.value(QLatin1String("mic")).toObject();
                  QStringList parts{ str(m, "name").isEmpty() ? tr("Microphone") : str(m, "name") };
                  if (m.value(QLatin1String("sensitivityDb")).isDouble())
                      parts << tr("sensitivity %1 dB").arg(m.value(QLatin1String("sensitivityDb")).toDouble(), 0, 'f', 1);
                  if (m.value(QLatin1String("angle")).toInt() == 90) parts << tr("90° calibration");
                  parts << (m.value(QLatin1String("approximate")).toBool(true) ? tr("dB SPL approximate (no calibrator step)")
                                                                                 : tr("dB SPL calibrated"));
                  setCalibration(doc, parts.join(QStringLiteral(", ")));
                  say(tr("Calibration imported: %1.").arg(m_micSummary));
                  fileImported(doc);
              });
}

// An imported calibration or target joins this console's library (the Documents tab)
void MeasureSession::fileImported(const QByteArray &doc) {
    storeCall(QStringLiteral("file"), [doc](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_measure_doc_put(c, doc.constData(), b, l); }); },
              [this](const QByteArray &) { reloadDocuments(); });
}

void MeasureSession::clearMic() {
    setCalibration({}, {});
}

// ---- The setup -------------------------------------------------------------------------

bool MeasureSession::setOutputField(int output, const QString &field, const QVariant &value) {
    for (QVariant &v : m_setupOutputs) {
        QVariantMap m = v.toMap();
        if (m.value(QStringLiteral("output")).toInt() != output) continue;
        if (m.value(field) == value) return false;
        m.insert(field, value);
        v = m;
        m_preflightDone = false;
        emit setupChanged();
        emit preflightChanged();
        return true;
    }
    return false;
}

void MeasureSession::setOutputOn(int output, bool on) { setOutputField(output, QStringLiteral("on"), on); }
// -- WP ROLEQ -- a role, speaker or group is written back to the speaker
// layout ("Saved to your speaker layout."), its confirmation as it was
void MeasureSession::setOutputRole(int output, const QString &role) {
    if (!setOutputField(output, QStringLiteral("role"), role)) return;
    if (auto *sc = Registrar::instance<SpeakersController>()) sc->writeBack(output, role);
}
void MeasureSession::setOutputSpeaker(int output, const QString &speaker) {
    const QString name = speaker.trimmed().left(16);
    if (!setOutputField(output, QStringLiteral("speaker"), name) || name.isEmpty()) return;
    if (auto *sc = Registrar::instance<SpeakersController>()) sc->writeBack(output, QString(), name);
}
void MeasureSession::setOutputGroup(int output, int group) {
    group = std::clamp(group, 0, 15);
    if (!setOutputField(output, QStringLiteral("group"), group)) return;
    if (auto *sc = Registrar::instance<SpeakersController>()) sc->writeBack(output, QString(), QString(), group);
}

void MeasureSession::setPlacement(const QString &p) {
    if (p == m_placement) return;
    m_placement = p;
    const auto r = countRange(p);
    if (r.first) m_count = std::clamp(m_count, r.first, r.second);
    m_preflightDone = false;
    emit setupChanged();
    emit preflightChanged();
}

void MeasureSession::setCount(int n) {
    if (n == m_count) return;
    m_count = n;
    emit setupChanged();
}

void MeasureSession::setCoupler(const QString &c) {
    if (c == m_coupler) return;
    m_coupler = c;
    m_preflightDone = false;
    emit setupChanged();
    emit preflightChanged();
}

void MeasureSession::setSweepMs(int ms) {
    if (ms == m_sweepMs) return;
    m_sweepMs = ms;
    emit setupChanged();
}

void MeasureSession::setPasses(int n) {
    if (n == m_passes) return;
    m_passes = n;
    emit setupChanged();
}

void MeasureSession::setRunName(const QString &n) {
    if (n == m_runName) return;
    m_runName = n;
    emit setupChanged();
}

QString MeasureSession::setupProblem() const {
    int on = 0;
    for (const QVariant &v : m_setupOutputs)
        if (v.toMap().value(QStringLiteral("on")).toBool()) ++on;
    if (on == 0) return tr("Choose at least one output to measure.");
    if (on > 16) return tr("At most 16 outputs a run.");
    // -- WP P5FOLLOW -- (coordinator, headphone safety) no placement is
    // assumed: the levels are the placement's, much lower for headphones
    if (m_placement.isEmpty())
        return tr("Choose the placement first: speakers or headphones on a coupler. Headphones are measured much more quietly.");
    const auto r = countRange(m_placement);
    if (r.first && (m_count < r.first || m_count > r.second))
        return m_placement == QLatin1String("headphones") ? tr("Reseats: 1 to 9.") : tr("Positions: 3 to 9.");
    if (m_sweepMs < 1000 || m_sweepMs > 15000) return tr("A sweep is 1 to 15 s.");
    if (m_passes < 2 || m_passes > 8) return tr("Passes: 2 to 8.");
    return {};
}

QByteArray MeasureSession::setupJson(int rateHz) const {
    QJsonArray outs;
    for (const QVariant &v : m_setupOutputs) {
        const QVariantMap m = v.toMap();
        if (!m.value(QStringLiteral("on")).toBool()) continue;
        outs.append(QJsonObject{ { QStringLiteral("output"), m.value(QStringLiteral("output")).toInt() },
                                 { QStringLiteral("role"), m.value(QStringLiteral("role")).toString() },
                                 { QStringLiteral("speaker"), m.value(QStringLiteral("speaker")).toString().trimmed() },
                                 { QStringLiteral("group"), m.value(QStringLiteral("group")).toInt() } });
    }
    const auto r = countRange(m_placement);
    QJsonObject s{ { QStringLiteral("kind"), QStringLiteral("walked") },
                   { QStringLiteral("outputs"), outs },
                   { QStringLiteral("placement"), m_placement },
                   { QStringLiteral("count"), r.first ? m_count : 0 },
                   { QStringLiteral("micDocument"), QString::fromUtf8(m_micDocument) },
                   { QStringLiteral("sweepMs"), m_sweepMs },
                   { QStringLiteral("passes"), m_passes },
                   { QStringLiteral("captureRateHz"), rateHz },
                   { QStringLiteral("name"), m_runName.trimmed() } };
    if (m_placement == QLatin1String("headphones")) s.insert(QStringLiteral("coupler"), m_coupler);
    // A short sweep gets the short gap (P5TM's rule): the default otherwise
    if (m_sweepMs < 2000) s.insert(QStringLiteral("gapMs"), 300);
    return compact(s);
}

void MeasureSession::proposeLayout() { fillSetup(false); }

void MeasureSession::fillFromLayout() { fillSetup(true); }

void MeasureSession::fillSetup(bool fromLayout) {
    onLoop(this,
           [fromLayout](FfiCore *c) {
               // -- WP ROLEQ -- the stored speaker layout first, when asked
               if (fromLayout) {
                   QByteArray stored = reply([&](char *b, uint32_t l) { return dspi_speaker_layout(c, b, l); });
                   const QJsonObject o = object(stored);
                   if (isOk(o) && !o.value(QLatin1String("outputs")).toArray().isEmpty()) return stored;
               }
               return reply([&](char *b, uint32_t l) { return dspi_measure_layout_proposal(c, b, l); });
           },
           [this](const QByteArray &r) {
               const QJsonObject o = object(r);
               if (!isOk(o)) return;   // a device without outputs fed: the setup stays as it is
               const bool any = layoutInto(m_setupOutputs, o.value(QLatin1String("outputs")).toArray(), true);
               // Nothing proposed: the first two outputs, full range
               if (!any)
                   for (int i = 0; i < m_setupOutputs.size() && i < 2; ++i) {
                       QVariantMap m = m_setupOutputs[i].toMap();
                       m.insert(QStringLiteral("on"), true);
                       m_setupOutputs[i] = m;
                   }
               m_preflightDone = false;
               emit setupChanged();
               emit preflightChanged();
           });
}

void MeasureSession::preflight() {
    const QString problem = setupProblem();
    if (!problem.isEmpty()) {
        fail(problem);
        return;
    }
    const QByteArray setup = setupJson(0);
    onLoop(this, [setup](FfiCore *c) { return reply([&](char *b, uint32_t l) { return dspi_measure_preflight(c, setup.constData(), b, l); }); },
           [this](const QByteArray &r) {
               const QJsonObject o = object(r);
               m_preflightLines.clear();
               if (!isOk(o)) {
                   m_preflightDone = false;
                   fail(failureText(o));
               } else {
                   for (const char *kind : { "blockers", "warnings" })
                       for (const QJsonValue &v : o.value(QLatin1String(kind)).toArray())
                           m_preflightLines.append(QVariantMap{
                               { QStringLiteral("kind"), QLatin1String(kind) == QLatin1String("blockers") ? QStringLiteral("blocker") : QStringLiteral("warning") },
                               { QStringLiteral("key"), str(v.toObject(), "key") },
                               { QStringLiteral("text"), issueWords(v.toObject()) } });
                   m_preflightDone = true;
                   m_preflightClear = o.value(QLatin1String("clear")).toBool();
               }
               emit preflightChanged();
           });
}

// ---- The run ---------------------------------------------------------------------------

void MeasureSession::start() {
    clearError();
    // -- WP P5QUI2 -- who may play first: a Viewer's setup does not matter
    if (!playBlocker().isEmpty()) {
        fail(playBlocker());
        return;
    }
    const QString problem = setupProblem();
    if (!problem.isEmpty()) {
        fail(problem);
        return;
    }
    // -- WP P5QUI2 -- said in words (running() covers it too)
    if (m_passiveOn || m_passiveStarting) {
        fail(tr("Stop the passive RTA first."));
        return;
    }
    if (running()) return;
    // Review P5-D: a Stop (or the window closed) while this start is on its
    // way withdraws it: nothing starts to play after the Stop
    const quint64 epoch = m_stopEpoch;
    m_starting = true;   // -- WP MDQT2c --
    // The live view forgets a finished run: none runs now
    openMic([this, epoch](bool ok, int rateHz) {
        if (!ok || epoch != m_stopEpoch) {
            m_starting = false;
            closeMic();
            return;
        }
        const QByteArray setup = setupJson(rateHz);
        onLoop(this,
               [setup](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_measure_start(c, setup.constData(), b, l); }); },
               [this, epoch](const QByteArray &r) {
                   m_starting = false;
                   const QJsonObject o = object(r);
                   if (!isOk(o)) {
                       closeMic();
                       fail(failureText(o));
                       return;
                   }
                   if (epoch != m_stopEpoch) {
                       m_loop->session.store(true);
                       stop();
                       return;
                   }
                   m_result.clear();
                   m_resultView.clear();
                   m_resultGraph.clear();
                   emit resultChanged();
                   m_tuneSession = false;
                   m_state = o;
                   m_wasActive = true;
                   m_loop->session.store(true);
                   emit stateChanged();
                   setRunning();
               });
    });
}

void MeasureSession::nextPosition() {
    onLoop(this, [](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_measure_next_position(c, b, l); }); },
           [this](const QByteArray &r) {
               const QJsonObject o = object(r);
               if (!isOk(o)) {
                   fail(failureText(o));
                   return;
               }
               m_state = o;
               m_loop->session.store(true);
               emit stateChanged();
           });
}

void MeasureSession::stop() {
    ++m_stopEpoch;   // review P5-D: a start still on its way is withdrawn
    // -- WP ATQT -- the several-units session too
    if (m_systemStop) m_systemStop();
    if (!m_loop) return;
    MeasureLoop *loop = m_loop;
    const bool tuneRuns = m_tuneRunning;
    // -- WP P5QUI2 -- a connection that may not play has no run to stop: it
    // sends nothing (as the web's), unless the core says a run is active
    const bool passiveOnly = !playBlocker().isEmpty() && !m_state.value(QLatin1String("active")).toBool() && !tuneRuns;
    const bool passive = m_passiveOn || m_passiveStarting;
    onLoop(this,
           [loop, tuneRuns, passiveOnly](FfiCore *c) {
               QByteArray live;
               // -- WP MCP5 -- the low-end search (its segment's session with it)
               if (loop->rolloff.exchange(false)) reply([&](char *b, uint32_t l) { return dspi_measure_rolloff_stop(c, b, l); });
               if (loop->passive.exchange(false)) reply([&](char *b, uint32_t l) { return dspi_measure_rta_passive_stop(c, b, l); });
               if (loop->live.exchange(false)) live = reply([&](char *b, uint32_t l) { return dspi_measure_live_stop(c, b, l); });
               // The tune's runs (a proposal and an applied tune stay: its guard runs on)
               if (tuneRuns) reply([&](char *b, uint32_t l) { return dspi_measure_tune_stop(c, b, l); });
               // Review P5-E: a run this loop was told of is stopped whatever the
               // page thinks (a start withdrawn while the connection became one
               // that may not play still had its run in the core)
               const bool ours = loop->session.exchange(false);
               const QByteArray st = reply(
                   [&](char *b, uint32_t l) { return passiveOnly && !ours ? dspi_measure_state(c, b, l) : dspi_measure_stop(c, b, l); });
               // -- WP P5QUI2 -- the live view's state after its stop too: the
               // loop ticks it no more, so this is its last word (before, a
               // live view stopped here stayed "active" on the screen)
               return QByteArray("[") + (st.isEmpty() ? QByteArray("{}") : st) + "," + (live.isEmpty() ? QByteArray("{}") : live) + "]";
           },
           [this, passive](const QByteArray &r) {
               const QJsonArray both = QJsonDocument::fromJson(r).array();
               if (passive) passiveEnded();
               const QJsonObject live = both.at(1).toObject();
               if (!live.isEmpty()) onLiveState(compact(live));
               else if (liveActive()) {
                   // Stopped by the core's own Stop (measure_stop ends the live view too)
                   m_live.insert(QStringLiteral("active"), false);
                   emit liveChanged();
               }
               onSessionState(compact(both.at(0).toObject()));
               closeMic();
           });
}

void MeasureSession::onSessionState(const QByteArray &json) {
    const QJsonObject o = object(json);
    if (o.isEmpty()) return;
    m_state = o;
    const bool active = o.value(QLatin1String("active")).toBool();
    const bool ended = m_wasActive && !active;
    m_wasActive = active;
    emit stateChanged();
    // -- WP MDQT2c -- a run on a device that is not shown says how it ended
    if (ended && !isActive() && !m_tuneRunning && !issueText().isEmpty()) m_owner->report(this, issueText(), true);
    // A live view's and a tune's runs are theirs: their results too
    if (ended && !liveActive() && !m_tuneRunning && !m_tuneSession) {
        if (o.value(QLatin1String("resultReady")).toBool()) takeResult();
        closeMic();
    }
    setRunning();
}

void MeasureSession::takeResult() {
    onLoop(this, [](FfiCore *c) { return reply([&](char *b, uint32_t l) { return dspi_measure_result(c, b, l); }, 1 << 21); },
           [this](const QByteArray &r) {
               const QJsonObject o = object(r);
               if (o.contains(QLatin1String("ok")) && !isOk(o)) return;
               setResultDocument(r, QStringLiteral("run"));
               say(tr("The measurement is complete: see Results."));
               if (!isActive()) m_owner->report(this, m_said, false);   // -- WP MDQT2c --
           });
}

QString MeasureSession::phase() const { return m_state.value(QLatin1String("phase")).toString(QStringLiteral("idle")); }

bool MeasureSession::measuring() const { return m_state.value(QLatin1String("active")).toBool() && !liveActive(); }

int MeasureSession::progress() const {
    const double p = m_state.value(QLatin1String("progress")).toDouble();
    return std::isfinite(p) ? int(std::lround(std::clamp(p, 0.0, 1.0) * 100)) : 0;
}

QString MeasureSession::phaseText() const {
    const QJsonObject &st = m_state;
    const QJsonValue outV = st.value(QLatin1String("output"));
    const QString out = outV.isDouble() ? outputName(outV.toInt()) : QString();
    const QJsonValue lv = st.value(QLatin1String("levelDb"));
    const QString level = lv.isDouble() ? tr(" at %1 dBFS").arg(lv.toDouble(), 0, 'f', 0) : QString();
    const int positions = st.value(QLatin1String("positions")).toInt();
    const int position = st.value(QLatin1String("position")).toInt();
    const QString pos = positions > 1 ? tr(" (position %1 of %2)").arg(position).arg(positions) : QString();
    const QString p = phase();
    if (p == QLatin1String("preRoll")) return tr("Listening to the room's noise (the generator is idle)%1.").arg(pos);
    if (p == QLatin1String("ramp"))
        return tr("Finding the level%1%2: quiet first, louder in steps%3.").arg(out.isEmpty() ? QString() : tr(" of %1").arg(out), level, pos);
    if (p == QLatin1String("run"))
        return tr("Measuring: run %1 of %2%3%4.")
            .arg(std::max(1, st.value(QLatin1String("run")).toInt()))
            .arg(std::max(1, st.value(QLatin1String("runs")).toInt()))
            .arg(level, pos);
    if (p == QLatin1String("analysing")) return tr("Analysing%1.").arg(pos);
    if (p == QLatin1String("waitPosition"))
        return tr("Move the microphone (or reseat the headphones) to position %1 of %2, then choose Next Position.").arg(position).arg(positions);
    if (p == QLatin1String("done")) return tr("The measurement is complete.");
    if (p == QLatin1String("aborted")) return tr("Stopped: nothing was kept.");
    if (p == QLatin1String("failed")) return tr("Stopped, with a question: nothing was kept.");
    return tr("Not measuring.");
}

QString MeasureSession::issueText() const { return issueWords(m_state.value(QLatin1String("issue")).toObject()); }

QString MeasureSession::timingText() const {
    const QJsonObject t = m_state.value(QLatin1String("timing")).toObject();
    if (t.isEmpty()) return {};
    if (t.value(QLatin1String("reliable")).toBool())
        return tr("Timing reliable: largest cycle residual %1 µs (bound %2 µs), microphone clock %3 ppm.")
            .arg(t.value(QLatin1String("maxResidualUs")).toDouble(), 0, 'f', 1)
            .arg(t.value(QLatin1String("residualBoundUs")).toDouble(), 0, 'f', 0)
            .arg(t.value(QLatin1String("ratioPpm")).toDouble(), 0, 'f', 1);
    QStringList reasons;
    for (const QJsonValue &r : t.value(QLatin1String("reasons")).toArray()) reasons << r.toString();
    return tr("Timing unreliable: the magnitudes are kept, but no delay, polarity or crossover change is offered from "
              "this measurement.") +
           (reasons.isEmpty() ? QString() : QStringLiteral(" ") + reasons.join(QStringLiteral("; ")) + QStringLiteral("."));
}

bool MeasureSession::timingReliable() const {
    return m_state.value(QLatin1String("timing")).toObject().value(QLatin1String("reliable")).toBool();
}

bool MeasureSession::running() const {
    // The simulated microphone may stay on: it hears nothing while no run plays
    return m_state.value(QLatin1String("active")).toBool() || liveActive() || m_tuneRunning || m_passiveOn ||
           m_systemRunning ||   // -- WP ATQT --
           m_rolloffRunning ||  // -- WP MCP5 --
           (m_capture && m_capture->isRunning());
}

QString MeasureSession::runningText() const {
    // -- WP MCP5 -- what an AI agent asked for says so
    if (!m_agentLabel.isEmpty()) {
        QString what;
        if (m_rolloffRunning) what = tr("Finding an output's low end with the DSPi's generator.");
        else if (m_tuneRunning) what = tr("The auto-tune's runs are playing the DSPi's generator.");
        else if (m_state.value(QLatin1String("active")).toBool()) what = tr("Measuring with the DSPi's generator.");
        //: %1 is "AI · " and the agent's name, %2 what plays
        if (!what.isEmpty()) return tr("Requested by %1: %2").arg(m_agentLabel, what);
    }
    if (m_systemRunning) return m_systemText;   // -- WP ATQT --
    if (m_passiveOn) return tr("The microphone is open for the passive RTA; nothing is played.");
    if (liveActive()) return tr("The live view is playing the DSPi's generator.");
    if (m_tuneRunning) return tr("The auto-tune's runs are playing the DSPi's generator.");
    if (m_state.value(QLatin1String("active")).toBool()) return tr("Measuring with the DSPi's generator.");
    if (m_capture && m_capture->isRunning()) return tr("The microphone is open.");
    return {};
}

// -- WP ATQT --
void MeasureSession::setSystemRun(bool running, const QString &text, std::function<void()> stop) {
    m_systemRunning = running;
    m_systemText = text;
    if (stop) m_systemStop = std::move(stop);
    setRunning();
}

void MeasureSession::setRunning() {
    const bool now = running();
    if (!now && !m_starting) m_agentLabel.clear();   // -- WP MCP5 -- an agent's request is over
    // -- WP MDQT2c -- the device's tab says it is busy
    if (m_session) m_session->setJob(QStringLiteral("MeasureController"), now ? runningText() : QString());
    if (now == m_wasRunning) {
        emit runningChanged();   // the words may have changed
        return;
    }
    m_wasRunning = now;
    emit runningChanged();
}

// ---- The result ------------------------------------------------------------------------

void MeasureSession::setResultDocument(const QByteArray &text, const QString &from) {
    m_result = text;
    m_resultFrom = from;
    m_resultView.clear();
    const QJsonObject doc = object(text);
    const QJsonObject meta = doc.value(QLatin1String("meta")).toObject();
    const QJsonObject m = doc.value(QLatin1String("measurement")).toObject();
    if (str(meta, "kind").trimmed().toLower() != QLatin1String("measurement") || m.isEmpty()) {
        m_result.clear();
        m_resultGraph.clear();
        m_curves = {};
        ++m_curvesAsk;
        emit resultChanged();
        return;
    }
    const QJsonObject place = m.value(QLatin1String("placement")).toObject();
    QString placeText = placementWords(str(place, "mode"));
    if (str(place, "mode") == QLatin1String("multiPosition") && place.value(QLatin1String("count")).toInt())
        placeText += tr(": %1").arg(place.value(QLatin1String("count")).toInt());
    if (str(place, "mode") == QLatin1String("headphones")) {
        if (!str(place, "coupler").isEmpty()) placeText += QStringLiteral(", ") + couplerWords(str(place, "coupler"));
        if (const int n = place.value(QLatin1String("reseats")).toInt()) placeText += tr(", %n reseat(s)", nullptr, n);
    }
    QStringList summary{ placeText };
    const int positions = m.value(QLatin1String("positions")).toInt(1);
    if (positions > 1) summary << tr("%1 positions averaged (the band is their spread)").arg(positions);
    const QJsonObject mic = m.value(QLatin1String("mic")).toObject();
    summary << (str(mic, "name").isEmpty() ? tr("Uncalibrated (relative dB)")
                                           : str(mic, "name") + (mic.value(QLatin1String("approximate")).toBool() ? tr(" (dB SPL approximate)") : QString()));
    const QJsonObject stim = m.value(QLatin1String("stimulus")).toObject();
    if (stim.value(QLatin1String("levelDb")).isDouble())
        summary << tr("played at %1 dBFS").arg(stim.value(QLatin1String("levelDb")).toDouble(), 0, 'f', 0);
    if (!str(m, "serial").isEmpty()) summary << str(m, "serial");
    const QJsonObject cap = m.value(QLatin1String("capture")).toObject();
    QVariant reliable;
    QString timing;
    if (str(stim, "kind") == QLatin1String("walked") && cap.value(QLatin1String("timingReliable")).isBool()) {
        reliable = cap.value(QLatin1String("timingReliable")).toBool();
        if (reliable.toBool()) {
            timing = cap.value(QLatin1String("maxResidualUs")).isDouble()
                         ? tr("Timing reliable: largest cycle residual %1 µs.").arg(cap.value(QLatin1String("maxResidualUs")).toDouble(), 0, 'f', 1)
                         : tr("Timing reliable.");
        } else {
            QStringList reasons;
            for (const QJsonValue &r : cap.value(QLatin1String("reasons")).toArray()) reasons << r.toString();
            timing = tr("Timing unreliable: the magnitudes are kept, but no delay, polarity or crossover change is "
                        "offered from this measurement.") +
                     (reasons.isEmpty() ? QString() : QStringLiteral(" ") + reasons.join(QStringLiteral("; ")) + QStringLiteral("."));
        }
    }
    QVariantList outs;
    int colour = 0;
    for (const QJsonValue &v : m.value(QLatin1String("outputs")).toArray()) {
        const QJsonObject o = v.toObject();
        if (!o.value(QLatin1String("output")).isDouble()) continue;
        const int n = o.value(QLatin1String("output")).toInt();
        outs.append(QVariantMap{ { QStringLiteral("output"), n },
                                 { QStringLiteral("label"), outputName(n) },
                                 { QStringLiteral("role"), roleWords(str(o, "role")) },
                                 { QStringLiteral("speaker"), str(o, "speaker") },
                                 { QStringLiteral("arrivalMs"), o.value(QLatin1String("arrivalMs")).toVariant() },
                                 { QStringLiteral("colour"), colour++ },
                                 { QStringLiteral("shown"), !m_hidden.contains(n) } });
    }
    m_resultView = QVariantMap{ { QStringLiteral("name"), str(meta, "name") },
                                { QStringLiteral("summary"), summary.join(QStringLiteral(" · ")) },
                                { QStringLiteral("timing"), timing },
                                { QStringLiteral("reliable"), reliable },
                                { QStringLiteral("outputs"), outs } };
    // -- WP P5QUI2 -- the graph is the core's curves (dspi_measure_doc_curves:
    // smoothed, the target levelled), drawn when they come
    m_curves = {};
    m_resultGraph.clear();
    emit resultChanged();
    requestCurves();
}

// -- WP P5QUI2 -- asks the core for the result's curves with the choices now;
// only the last ask's answer is drawn
void MeasureSession::requestCurves() {
    const quint64 ask = ++m_curvesAsk;
    if (m_result.isEmpty()) return;
    const QByteArray doc = m_result;
    const QByteArray options = compact(QJsonObject{ { QStringLiteral("smoothing"), m_resultSmoothing },
                                                    { QStringLiteral("target"), m_resultTarget },
                                                    { QStringLiteral("targetDocument"), QString::fromUtf8(m_targetDocument) } });
    if (gone()) return;   // -- WP MDQT2c --
    track(1);
    m_session->worker()->call(this,
                               [doc, options](FfiCore *) {
                                   return reply([&](char *b, uint32_t l) { return dspi_measure_doc_curves(doc.constData(), options.constData(), b, l); }, 1 << 20);
                               },
                               [this, ask](QByteArray r) {
                                   track(-1);
                                   if (ask != m_curvesAsk) return;
                                   const QJsonObject o = object(r);
                                   if (!isOk(o)) {
                                       fail(failureText(o));
                                       m_curves = {};
                                   } else {
                                       m_curves = o;
                                   }
                                   buildResultGraph();
                                   emit resultChanged();
                               });
}

// The graph of the core's curves: each shown output's measured response
// (its spread as a band) and, when asked, its levelled target (dashed)
void MeasureSession::buildResultGraph() {
    QVariantList list;
    bool spl = false;
    int colour = 0;
    for (const QJsonValue &v : m_curves.value(QLatin1String("outputs")).toArray()) {
        const QJsonObject o = v.toObject();
        const int n = o.value(QLatin1String("output")).toInt();
        const int c = colour++;
        if (m_hidden.contains(n)) continue;
        QJsonObject measured = o.value(QLatin1String("measured")).toObject();
        if (measured.isEmpty()) continue;
        if (!o.value(QLatin1String("spreadDb")).toArray().isEmpty()) measured.insert(QStringLiteral("spreadDb"), o.value(QLatin1String("spreadDb")));
        spl = spl || measured.value(QLatin1String("spl")).toBool();
        list.append(series(measured, QStringLiteral("measured"), outputName(n), c));
        const QJsonObject target = o.value(QLatin1String("target")).toObject();
        if (!target.isEmpty()) list.append(series(target, QStringLiteral("target"), tr("Target for %1").arg(outputName(n)), c));
    }
    m_resultGraph = m_curves.isEmpty() ? QVariantMap() : graph(list, spl ? tr("dB SPL") : tr("dB (relative)"));
}

int MeasureSession::resultCurvesSmoothing() const {
    return m_curves.isEmpty() ? -1 : m_curves.value(QLatin1String("smoothing")).toInt();
}

void MeasureSession::setResultSmoothing(int n) {
    n = std::clamp(n, 0, 48);
    if (n == m_resultSmoothing) return;
    m_resultSmoothing = n;
    emit resultChanged();
    requestCurves();
}

void MeasureSession::setResultTarget(bool on) {
    if (on == m_resultTarget) return;
    m_resultTarget = on;
    emit resultChanged();
    requestCurves();
}

void MeasureSession::setOutputShown(int output, bool shown) {
    if (shown) m_hidden.removeAll(output);
    else if (!m_hidden.contains(output)) m_hidden.append(output);
    if (m_result.isEmpty()) return;
    // The outputs' rows and the graph again, from the curves the core gave
    QVariantList outs = m_resultView.value(QStringLiteral("outputs")).toList();
    for (QVariant &v : outs) {
        QVariantMap m = v.toMap();
        m.insert(QStringLiteral("shown"), !m_hidden.contains(m.value(QStringLiteral("output")).toInt()));
        v = m;
    }
    m_resultView.insert(QStringLiteral("outputs"), outs);
    buildResultGraph();
    emit resultChanged();
}

void MeasureSession::chooseResultFile() {
    FilesController::pickFile(false, tr("Open a Measurement"), tr("Measurements (*.dspipreset *.json);;All files (*)"), QString(),
                              this, [this](const QString &path) { openResultFile(path); });
}

void MeasureSession::openResultFile(const QString &path) {
    QString error;
    const QByteArray text = readText(path, &error);
    if (!error.isEmpty()) {
        fail(error);
        emit stored(QStringLiteral("open"), false);
        return;
    }
    const QString name = QFileInfo(path).fileName();
    // The core validates it (bounds, kind) before it is shown
    storeCall(QStringLiteral("open"), [text](FfiCore *) { return reply([&](char *b, uint32_t l) { return dspi_measure_doc_info(text.constData(), b, l); }, 4096); },
              [this, text, name](const QByteArray &r) {
                  if (str(object(r), "kind") != QLatin1String("measurement")) {
                      fail(tr("%1 is not a measurement document.").arg(name));
                      return;
                  }
                  setResultDocument(text, name);
                  clearError();
              });
}

void MeasureSession::saveResult() {
    const QByteArray text = m_result;
    if (text.isEmpty()) return;
    storeCall(QStringLiteral("save"), [text](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_measure_doc_put(c, text.constData(), b, l); }); },
              [this](const QByteArray &r) {
                  say(tr("Saved in this console as \"%1\".").arg(str(object(r), "name")));
                  reloadDocuments();
              });
}

void MeasureSession::saveResultToHub() {
    const QByteArray text = m_result;
    if (text.isEmpty()) return;
    storeCall(QStringLiteral("hub-save"),
              [text](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_hub_measure_doc_put(c, text.constData(), b, l); }); },
              [this](const QByteArray &r) {
                  const QJsonObject o = object(r);
                  const QString name = str(o.value(QLatin1String("doc")).toObject(), "name");
                  say(o.value(QLatin1String("created")).toBool(true) ? tr("Saved on the hub as \"%1\".").arg(name)
                                                                     : tr("The hub already holds \"%1\".").arg(name));
                  reloadDocuments();
              });
}

void MeasureSession::exportResult() {
    if (m_result.isEmpty()) return;
    QString base = resultName().isEmpty() ? tr("Measurement") : resultName();
    base.replace(QRegularExpression(QStringLiteral("[^\\w .-]+")), QStringLiteral("-"));
    FilesController::pickFile(true, tr("Export the Measurement"), tr("Measurements (*.dspipreset)"), base + QStringLiteral(".dspipreset"),
                              this, [this](const QString &path) { saveResultTo(path); });
}

void MeasureSession::saveResultTo(const QString &path) {
    const bool ok = writeText(path, m_result);
    if (ok) say(tr("Exported to %1.").arg(QFileInfo(path).fileName()));
    else fail(tr("%1 could not be written.").arg(QFileInfo(path).fileName()));
    emit stored(QStringLiteral("export"), ok);
}

void MeasureSession::exportRew(int output) {
    if (m_result.isEmpty()) return;
    QString base = (resultName().isEmpty() ? tr("Measurement") : resultName()) + QStringLiteral(" - ") + outputName(output);
    base.replace(QRegularExpression(QStringLiteral("[^\\w .-]+")), QStringLiteral("-"));
    FilesController::pickFile(true, tr("Export as REW Text"), tr("Text (*.txt)"), base + QStringLiteral(".txt"), this,
                              [this, output](const QString &path) { exportRewTo(output, path); });
}

void MeasureSession::exportRewTo(int output, const QString &path) {
    const QByteArray text = m_result;
    storeCall(QStringLiteral("rew"),
              [text, output](FfiCore *c) {
                  return reply([&](char *b, uint32_t l) { return dspi_measurement_export_rew(c, text.constData(), uint8_t(output), b, l); }, 1 << 18);
              },
              [this, path](const QByteArray &rew) {
                  if (writeText(path, rew)) say(tr("Exported to %1.").arg(QFileInfo(path).fileName()));
                  else fail(tr("%1 could not be written.").arg(QFileInfo(path).fileName()));
              });
}

void MeasureSession::clearResult() {
    m_result.clear();
    m_resultView.clear();
    m_resultGraph.clear();
    m_curves = {};
    ++m_curvesAsk;
    emit resultChanged();
}

// ---- The live view ---------------------------------------------------------------------

bool MeasureSession::liveActive() const { return m_live.value(QLatin1String("active")).toBool(); }

QString MeasureSession::liveText() const {
    if (!liveActive()) return tr("Not running.");
    const QJsonValue lv = m_live.value(QLatin1String("levelDb"));
    const QString level = lv.isDouble() ? tr(" at %1 dBFS").arg(lv.toDouble(), 0, 'f', 0) : QString();
    if (liveMode() == QLatin1String("pinkRta")) return tr("Pink noise%1: the microphone RTA.").arg(level);
    return tr("Sweeping%1: %n curve(s) measured.", nullptr, liveSeq()).arg(level);
}

QString MeasureSession::liveIssueText() const {
    if (liveActive()) return {};
    QJsonObject i = m_frame.value(QLatin1String("issue")).toObject();
    if (i.isEmpty()) i = m_live.value(QLatin1String("issue")).toObject();
    return issueWords(i);
}

void MeasureSession::liveStart(const QVariantMap &s) {
    clearError();
    if (!playBlocker().isEmpty()) {
        fail(playBlocker());
        return;
    }
    if (m_passiveOn || m_passiveStarting) {   // -- WP P5QUI2 --
        fail(tr("Stop the passive RTA first."));
        return;
    }
    // -- WP P5FOLLOW -- (coordinator, headphone safety) never a placement assumed
    if (s.value(QStringLiteral("placement")).toString().isEmpty()) {
        fail(tr("Choose the placement first: speakers or headphones on a coupler. Headphones are measured much more quietly."));
        return;
    }
    if (running()) return;
    m_liveSaid.clear();
    m_pending = {};
    m_proposal.clear();
    emit proposalChanged();
    // The live view forgets a finished run: its result is taken first (it is)
    QJsonObject setup = QJsonObject::fromVariantMap(s);
    setup.insert(QStringLiteral("micDocument"), QString::fromUtf8(m_micDocument));
    setup.insert(QStringLiteral("targetDocument"), QString::fromUtf8(m_targetDocument));
    if (str(setup, "placement") != QLatin1String("headphones")) setup.remove(QStringLiteral("coupler"));
    // Review P5-D: as `start`
    const quint64 epoch = m_stopEpoch;
    m_liveStarting = true;   // -- WP MDQT2c --
    openMic([this, setup, epoch](bool ok, int rateHz) mutable {
        if (!ok || epoch != m_stopEpoch) {
            m_liveStarting = false;
            closeMic();
            return;
        }
        setup.insert(QStringLiteral("captureRateHz"), rateHz);
        const QByteArray text = compact(setup);
        onLoop(this, [text](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_measure_live_start(c, text.constData(), b, l); }); },
               [this, epoch](const QByteArray &r) {
                   m_liveStarting = false;
                   const QJsonObject o = object(r);
                   if (!isOk(o)) {
                       closeMic();
                       fail(failureText(o));
                       return;
                   }
                   if (epoch != m_stopEpoch) {
                       m_loop->live.store(true);
                       stop();
                       return;
                   }
                   m_live = o;
                   m_frame = {};
                   m_loop->live.store(true);
                   m_loop->refreshFrame();
                   emit liveChanged();
                   setRunning();
               });
    });
}

void MeasureSession::liveStop() {
    MeasureLoop *loop = m_loop;
    if (!loop) return;
    onLoop(this,
           [loop](FfiCore *c) {
               loop->live.store(false);
               return reply([&](char *b, uint32_t l) { return dspi_measure_live_stop(c, b, l); });
           },
           [this](const QByteArray &r) {
               onLiveState(r);
               closeMic();
           });
}

void MeasureSession::onLiveState(const QByteArray &json) {
    const QJsonObject o = object(json);
    if (o.isEmpty() || !isOk(o)) return;
    const bool was = liveActive();
    m_live = o;
    emit liveChanged();
    if (was && !liveActive()) {
        // Ended by itself (an abort) or stopped: the microphone with it
        if (m_loop) m_loop->refreshFrame();
        closeMic();
    }
    setRunning();
}

void MeasureSession::onLiveFrame(const QByteArray &json) {
    const QJsonObject f = object(json);
    if (f.isEmpty() || !isOk(f)) return;
    m_frame = f;
    const QString mode = str(f, "mode");
    const QJsonObject measured = f.value(QLatin1String("measured")).toObject();
    const QJsonObject rta = f.value(QLatin1String("rta")).toObject();
    const QJsonObject target = f.value(QLatin1String("target")).toObject();
    QVariantList list;
    bool spl = measured.value(QLatin1String("spl")).toBool();
    const QJsonObject current = rta.value(QLatin1String("current")).toObject();
    if (mode == QLatin1String("pinkRta") && !current.isEmpty()) {
        list.append(series(current, QStringLiteral("measured"), tr("Microphone RTA")));
        spl = spl || current.value(QLatin1String("spl")).toBool();
    } else if (!measured.isEmpty()) {
        list.append(series(measured, QStringLiteral("measured"), tr("%1 measured").arg(outputName(f.value(QLatin1String("output")).toInt()))));
    }
    const QJsonObject held = rta.value(QLatin1String("held")).toObject();
    if (!held.isEmpty()) list.append(series(held, QStringLiteral("held"), tr("Held")));
    if (!target.isEmpty())
        list.append(series(target, QStringLiteral("target"), m_targetName.isEmpty() ? tr("Target") : tr("Target (%1)").arg(m_targetName)));
    // The pending change's prediction, else that of a change no sweep has measured yet
    QJsonObject pending = f.value(QLatin1String("pending")).toObject();
    QJsonObject predicted = pending.value(QLatin1String("predicted")).toObject();
    if (predicted.isEmpty()) predicted = m_pending.value(QLatin1String("predicted")).toObject();
    if (predicted.isEmpty()) predicted = f.value(QLatin1String("predicted")).toObject();
    if (!predicted.isEmpty()) list.append(series(predicted, QStringLiteral("predicted"), tr("Predicted")));
    m_liveGraph = graph(list, spl ? tr("dB SPL") : tr("dB (relative)"));
    const QJsonObject compare = rta.value(QLatin1String("compare")).toObject();
    m_compareGraph = compare.isEmpty() ? QVariantMap()
                                       : graph({ series(compare, QStringLiteral("compare"), tr("Now minus held")) }, tr("dB"));
    m_rtaHeld = !held.isEmpty();
    m_rtaReady = !current.isEmpty();
    m_rtaFrames = rta.value(QLatin1String("frames")).toInt();
    m_liveMeasured = !measured.isEmpty();
    // Where "cut this peak" starts: the highest trusted point from 100 Hz to 5 kHz
    m_livePeakHz = 0;
    {
        const QJsonArray fr = measured.value(QLatin1String("freqsHz")).toArray(), mg = measured.value(QLatin1String("magDb")).toArray(),
                         va = measured.value(QLatin1String("valid")).toArray();
        double best = -INFINITY;
        for (int i = 0; i < fr.size() && i < mg.size(); ++i) {
            const double hz = fr.at(i).toDouble(), db = mg.at(i).toDouble(NAN);
            if (hz < 100 || hz > 5000 || !std::isfinite(db) || (i < va.size() && !va.at(i).toBool(true))) continue;
            if (db > best) {
                best = db;
                m_livePeakHz = hz;
            }
        }
    }
    // A pending change the core holds is the one shown; one that changes
    // nothing stays shown (its reason) until the next
    if (!pending.isEmpty()) {
        if (pending.value(QLatin1String("id")) != m_pending.value(QLatin1String("id"))) showProposal(pending);
    } else if (!m_pending.isEmpty() && !m_pending.value(QLatin1String("changes")).toArray().isEmpty()) {
        // Applied or discarded elsewhere
        m_pending = {};
        m_proposal.clear();
        emit proposalChanged();
    }
    emit liveChanged();
}

void MeasureSession::propose(std::function<QByteArray(FfiCore *)> job) {
    m_liveSaid.clear();
    onLoop(this, std::move(job), [this](const QByteArray &r) {
        const QJsonObject o = object(r);
        if (!isOk(o)) {
            fail(failureText(o));
            return;
        }
        showProposal(o);
        if (m_loop) m_loop->refreshFrame();
    });
}

void MeasureSession::showProposal(const QJsonObject &o) {
    {
        m_pending = o;
        QStringList changes;
        for (const QJsonValue &c : o.value(QLatin1String("changes")).toArray()) changes << str(c.toObject(), "words");
        const double gb = o.value(QLatin1String("gainBeforeDb")).toDouble(), ga = o.value(QLatin1String("gainAfterDb")).toDouble();
        m_proposal = QVariantMap{
            { QStringLiteral("id"), o.value(QLatin1String("id")).toInt() },
            { QStringLiteral("label"), str(o, "label") },
            { QStringLiteral("changes"), changes },
            { QStringLiteral("gainText"), std::fabs(ga - gb) > 0.005 ? tr("Output gain %1 → %2 dB (headroom)").arg(gb, 0, 'f', 1).arg(ga, 0, 'f', 1) : QString() },
            { QStringLiteral("rmsText"), tr("Error to the target: %1 dB RMS now, %2 dB RMS predicted (dotted).")
                                             .arg(o.value(QLatin1String("rmsBeforeDb")).toDouble(), 0, 'f', 1)
                                             .arg(o.value(QLatin1String("rmsAfterDb")).toDouble(), 0, 'f', 1) },
            { QStringLiteral("reason"), changes.isEmpty() ? (o.value(QLatin1String("issue")).isObject()
                                                                 ? issueWords(o.value(QLatin1String("issue")).toObject())
                                                                 : tr("Nothing to change there."))
                                                          : QString() },
        };
        emit proposalChanged();
    }
}

void MeasureSession::cutPeak(double freqHz) {
    if (!(freqHz > 0) || !std::isfinite(freqHz)) {
        fail(tr("Type the frequency of the peak, or click it on the graph."));
        return;
    }
    const uint8_t out = uint8_t(liveOutput());
    propose([out, freqHz](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_measure_live_cut_peak(c, out, freqHz, b, l); }); });
}

void MeasureSession::fitRange(double f1Hz, double f2Hz) {
    if (!(f1Hz > 0) || !(f2Hz > f1Hz) || !std::isfinite(f2Hz)) {
        fail(tr("Type the range (from below to), or drag across it on the graph."));
        return;
    }
    const uint8_t out = uint8_t(liveOutput());
    propose([out, f1Hz, f2Hz](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_measure_live_fit_range(c, out, f1Hz, f2Hz, b, l); }); });
}

void MeasureSession::liveApply() {
    if (gone() || m_proposal.isEmpty() || m_proposal.value(QStringLiteral("changes")).toStringList().isEmpty()) return;
    const uint32_t id = m_proposal.value(QStringLiteral("id")).toUInt();
    const QString label = m_proposal.value(QStringLiteral("label")).toString();
    auto result = std::make_shared<QByteArray>();
    // A write: the device's one path (the snapshot and the history follow)
    track(1);
    model()->write(
        QStringLiteral("measure live apply"),
        [id, result](FfiCore *c) {
            *result = once([&](char *b, uint32_t l) { return dspi_measure_live_apply(c, id, b, l); });
            return isOk(object(*result));
        },
        [this, result, label](bool ok) {
            track(-1);
            if (!ok) {
                fail(failureText(object(*result)));
                emit stored(QStringLiteral("live-apply"), false);
                return;
            }
            m_pending = {};
            m_proposal.clear();
            m_liveSaid = tr("Applied: %1. Undo takes it back as one step.").arg(label);
            emit proposalChanged();
            onLiveState(*result);
            if (m_loop) m_loop->refreshFrame();
            emit stored(QStringLiteral("live-apply"), true);
        });
}

void MeasureSession::liveDiscard() {
    onLoop(this, [](FfiCore *c) { return reply([&](char *b, uint32_t l) { return dspi_measure_live_discard(c, b, l); }); },
           [this](const QByteArray &r) {
               m_pending = {};
               m_proposal.clear();
               emit proposalChanged();
               onLiveState(r);
               if (m_loop) m_loop->refreshFrame();
           });
}

void MeasureSession::liveHold(bool on) {
    onLoop(this, [on](FfiCore *c) { return reply([&](char *b, uint32_t l) { return dspi_measure_live_hold(c, on, b, l); }); },
           [this](const QByteArray &r) {
               const QJsonObject o = object(r);
               if (!isOk(o)) fail(failureText(o));
               if (m_loop) m_loop->refreshFrame();
           });
}

void MeasureSession::liveAveraging(const QString &averaging) {
    if (!liveActive() || liveMode() != QLatin1String("pinkRta")) return;
    const QByteArray word = averaging.toUtf8();
    onLoop(this, [word](FfiCore *c) { return reply([&](char *b, uint32_t l) { return dspi_measure_live_averaging(c, word.constData(), b, l); }); },
           [this](const QByteArray &r) {
               const QJsonObject o = object(r);
               if (!isOk(o)) fail(failureText(o));
           });
}

void MeasureSession::undo() {
    if (auto *history = Registrar::instance<HistoryController>()) history->undo();
}

void MeasureSession::chooseTargetFile(const QString &use, const QString &coupler) {
    FilesController::pickFile(false, tr("Import a Target Curve"), tr("Target files (*.txt *.csv *.frd);;All files (*)"), QString(), this,
                              [this, use, coupler](const QString &path) { importTargetFile(path, use, coupler); });
}

void MeasureSession::importTargetFile(const QString &path, const QString &use, const QString &coupler) {
    QString error;
    const QByteArray text = readText(path, &error);
    if (!error.isEmpty()) {
        fail(error);
        emit stored(QStringLiteral("target"), false);
        return;
    }
    const QString file = QFileInfo(path).fileName();
    const QByteArray name = QFileInfo(path).completeBaseName().toUtf8();
    const QByteArray u = (use == QLatin1String("headphones") ? QStringLiteral("headphones") : QStringLiteral("speakers")).toUtf8();
    const QByteArray cp = (use == QLatin1String("headphones") ? coupler : QString()).toUtf8();
    storeCall(QStringLiteral("target"),
              [text, name, u, cp](FfiCore *) {
                  return once([&](char *b, uint32_t l) { return dspi_target_import(name.constData(), text.constData(), u.constData(), cp.constData(), b, l); });
              },
              [this, file](const QByteArray &doc) {
                  m_targetDocument = doc;
                  m_targetName = file;
                  emit targetChanged();
                  requestCurves();   // -- WP P5QUI2 -- the result's target too
                  say(tr("Target imported: %1. It is used from the next live start and by the auto-tune.").arg(file));
                  fileImported(doc);
              });
}

void MeasureSession::clearTarget() {
    m_targetDocument.clear();
    m_targetName.clear();
    emit targetChanged();
    requestCurves();   // -- WP P5QUI2 --
}

// ---- The passive microphone RTA (-- WP P5QUI2 --) ---------------------------------------

QString MeasureSession::passiveBlocker() const {
    // Never beside this console's generator: it would show the test signal
    // as programme audio (the core refuses it too, `busy`)
    if (m_state.value(QLatin1String("active")).toBool() || liveActive() || m_tuneRunning)
        return tr("A measurement, the live view or the auto-tune plays the generator now: the passive RTA would show "
                  "the test signal.");
    return {};
}

void MeasureSession::passiveStart(const QString &averaging) {
    clearError();
    if (!passiveBlocker().isEmpty()) {
        fail(passiveBlocker());
        return;
    }
    if (m_passiveOn || m_passiveStarting || !m_loop) return;
    m_passiveAveraging = averaging;
    m_passiveStarting = true;
    emit passiveChanged();
    // Review P5-D's rule: a Stop (or the window closed) meanwhile withdraws it
    const quint64 epoch = m_stopEpoch;
    openCapture([this, epoch](bool ok, int rateHz) {
        if (!ok || epoch != m_stopEpoch) {
            m_passiveStarting = false;
            closeMic();
            emit passiveChanged();
            return;
        }
        const QByteArray setup = compact(QJsonObject{ { QStringLiteral("micDocument"), QString::fromUtf8(m_micDocument) },
                                                      { QStringLiteral("captureRateHz"), rateHz },
                                                      { QStringLiteral("averaging"), m_passiveAveraging } });
        onLoop(this,
               [setup](FfiCore *c) { return once([&](char *b, uint32_t l) { return dspi_measure_rta_passive_start(c, setup.constData(), b, l); }, kStateBytes); },
               [this, epoch](const QByteArray &r) {
                   m_passiveStarting = false;
                   const QJsonObject o = object(r);
                   if (!isOk(o)) {
                       closeMic();
                       fail(failureText(o));
                       emit passiveChanged();
                       return;
                   }
                   if (epoch != m_stopEpoch) {
                       m_loop->passive.store(true);
                       stop();
                       return;
                   }
                   m_passiveOn = true;
                   m_passiveFrames = int(o.value(QLatin1String("frames")).toDouble());
                   m_loop->passive.store(true);
                   emit passiveChanged();
                   setRunning();
               });
    });
}

void MeasureSession::passiveStop() {
    MeasureLoop *loop = m_loop;
    ++m_stopEpoch;   // a start on its way too
    if (!loop) return;
    onLoop(this,
           [loop](FfiCore *c) {
               loop->passive.store(false);
               return reply([&](char *b, uint32_t l) { return dspi_measure_rta_passive_stop(c, b, l); });
           },
           [this](const QByteArray &) {
               passiveEnded();
               closeMic();
           });
}

// The passive RTA is over (stopped, or the core ended it): its views go
void MeasureSession::passiveEnded() {
    m_passiveOn = false;
    m_passiveStarting = false;
    m_passiveGraph.clear();
    m_passiveCompareGraph.clear();
    m_passiveHeld = m_passiveReady = false;
    m_passiveFrames = 0;
    emit passiveChanged();
    setRunning();
}

void MeasureSession::onPassiveFrame(const QByteArray &json) {
    const QJsonObject f = object(json);
    if (!m_passiveOn || f.isEmpty() || !isOk(f)) return;
    const QJsonObject st = f.value(QLatin1String("state")).toObject();
    if (!st.value(QLatin1String("active")).toBool()) {
        // The core ended it (a run of this core started): the microphone with
        // it, unless that run uses it now
        passiveEnded();
        if (!passiveBlocker().isEmpty()) return;
        closeMic();
        return;
    }
    const QJsonObject rta = f.value(QLatin1String("rta")).toObject();
    const QJsonObject current = rta.value(QLatin1String("current")).toObject();
    const QJsonObject held = rta.value(QLatin1String("held")).toObject();
    const QJsonObject compare = rta.value(QLatin1String("compare")).toObject();
    QVariantList list;
    if (!current.isEmpty()) list.append(series(current, QStringLiteral("measured"), tr("Microphone RTA")));
    if (!held.isEmpty()) list.append(series(held, QStringLiteral("held"), tr("Held")));
    m_passiveGraph = graph(list, st.value(QLatin1String("spl")).toBool() ? tr("dB SPL") : tr("dBFS"));
    m_passiveCompareGraph = compare.isEmpty() ? QVariantMap()
                                              : graph({ series(compare, QStringLiteral("compare"), tr("Now minus held")) }, tr("dB"));
    m_passiveReady = !current.isEmpty();
    m_passiveHeld = st.value(QLatin1String("held")).toBool();
    m_passiveFrames = int(st.value(QLatin1String("frames")).toDouble());
    m_passiveAveraging = str(st, "averaging").isEmpty() ? m_passiveAveraging : str(st, "averaging");
    emit passiveChanged();
}

void MeasureSession::passiveHold(bool on) {
    if (!m_passiveOn) return;
    onLoop(this, [on](FfiCore *c) { return reply([&](char *b, uint32_t l) { return dspi_measure_rta_passive_hold(c, on ? 1 : 0, b, l); }); },
           [this](const QByteArray &r) {
               const QJsonObject o = object(r);
               if (!isOk(o)) fail(failureText(o));
               else {
                   m_passiveHeld = o.value(QLatin1String("held")).toBool();
                   emit passiveChanged();
               }
           });
}

void MeasureSession::setPassiveAveraging(const QString &averaging) {
    if (averaging == m_passiveAveraging && !m_passiveOn) return;
    m_passiveAveraging = averaging;
    emit passiveChanged();
    if (!m_passiveOn) return;
    // A JSON string: "fast", "slow", "infinite"
    const QByteArray word = QJsonDocument(QJsonArray{ averaging }).toJson(QJsonDocument::Compact).mid(1).chopped(1);
    onLoop(this, [word](FfiCore *c) { return reply([&](char *b, uint32_t l) { return dspi_measure_rta_passive_averaging(c, word.constData(), b, l); }); },
           [this](const QByteArray &r) {
               const QJsonObject o = object(r);
               if (!isOk(o)) fail(failureText(o));
               else {
                   m_passiveAveraging = str(o, "averaging");
                   emit passiveChanged();
               }
           });
}

// ---- The documents ---------------------------------------------------------------------

void MeasureSession::reloadDocuments() {
    storeCall(QStringLiteral("list"),
              [](FfiCore *c) { return reply([&](char *b, uint32_t l) { return dspi_measure_docs_merged(c, nullptr, nullptr, b, l); }); },
              [this](const QByteArray &r) {
                  const QJsonObject o = object(r);
                  QVariantList rows;
                  for (const QJsonValue &v : o.value(QLatin1String("entries")).toArray()) {
                      const QJsonObject e = v.toObject();
                      const QJsonObject d = e.value(QLatin1String("doc")).toObject();
                      const qint64 ms = qint64(d.value(QLatin1String("savedMs")).toDouble());
                      rows.append(QVariantMap{ { QStringLiteral("id"), str(d, "id") },
                                               { QStringLiteral("kind"), str(d, "kind") },
                                               { QStringLiteral("name"), str(d, "name") },
                                               { QStringLiteral("note"), str(d, "note") },
                                               { QStringLiteral("serial"), str(d, "serial") },
                                               { QStringLiteral("saved"), ms > 0 ? QLocale().toString(QDateTime::fromMSecsSinceEpoch(ms), QLocale::ShortFormat) : QString() },
                                               { QStringLiteral("where"), str(e, "where") } });
                  }
                  m_documents = rows;
                  m_hubDocuments = o.value(QLatin1String("hubListed")).toBool();
                  const QString key = str(o, "hubNoticeKey");
                  m_documentsNote.clear();
                  if (!key.isEmpty())
                      if (auto *notices = Registrar::instance<NoticeCenter>()) m_documentsNote = notices->line(key, key);
                  emit documentsChanged();
              });
}

void MeasureSession::openDocument(const QString &id, const QString &where) {
    const QByteArray i = id.toUtf8();
    const bool hub = where == QLatin1String("hub");
    storeCall(QStringLiteral("get"),
              [i, hub](FfiCore *c) {
                  return reply([&](char *b, uint32_t l) { return hub ? dspi_hub_measure_doc_get(c, i.constData(), b, l) : dspi_measure_doc_get(c, i.constData(), b, l); }, 1 << 21);
              },
              [this, hub](const QByteArray &text) {
                  const QJsonObject d = object(text);
                  const QString kind = str(d.value(QLatin1String("meta")).toObject(), "kind").trimmed().toLower();
                  const QString name = str(d.value(QLatin1String("meta")).toObject(), "name");
                  if (kind == QLatin1String("measurement")) {
                      setResultDocument(text, hub ? QStringLiteral("hub") : QStringLiteral("store"));
                      say(tr("Opened \"%1\": see Results.").arg(name));
                  } else if (kind == QLatin1String("mic")) {
                      setCalibration(text, name);
                      say(tr("Calibration in use: %1.").arg(name));
                  } else if (kind == QLatin1String("target")) {
                      m_targetDocument = text;
                      m_targetName = name;
                      emit targetChanged();
                      requestCurves();   // -- WP P5QUI2 --
                      say(tr("Target in use: %1.").arg(name));
                  }
              });
}

void MeasureSession::renameDocument(const QString &id, const QString &where, const QString &name) {
    const QByteArray i = id.toUtf8(), n = name.trimmed().toUtf8();
    if (n.isEmpty()) return;
    const bool hub = where == QLatin1String("hub");
    storeCall(QStringLiteral("rename"),
              [i, n, hub](FfiCore *c) {
                  return once([&](char *b, uint32_t l) { return hub ? dspi_hub_measure_doc_rename(c, i.constData(), n.constData(), b, l) : dspi_measure_doc_rename(c, i.constData(), n.constData(), b, l); });
              },
              [this](const QByteArray &) { reloadDocuments(); });
}

void MeasureSession::noteDocument(const QString &id, const QString &where, const QString &note) {
    const QByteArray i = id.toUtf8(), n = note.toUtf8();
    const bool hub = where == QLatin1String("hub");
    storeCall(QStringLiteral("note"),
              [i, n, hub](FfiCore *c) {
                  return once([&](char *b, uint32_t l) { return hub ? dspi_hub_measure_doc_note(c, i.constData(), n.constData(), b, l) : dspi_measure_doc_note(c, i.constData(), n.constData(), b, l); });
              },
              [this](const QByteArray &) { reloadDocuments(); });
}

void MeasureSession::deleteDocument(const QString &id, const QString &where) {
    const QByteArray i = id.toUtf8();
    const bool hub = where == QLatin1String("hub");
    storeCall(QStringLiteral("delete"),
              [i, hub](FfiCore *c) {
                  return once([&](char *b, uint32_t l) { return hub ? dspi_hub_measure_doc_delete(c, i.constData(), b, l) : dspi_measure_doc_delete(c, i.constData(), b, l); });
              },
              [this](const QByteArray &) { reloadDocuments(); });
}

// ---- An AI agent's request (-- WP MCP5 --) ------------------------------------------

void MeasureSession::startForAgent(const QJsonObject &ticket, std::function<void(bool, const QString &)> done) {
    const QString loopWord = ticket.value(QLatin1String("loop")).toString();
    const QString label = ticket.value(QLatin1String("client_name")).toString();
    if (gone()) {
        done(false, tr("That device is closing."));
        return;
    }
    if (!playBlocker().isEmpty()) {
        done(false, playBlocker());
        return;
    }
    if (running() || m_starting || m_liveStarting || m_passiveStarting) {
        done(false, tr("Something already runs on this device in the app; nothing was played."));
        return;
    }
    const QByteArray text = QJsonDocument(ticket).toJson(QJsonDocument::Compact);
    const auto begin = [text](int rateHz) {
        return std::function<QByteArray(FfiCore *)>([text, rateHz](FfiCore *c) {
            return once([&](char *b, uint32_t l) { return dspi_mcp_measure_begin(c, text.constData(), uint32_t(rateHz), b, l); });
        });
    };
    if (loopWord == QLatin1String("tune")) {
        // The tune's runs are the tune's (TuneSession), as its own Check and Verify
        m_agentLabel = label;
        m_tune->agentRuns(begin(0), [this, done](bool ok, const QString &why) {
            if (!ok) m_agentLabel.clear();
            setRunning();
            done(ok, why);
        });
        return;
    }
    if (loopWord != QLatin1String("session") && loopWord != QLatin1String("rolloff")) {
        done(false, tr("The Measure view cannot run that."));
        return;
    }
    clearError();
    const quint64 epoch = m_stopEpoch;
    m_starting = true;
    m_agentLabel = label;
    openMic([this, epoch, begin, loopWord, done](bool ok, int rateHz) {
        if (!ok || epoch != m_stopEpoch) {
            m_starting = false;
            m_agentLabel.clear();
            closeMic();
            done(false, ok ? tr("Stopped in the app before it began.")
                           : (m_error.isEmpty() ? tr("The microphone could not be opened.") : m_error));
            return;
        }
        onLoop(this, begin(rateHz), [this, epoch, loopWord, done](const QByteArray &r) {
            m_starting = false;
            const QJsonObject o = object(r);
            if (!isOk(o)) {
                m_agentLabel.clear();
                closeMic();
                const QString why = str(o, "error");
                fail(why);
                done(false, why);
                return;
            }
            if (loopWord == QLatin1String("rolloff")) {
                m_rolloffRunning = true;
                m_state = o.value(QLatin1String("session")).toObject();
                m_loop->rolloff.store(true);
            } else {
                m_result.clear();
                m_resultView.clear();
                m_resultGraph.clear();
                emit resultChanged();
                m_tuneSession = false;
                m_state = o;
                m_wasActive = true;
                m_loop->session.store(true);
            }
            emit stateChanged();
            setRunning();
            done(true, QString());
            // A Stop while it started: stopped at once
            if (epoch != m_stopEpoch) stop();
        });
    });
}

void MeasureSession::onRolloffState(const QByteArray &json) {
    const QJsonObject o = object(json);
    if (o.isEmpty()) return;
    m_state = o.value(QLatin1String("session")).toObject();
    emit stateChanged();
    if (o.value(QLatin1String("running")).toBool()) {
        setRunning();
        return;
    }
    // The search ended: its outcome in words
    m_rolloffRunning = false;
    closeMic();
    const QJsonObject res = o.value(QLatin1String("result")).toObject();
    QString text;
    if (!res.isEmpty()) {
        const int out = o.value(QLatin1String("output")).toInt();
        const QJsonValue hz = res.value(QLatin1String("rolloffHz"));
        //: %1 an output's name, %2 a frequency in Hz, %3 the dB below its loudest point
        text = hz.isDouble() ? tr("%1's low end: %3 dB down at %2 Hz.")
                                   .arg(outputName(out))
                                   .arg(qRound(hz.toDouble()))
                                   .arg(qRound(res.value(QLatin1String("stopDb")).toDouble()))
                             //: %1 an output's name, %2 a frequency in Hz
                             : tr("%1 still plays well at %2 Hz, where the search stops.")
                                   .arg(outputName(out))
                                   .arg(qRound(res.value(QLatin1String("lowestHz")).toDouble()));
        const QJsonObject note = res.value(QLatin1String("note")).toObject();
        if (!note.isEmpty()) text += QLatin1Char(' ') + issueWords(note);
        say(text);
    } else {
        text = issueWords(o.value(QLatin1String("issue")).toObject());
        if (!text.isEmpty()) fail(text);
    }
    if (!isActive() && !text.isEmpty()) m_owner->report(this, text, res.isEmpty());
    setRunning();
}
