#include "VregController.h"
#include "DSPiBridge.h"
#include "Registrar.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QMetaObject>
#include <QVariantList>
#include <utility>

static const Registrar::Controller<VregController> vregRegistered("vreg");

namespace {

// CG11: about once a second while the page shows
constexpr int kPollMs = 1000;

// A view from the core: on the worker, never the GUI thread
template <class F>
QByteArray coreText(F call) {
    QByteArray reply(4 * 1024, '\0');
    uint32_t needed = call(reply.data(), uint32_t(reply.size()));
    if (needed > uint32_t(reply.size())) {
        reply.resize(int(needed));
        needed = call(reply.data(), uint32_t(reply.size()));
    }
    reply.truncate(needed > 0 && needed <= uint32_t(reply.size()) ? int(needed) - 1 : 0);
    return reply;
}

} // namespace

VregController::VregController(DSPiBridge *bridge, QObject *parent)
    : QObject(parent), m_bridge(bridge), m_core(bridge->core())
{
    m_worker.setMaxThreadCount(1);
    m_pollTimer.setInterval(kPollMs);
    connect(&m_pollTimer, &QTimer::timeout, this, &VregController::poll);
    connect(m_bridge, &DSPiBridge::stateChanged, this, &VregController::sync);
    connect(m_bridge, &DSPiBridge::devicesChanged, this, &VregController::sync);
    connect(m_bridge, &DSPiBridge::writableChanged, this, &VregController::sync);
    // Before the next device opens, not after: nothing queued reaches it, the same
    // device included (no write is replayed), and it is polled once on arrival
    connect(m_bridge, &DSPiBridge::deviceChanging, this, [this]() {
        m_generation++;
        m_reconnecting = true;
        clearPending();
    });
    sync();
}

VregController::~VregController() {
    m_generation++;
    m_worker.waitForDone();
}

bool VregController::available() const {
    return m_view.value(QStringLiteral("available")).toBool() || phase() != QLatin1String("idle");
}

QString VregController::volts(int code) const {
    // dspi_proto::ext::vreg_mv: 0.85 V at 0x06, 50 mV a step, to 1.30 V at 0x0F
    if (code < 0x06 || code > 0x0F) return QStringLiteral("code 0x%1").arg(QString::number(code, 16).toUpper().rightJustified(2, QLatin1Char('0')));
    return QStringLiteral("%1 V").arg((850 + 50 * (code - 0x06)) / 1000.0, 0, 'f', 2);
}

void VregController::setWatching(bool on) {
    if (on == m_watching) return;
    m_watching = on;
    emit watchingChanged();
    syncTimer();
    if (on) poll();
}

void VregController::setPollPaused(bool on) {
    m_pollPaused = on;
    syncTimer();
}

void VregController::setViewOverride(const QByteArray &json) {
    m_viewOverride = json;
    if (!json.isEmpty()) setView(json, false);
    else poll();
}

// A device arriving is polled once, whether or not the page shows: a trial left
// running is found there (CG11). Another device starts clean; the same one keeps
// its view, as the core keeps its flow.
void VregController::sync() {
    const bool connected = m_bridge->connected();
    const QString serial = connected ? m_bridge->selectedSerial() : m_serial;
    const bool arrived = connected && (!m_connected || m_reconnecting || serial != m_serial);
    if (serial != m_serial) {
        m_generation++;
        m_serial = serial;
        m_view.clear();
        m_polled = false;
        m_lastError.clear();
        clearPending();
        emit viewChanged();
    }
    if (connected != m_connected) {
        m_connected = connected;
        if (!connected) clearPending();
    }
    m_reconnecting = false;
    syncTimer();
    if (arrived) poll();
}

void VregController::syncTimer() {
    if (m_watching && m_connected && !m_pollPaused) {
        if (!m_pollTimer.isActive()) m_pollTimer.start();
    } else {
        m_pollTimer.stop();
    }
}

void VregController::refresh() { poll(); }

// One poll at a time, in line with the requests on the worker. A reply from before
// a change of device is dropped.
void VregController::poll() {
    if (!m_connected || m_pollPending || !m_viewOverride.isEmpty()) return;
    m_pollPending = true;
    FfiCore *core = m_core;
    const quint64 generation = m_generation;
    m_worker.start([this, core, generation]() {
        const QByteArray reply = m_generation != generation ? QByteArray()
            : coreText([core](char *b, uint32_t n) { return dspi_core_vreg_cal_poll(core, b, n); });
        QMetaObject::invokeMethod(this, [this, generation, reply]() {
            m_pollPending = false;
            if (generation == m_generation && !reply.isEmpty()) setView(reply, true);
        }, Qt::QueuedConnection);
    });
}

