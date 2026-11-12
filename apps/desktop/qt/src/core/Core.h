#ifndef CORE_H
#define CORE_H

#include <QObject>
#include <QTimer>
#include <atomic>

#include "CoreWorker.h"

struct FfiCore;

// Owns the core handle (one per process), its worker thread and the core's
// callbacks. Not seen by QML: screens reach the device through DeviceModel and
// the controllers, which post their calls to worker().
//
// The callbacks run on the core's threads and only schedule work here:
// - notifications: notificationsWaiting() on the GUI thread, once per batch;
// - hot-plug: deviceEvent() on the GUI thread, from the polls this object runs
//   on the worker every 500 ms once startHotplug() was called.
class Core : public QObject {
    Q_OBJECT
public:
    // Takes ownership of `core` (from dspi_core_new, not null).
    explicit Core(FfiCore *core, QObject *parent = nullptr);
    ~Core() override;

    CoreWorker *worker() { return &m_worker; }
    // -- WP QP13 -- The handle, for the core's calls documented as safe from any
    // thread (the Link session's, dspi-capi's fns_link.rs), which LinkController
    // runs on threads of its own so a dial never holds up this worker. Null
    // after shutdown().
    FfiCore *handle() const { return m_ffi; }

    // Scans once (the core announces every unit present as arrived otherwise),
    // then polls for hot-plug on the worker.
    void startHotplug();

    // Stops the polls and the worker and frees the core. Afterwards nothing
    // reaches the core again; safe to call twice.
    void shutdown();
    bool isShutDown() const { return m_ffi == nullptr; }

    enum DeviceEvent { Arrived = 0, Departed = 1 };

signals:
    // Packets are waiting: DeviceModel processes them on the worker.
    void notificationsWaiting();
    // A DSPi came (Arrived) or went (Departed), from a hot-plug poll.
    void deviceEvent(int event, const QString &serial);

private:
    static void notifyThunk(void *self);
    static void hotplugThunk(uint8_t event, const char *serial, void *self);
    void pollHotplug();

    FfiCore *m_ffi;
    CoreWorker m_worker;
    QTimer m_hotplugTimer;
    std::atomic<bool> m_notifyQueued{false};
    std::atomic<bool> m_pollInFlight{false};

    friend class DeviceModel;   // clears m_notifyQueued when it processes a batch
};

#endif // CORE_H
