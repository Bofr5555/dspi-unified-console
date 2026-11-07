#include "OutputLatencyController.h"
#include "DSPiBridge.h"
#include "Registrar.h"

#include <QElapsedTimer>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QRunnable>
#include <QThread>

static const Registrar::Controller<OutputLatencyController> latencyRegistered("outputLatency");

namespace {
// As the Mac's 40 x 100 ms (Commands.swift:757-762), held to a clock so slow
// reads can't stretch it
constexpr int kPollMs = 100;
constexpr int kSettleMs = 4000;
constexpr int kDefaultId = 5;   // OUTPUT_LATENCY_DEFAULT

template <class F>
QJsonObject coreJson(F call) {
    QByteArray buf(4 * 1024, '\0');
    uint32_t needed = call(buf.data(), uint32_t(buf.size()));
    if (needed > uint32_t(buf.size())) {
        buf.resize(int(needed));
        needed = call(buf.data(), uint32_t(buf.size()));
    }
    if (needed == 0 || needed > uint32_t(buf.size())) return {};
    buf.truncate(int(needed - 1));
    return QJsonDocument::fromJson(buf).object();
}

// "0.67 ms", "4 ms"
QString formatMs(int us) {
    if (us % 1000 == 0) return QStringLiteral("%1 ms").arg(us / 1000);
    return QStringLiteral("%1 ms").arg(us / 1000.0, 0, 'f', 2);
}
}

OutputLatencyController::OutputLatencyController(DSPiBridge *bridge, QObject *parent)
    : QObject(parent), m_bridge(bridge), m_core(bridge->core())
{
    m_worker.setMaxThreadCount(1);
    connect(m_bridge, &DSPiBridge::devicesChanged, this, &OutputLatencyController::sync);
    connect(m_bridge, &DSPiBridge::stateChanged, this, &OutputLatencyController::sync);
    sync();
}

OutputLatencyController::~OutputLatencyController() {
    m_generation++;
    m_worker.waitForDone();
}

OutputLatencyController::Status OutputLatencyController::readStatus(FfiCore *core) {
    const QJsonObject o = coreJson([core](char *b, uint32_t n) { return dspi_fetch_output_latency(core, b, n); });
    Status s;
    if (!o.value(QLatin1String("ok")).toBool()) {
        if (o.value(QLatin1String("code")).toString() == QLatin1String("unsupported")) s.result = Read::Unsupported;
        return s;
    }
    bool haveRequested = false, havePending = false;
    const QJsonArray fields = o.value(QLatin1String("fields")).toArray();
    for (const QJsonValue &v : fields) {
        const QJsonObject f = v.toObject();
        const QString name = f.value(QLatin1String("name")).toString();
        const QString value = f.value(QLatin1String("value")).toString();
        if (name == QLatin1String("requested_id")) s.requested = value.toInt(&haveRequested);
        else if (name == QLatin1String("pending")) s.pending = value.toInt(&havePending) != 0;
    }
    if (haveRequested && havePending) s.result = Read::Ok;
    return s;
}

OutputLatencyController::Reading OutputLatencyController::read(FfiCore *core) {
    Reading r;
    r.status = readStatus(core);
    if (r.status.result != Read::Ok) return r;
    const QJsonObject o = coreJson([core](char *b, uint32_t n) { return dspi_fetch_output_latency_options(core, b, n); });
    if (!o.value(QLatin1String("ok")).toBool()) return r;
    r.haveOptions = true;
    const QJsonArray list = o.value(QLatin1String("value")).toArray();
    for (const QJsonValue &v : list) {
        const QJsonObject e = v.toObject();
        Option option;
        option.id = e.value(QLatin1String("id")).toInt();
        option.latencyUs = e.value(QLatin1String("latencyUs")).toInt();
        option.effectiveUs = e.value(QLatin1String("effectiveUs")).toInt();
        option.available = e.value(QLatin1String("available")).toBool();
        r.options.push_back(option);
    }
    return r;
}

// A new device (or none) starts clean. A new source changes what each option gives.
void OutputLatencyController::sync() {
    const QString serial = m_bridge->connected() ? m_bridge->selectedSerial() : QString();
    const int source = serial.isEmpty() ? -1 : m_bridge->inputSource();
    const bool newDevice = serial != m_serial;
    const bool newSource = source != m_inputSource;
    if (!newDevice && !newSource) return;
    m_serial = serial;
    m_inputSource = source;
    if (newDevice) {
        m_generation++;
        clearStored();
    }
    if (!serial.isEmpty()) refresh();
}

void OutputLatencyController::clearStored() {
    m_supported = false;
    m_requested = 0;
    m_options.clear();
    m_held = false;
    m_draft = 0;
    emit storedChanged();
    emit shownChanged();
}