void VregController::setView(const QByteArray &json, bool fromPoll) {
    QVariantMap view = QJsonDocument::fromJson(json).object().toVariantMap();
    if (!view.take(QStringLiteral("ok")).toBool()) return;  // Keep what is known
    const bool firstPoll = fromPoll && !m_polled;
    if (fromPoll) m_polled = true;
    if (view == m_view && !firstPoll) return;
    m_view = view;
    emit viewChanged();
}

void VregController::setLastError(const QString &error) {
    if (error == m_lastError) return;
    m_lastError = error;
    emit viewChanged();
}

void VregController::clearPending() {
    if (m_pending.isEmpty()) return;
    m_pending.clear();
    emit pendingChanged();
}

// Whether the view offers `code` now, and whether `flag` (needsWord, saveNeedsWord)
// says the word is needed for it
bool VregController::offered(int code, const char *flag, bool *needsWord) const {
    const QVariantList choices = m_view.value(QStringLiteral("choices")).toList();
    for (const QVariant &c : choices) {
        const QVariantMap choice = c.toMap();
        if (choice.value(QStringLiteral("code")).toInt() != code) continue;
        *needsWord = choice.value(QLatin1String(flag)).toBool();
        return true;
    }
    return false;
}

void VregController::openFlow() {
    setLastError(QString());
    apply({ QStringLiteral("open") });
}

void VregController::closeFlow() {
    clearPending();
    apply({ QStringLiteral("close") });
}

void VregController::keepTrial(int soakS) {
    clearPending();
    apply({ QStringLiteral("keep"), 0, soakS });
}

void VregController::endTrial() {
    clearPending();
    apply({ QStringLiteral("end") });
}

bool VregController::request(const QString &action, int value, int soakS) {
    clearPending();
    setLastError(QString());
    if (!m_connected || !m_bridge->writable() || !m_view.value(QStringLiteral("available")).toBool()) {
        setLastError(QStringLiteral("Core voltage can't be changed on this connection."));
        return false;
    }
    const int minSoakS = m_view.value(QStringLiteral("minSoakS")).toInt();
    const QString word = m_view.value(QStringLiteral("confirmationWord")).toString();
    bool needsWord = false;
    QString title, detail;
    if (action == QLatin1String("start")) {
        if (!m_view.value(QStringLiteral("canStart")).toBool() || !offered(value, "needsWord", &needsWord) || soakS < minSoakS) {
            setLastError(QStringLiteral("That trial isn't offered now."));
            return false;
        }
        title = QStringLiteral("Start a trial at %1 for %2 min").arg(volts(value)).arg(soakS / 60);
        detail = QStringLiteral("The device's record of hangs and failures is cleared first. The trial runs in RAM only: a hang or a reboot returns the device to its saved voltage. Nothing is saved until you choose to.");
    } else if (action == QLatin1String("commit")) {
        if (!m_view.value(QStringLiteral("canCommit")).toBool()) {
            setLastError(QStringLiteral("There is no passed trial to save."));
            return false;
        }
        value = m_view.value(QStringLiteral("targetVreg")).toInt();
        needsWord = m_view.value(QStringLiteral("commitNeedsWord")).toBool();
        title = QStringLiteral("Save %1 on the device").arg(volts(value));
        detail = QStringLiteral("The device starts at this voltage from now on, after every reboot and power cycle, until it is reset.");
    } else if (action == QLatin1String("set") || action == QLatin1String("trial")) {
        const bool set = action == QLatin1String("set");
        if (!offered(value, set ? "saveNeedsWord" : "needsWord", &needsWord)) {
            setLastError(QStringLiteral("%1 isn't offered on this device.").arg(volts(value)));
            return false;
        }
        title = set ? QStringLiteral("Save %1 on the device").arg(volts(value))
                    : QStringLiteral("Run %1 as a trial").arg(volts(value));
        detail = set ? QStringLiteral("The device starts at this voltage from now on, after every reboot and power cycle, until it is reset.")
                     : QStringLiteral("The trial runs in RAM only: a hang or a reboot returns the device to its saved voltage.");
    } else if (action == QLatin1String("reset")) {
        value = 0;
        title = QStringLiteral("Reset to the build default");
        detail = QStringLiteral("The device forgets the voltage saved on it and starts at its build default.");
    } else if (action == QLatin1String("auto")) {
        if (value != 0 && value != 1) return false;
        title = value ? QStringLiteral("Turn automatic raising on") : QStringLiteral("Turn automatic raising off");
        detail = value ? QStringLiteral("After two watchdog hangs in one power session the device raises its saved voltage one step by itself, up to 1.25 V.")
                       : QStringLiteral("The device no longer raises its saved voltage after repeated hangs.");
    } else if (action == QLatin1String("clear")) {
        title = QStringLiteral("Clear the device's record");
        detail = QStringLiteral("The hang count and the last trial failure are forgotten.");
    } else {
        return false;
    }
    m_pending = QVariantMap {
        { QStringLiteral("action"), action },
        { QStringLiteral("value"), value },
        { QStringLiteral("soakS"), soakS },
        { QStringLiteral("needsWord"), needsWord },
        { QStringLiteral("word"), word },
        { QStringLiteral("title"), title },
        { QStringLiteral("detail"), detail },
    };
    emit pendingChanged();
    return true;
}

