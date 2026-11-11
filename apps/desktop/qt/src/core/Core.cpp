#include "Core.h"

#include "CoreApi.h"

Core::Core(FfiCore *core, QObject *parent) : QObject(parent), m_ffi(core), m_worker(core) {
    // Registered on the GUI thread before any device opens, so no packet is missed
    dspi_set_notify_callback(m_ffi, &Core::notifyThunk, this);
    dspi_set_hotplug_callback(m_ffi, &Core::hotplugThunk, this);
    m_hotplugTimer.setInterval(500);
    connect(&m_hotplugTimer, &QTimer::timeout, this, &Core::pollHotplug);
}

Core::~Core() { shutdown(); }

// The core's listener thread: schedule, never call back into the core
void Core::notifyThunk(void *self) {
    auto *core = static_cast<Core *>(self);
    if (core->m_notifyQueued.exchange(true)) return;
    QMetaObject::invokeMethod(core, &Core::notificationsWaiting, Qt::QueuedConnection);
}

// The worker thread, inside dspi_poll_hotplug: schedule only
void Core::hotplugThunk(uint8_t event, const char *serial, void *self) {
    auto *core = static_cast<Core *>(self);
    const QString s = serial ? QString::fromUtf8(serial) : QString();
    const int e = event;
    QMetaObject::invokeMethod(core, [core, e, s]() { emit core->deviceEvent(e, s); }, Qt::QueuedConnection);
}

void Core::startHotplug() {
    if (!m_ffi || m_hotplugTimer.isActive()) return;
    m_worker.post([](FfiCore *c) { dspi_scan_devices(c, nullptr, 0); });
    m_hotplugTimer.start();
}

void Core::pollHotplug() {
    // One poll at a time: a replug's reconnect can take a few hundred ms
    if (m_pollInFlight.exchange(true)) return;
    m_worker.post([this](FfiCore *c) {
        dspi_poll_hotplug(c);
        m_pollInFlight.store(false);
    });
}

void Core::shutdown() {
    if (!m_ffi) return;
    m_hotplugTimer.stop();
    m_worker.stop();
    // Clears the callbacks first, waiting for one in progress (header contract)
    dspi_core_free(m_ffi);
    m_ffi = nullptr;
}