void OutputLatencyController::refresh() {
    if (m_serial.isEmpty()) return;
    const unsigned generation = m_generation;
    FfiCore *core = m_core;
    m_worker.start(QRunnable::create([this, core, generation]() {
        if (m_generation != generation) return;
        const Reading reading = read(core);
        QMetaObject::invokeMethod(this, [this, reading, generation]() {
            if (m_generation == generation) install(reading);
        }, Qt::QueuedConnection);
    }));
}

void OutputLatencyController::install(const Reading &reading) {
    switch (reading.status.result) {
    case Read::Failed:
        return;     // Keep what is known: the next read may answer
    case Read::Unsupported:
        if (m_supported) clearStored();
        return;
    case Read::Ok:
        break;
    }
    m_supported = true;
    m_requested = reading.status.requested;
    if (reading.haveOptions) m_options = reading.options;
    if (m_held && m_draft == m_requested) m_held = false;   // It landed after the wait ended
    emit storedChanged();
    emit shownChanged();
}

QVariantList OutputLatencyController::choices() const {
    QVariantList list;
    if (m_options.empty()) return list;
    for (const Option &o : m_options) {
        QString text = formatMs(o.latencyUs);
        if (o.id == kDefaultId) text += QStringLiteral(" (default)");
        list.append(QVariantMap { { "text", text }, { "value", o.id } });
    }
    if (!optionFor(shownId())) list.append(QVariantMap { { "text", QStringLiteral("-") }, { "value", shownId() } });
    return list;
}

const OutputLatencyController::Option *OutputLatencyController::optionFor(int id) const {
    for (const Option &o : m_options)
        if (o.id == id) return &o;
    return nullptr;
}

QString OutputLatencyController::caption() const {
    QString text = QStringLiteral("Lower values reduce the delay from input to output. Higher values may benefit sources exhibiting high jitter in the event of audio stutters.");
    const Option *o = optionFor(shownId());
    if (o && o->available && o->effectiveUs != o->latencyUs)
        text += QStringLiteral(" At the current sample rate the output buffer is smaller than this, so it runs as %1.").arg(formatMs(o->effectiveUs));
    text += QStringLiteral(" Values less than 2 ms have no effect upon USB Audio.");
    return text;
}

void OutputLatencyController::edit(int id) {
    if (m_applying || !optionFor(id)) return;
    m_held = id != m_requested;
    m_draft = m_held ? id : 0;
    emit shownChanged();
}

void OutputLatencyController::revert() {
    if (m_applying) return;     // What went out can't be called back; applied() settles it
    m_held = false;
    m_draft = 0;
    emit shownChanged();
}

void OutputLatencyController::save() {
    if (m_applying || !dirty() || !m_bridge->connected()) return;
    m_applying = true;
    emit applyingChanged();
    apply(m_draft);
}

// The one place the latency reaches the device
void OutputLatencyController::apply(int id) {
    const unsigned generation = m_generation;
    FfiCore *core = m_core;
    m_worker.start(QRunnable::create([this, core, id, generation]() {
        Reading reading;
        Outcome outcome = Outcome::NotSent;
        const bool hookOk = !m_writeHook || m_writeHook();
        if (hookOk && m_generation == generation && dspi_set_output_latency(core, uint8_t(id))) {
            outcome = Outcome::NotYetApplied;
            // A failed read is the flash window: keep waiting. Never sent again.
            Status last;
            QElapsedTimer wait;
            wait.start();
            while (m_generation == generation) {
                QThread::msleep(kPollMs);
                if (m_generation != generation) break;
                const Status s = readStatus(core);
                if (s.result == Read::Ok) last = s;
                if (s.result == Read::Ok && !s.pending) {
                    outcome = s.requested == id ? Outcome::Applied : Outcome::Kept;
                    break;
                }
                if (wait.hasExpired(kSettleMs)) break;
            }
            // The table's effective values follow the new option
            if (m_generation == generation) reading = read(core);
            if (reading.status.result != Read::Ok) reading.status = last;
        }
        QMetaObject::invokeMethod(this, [this, outcome, reading, generation]() {
            m_applying = false;
            emit applyingChanged();
            if (m_generation == generation) applied(outcome, reading);
        }, Qt::QueuedConnection);
    }));
}

void OutputLatencyController::applied(Outcome outcome, const Reading &reading) {
    install(reading);
    switch (outcome) {
    case Outcome::Applied:
        break;
    case Outcome::NotSent:
        emit saveFailed(QStringLiteral("The device did not take the latency."));
        break;
    case Outcome::NotYetApplied:
        emit saveFailed(QStringLiteral("The device has not applied the latency yet."));
        break;
    case Outcome::Kept:
        emit saveFailed(QStringLiteral("The device kept the previous latency."));
        break;
    }
}