void VregController::cancel() { clearPending(); }

bool VregController::confirm(const QString &typed) {
    if (m_pending.isEmpty()) return false;
    Call call;
    call.action = m_pending.value(QStringLiteral("action")).toString();
    call.value = m_pending.value(QStringLiteral("value")).toInt();
    call.soakS = m_pending.value(QStringLiteral("soakS")).toInt();
    call.sendsWord = m_pending.value(QStringLiteral("needsWord")).toBool();
    if (call.sendsWord) call.typed = typed.trimmed().toUtf8();
    clearPending();
    apply(call);
    return true;
}

// Every call to the core's flow, user requests and the abort alike. The view is
// read back from memory afterwards, whatever the answer, so the page shows where
// the flow is and the core's reason for a refusal.
void VregController::apply(const Call &call) {
    m_calls++;
    emit busyChanged();
    FfiCore *core = m_core;
    const quint64 generation = m_generation;
    m_worker.start([this, core, generation, call]() {
        int code = -1;  // not sent: the device changed while this waited, and the result is dropped
        QByteArray view;
        if (m_generation == generation) {
            const char *typed = call.sendsWord ? call.typed.constData() : nullptr;
            const auto value = uint8_t(call.value);
            const auto soak = uint32_t(qMax(0, call.soakS));
            const QString &a = call.action;
            if (a == QLatin1String("open")) code = dspi_core_vreg_cal_open(core);
            else if (a == QLatin1String("close")) code = dspi_core_vreg_cal_close(core);
            else if (a == QLatin1String("keep")) code = dspi_core_vreg_cal_keep(core, soak);
            else if (a == QLatin1String("end")) code = dspi_core_vreg_trial_end(core);
            else if (a == QLatin1String("start")) code = dspi_core_vreg_cal_start(core, value, soak, typed);
            else if (a == QLatin1String("commit")) code = dspi_core_vreg_cal_commit(core, typed);
            else if (a == QLatin1String("set") || a == QLatin1String("reset")) code = dspi_core_vreg_manual_set(core, value, typed);
            else if (a == QLatin1String("trial")) code = dspi_core_vreg_manual_trial(core, value, typed);
            else if (a == QLatin1String("auto")) code = dspi_core_vreg_manual_auto(core, value != 0);
            else if (a == QLatin1String("clear")) code = dspi_core_vreg_manual_clear(core);
            view = coreText([core](char *b, uint32_t n) { return dspi_core_vreg_cal_view(core, b, n); });
        }
        QMetaObject::invokeMethod(this, [this, generation, call, code, view]() {
            m_calls--;
            emit busyChanged();
            if (generation == m_generation) {
                if (m_viewOverride.isEmpty()) setView(view, false);
                if (code != 0) {
                    const QString why = QJsonDocument::fromJson(view).object().value(QStringLiteral("lastError")).toString();
                    setLastError(!why.isEmpty() ? why : QStringLiteral("The device did not take it (status %1).").arg(code));
                }
            }
            emit applied(call.action, code == 0);
        }, Qt::QueuedConnection);
    });
}
