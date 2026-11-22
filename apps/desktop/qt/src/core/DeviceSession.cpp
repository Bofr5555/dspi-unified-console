// -- WP MDQT1 -- One open device: its core's worker, callbacks and model
#include "DeviceSession.h"

#include "CoreApi.h"
#include "SessionModel.h"

namespace {
// How often a device that is not the active one is read
constexpr int kBackgroundMs = 1000;
} // namespace

DeviceSession::DeviceSession(FfiCore *core, bool main, QObject *parent)
    : QObject(parent), m_ffi(core), m_main(main), m_worker(core), m_model(new SessionModel(this)) {
    // The main session's are registered before any device opens, so no packet
    // is missed; another's right after its open (DeviceRegistry reads it then)
    dspi_set_notify_callback(m_ffi, &DeviceSession::notifyThunk, this);
    dspi_set_hotplug_callback(m_ffi, &DeviceSession::hotplugThunk, this);
    m_background.setInterval(kBackgroundMs);
    connect(&m_background, &QTimer::timeout, this, &DeviceSession::readBackground);
    connect(m_model, &SessionModel::applied, this, &DeviceSession::checkIdentity);
}

DeviceSession::~DeviceSession() { shutdown(); }

// The core's listener thread: schedule, never call back into the core
void DeviceSession::notifyThunk(void *self) {
    auto *session = static_cast<DeviceSession *>(self);
    if (session->m_notifyQueued.exchange(true)) return;
    QMetaObject::invokeMethod(session, &DeviceSession::notificationsWaiting, Qt::QueuedConnection);
}

// The polling thread, inside dspi_core_set_poll_hotplug: schedule only
void DeviceSession::hotplugThunk(uint8_t event, const char *serial, void *self) {
    auto *session = static_cast<DeviceSession *>(self);
    const QString s = serial ? QString::fromUtf8(serial) : QString();
    const int e = event;
    QMetaObject::invokeMethod(session, [session, e, s]() { emit session->deviceEvent(e, s); }, Qt::QueuedConnection);
}

void DeviceSession::shutdown() {
    if (!m_ffi) return;
    m_background.stop();
    // Unset before this goes: each waits for a call in progress. The core
    // outlives this (the set retires it, RA F2), so a late one is safe.
    dspi_set_notify_callback(m_ffi, nullptr, nullptr);
    dspi_set_hotplug_callback(m_ffi, nullptr, nullptr);
    m_worker.stop();
    m_ffi = nullptr;
}

QString DeviceSession::title() const {
    const QString label = m_row.value(QStringLiteral("label")).toString();
    // Several demos are told apart by the set's label (Demo 2)
    if (m_model->demo() || m_model->document()) return label.isEmpty() ? m_model->deviceTitle() : label;
    const QString title = m_model->deviceTitle();
    return title.isEmpty() ? label : title;
}

void DeviceSession::checkIdentity() {
    const DeviceSnapshot &snap = m_model->snapshot();
    const QString now = QStringLiteral("%1/%2/%3/%4/%5")
                            .arg(snap.generation)
                            .arg(snap.connected)
                            .arg(m_model->remote())
                            .arg(m_model->serial(), m_model->deviceTitle());
    if (now == m_identity) return;
    m_identity = now;
    emit identityChanged();
}

// -- WP MDQT2c --
void DeviceSession::setJob(const QString &controller, const QString &text) {
    if (m_jobs.value(controller) == text) return;
    if (text.isEmpty()) m_jobs.remove(controller);
    else m_jobs.insert(controller, text);
    emit jobsChanged();
}

void DeviceSession::setBackground(bool on) {
    if (on == m_background.isActive()) return;
    if (on) m_background.start();
    else m_background.stop();
    if (!on && m_clipHold != 0) {
        m_clipHold = 0;
        emit clipHoldChanged();
    }
}

void DeviceSession::readBackground() {
    if (m_reading || !m_ffi || !m_model->connected()) return;
    m_reading = true;
    m_worker.call(
        this,
        [](FfiCore *c) {
            SystemStatus status{};
            return dspi_fetch_status(c, &status) ? uint(status.clip_hold) : 0u;
        },
        [this](uint clip) {
            m_reading = false;
            ++m_backgroundReads;
            if (!m_background.isActive() || clip == m_clipHold) return;
            m_clipHold = clip;
            emit clipHoldChanged();
        });
}
