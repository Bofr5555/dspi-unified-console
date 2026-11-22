#ifndef CORE_H
#define CORE_H

#include <QObject>

#include "CoreWorker.h"
#include "DeviceSession.h"

struct FfiCore;
class DeviceRegistry;

// The core of the active device (-- WP MDQT1 -- a facade over DeviceRegistry's
// active session). Not seen by QML: screens reach the device through
// DeviceModel and the controllers, which post their calls to worker(). Ask for
// worker() and handle() at each call: they are another device's after
// activeChanged(). What must stay with one device takes its DeviceSession.
class Core : public QObject {
    Q_OBJECT
public:
    explicit Core(DeviceRegistry *registry, QObject *parent = nullptr);
    ~Core() override;

    CoreWorker *worker();
    // -- WP QP13 -- The handle, for the core's calls documented as safe from any
    // thread (the Link session's, dspi-capi's fns_link.rs), which LinkSession
    // runs on threads of its own so a dial never holds up this worker. Null
    // after shutdown().
    FfiCore *handle() const;
    // The main session's worker: what is the app's and no device's (sharing
    // through the app's own hub), whichever device is active
    CoreWorker *mainWorker();

    // Scans once, then polls for hot-plug.
    void startHotplug();

    // Stops the polls and the workers and frees the cores. Afterwards nothing
    // reaches a core again; safe to call twice.
    void shutdown();
    bool isShutDown() const;

    enum DeviceEvent { Arrived = DeviceSession::Arrived, Departed = DeviceSession::Departed };

signals:
    // A DSPi came (Arrived) or went (Departed), from a hot-plug poll.
    void deviceEvent(int event, const QString &serial);
    // Another device is the active one
    void activeChanged();

private:
    DeviceRegistry *m_registry;
};

#endif // CORE_H
