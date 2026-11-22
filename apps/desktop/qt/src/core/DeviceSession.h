#ifndef DEVICESESSION_H
#define DEVICESESSION_H

#include <QHash>
#include <QObject>
#include <QString>
#include <QTimer>
#include <QVariantMap>
#include <atomic>

#include "CoreWorker.h"

struct FfiCore;
class SessionModel;

// -- WP MDQT1 -- One open device (MPLAN 1.2): its core of the set, that core's
// worker thread and callbacks, and the model of the device (SessionModel). Made
// by DeviceRegistry, which owns it; GUI thread only.
//
// The main session's core is the app's own: it is there with no device open
// too, and is never closed. Every other session goes when its device is closed.
//
// The callbacks run on the core's threads and only schedule work here:
// - notifications: notificationsWaiting() on the GUI thread, once per batch;
// - hot-plug: deviceEvent() on the GUI thread, from the set's polls.
//
// A session that is not the active one is read once a second, for its tab:
// the clip hold. What its device reports comes in as ever.
class DeviceSession : public QObject {
    Q_OBJECT
public:
    // `core` belongs to the set (dspi_core_set_main, dspi_core_set_core)
    DeviceSession(FfiCore *core, bool main, QObject *parent);
    ~DeviceSession() override;

    CoreWorker *worker() { return &m_worker; }
    // Null after shutdown()
    FfiCore *handle() const { return m_ffi; }
    SessionModel *model() const { return m_model; }
    bool isMain() const { return m_main; }
    // The worker runs nothing more and no callback comes: called before the
    // core goes. Waits for the job that runs now; safe to call twice.
    void shutdown();

    // The device's row in the set (dspi_core_set_list): { id, label, transport,
    // serial, state }; empty while the main session has no device
    QVariantMap row() const { return m_row; }
    QString id() const { return m_row.value(QStringLiteral("id")).toString(); }
    void setRow(const QVariantMap &row) { m_row = row; }
    // What people call it: the device's own name, else the set's label
    QString title() const;

    // What a controller keeps per device (the "state lives in the core or the
    // session" rule): its own map, by the controller's class name. Gone with
    // the session.
    QVariantMap &stash(const QString &controller) { return m_stash[controller]; }

    // The clip hold of the last background read (0 while it is the active one)
    uint clipHold() const { return m_clipHold; }
    // The registry: read it in the background, or not
    void setBackground(bool on);
    // Background reads done so far, for tests
    int backgroundReads() const { return m_backgroundReads; }

    enum DeviceEvent { Arrived = 0, Departed = 1 };
    // SessionModel, on the worker: packets from here on schedule another batch
    void takeNotifications() { m_notifyQueued.store(false); }

signals:
    void notificationsWaiting();
    // A DSPi came (Arrived) or went (Departed), from a hot-plug poll.
    void deviceEvent(int event, const QString &serial);
    // The device, or its title, is another than before
    void identityChanged();
    void clipHoldChanged();

private:
    static void notifyThunk(void *self);
    static void hotplugThunk(uint8_t event, const char *serial, void *self);
    void readBackground();
    void checkIdentity();

    FfiCore *m_ffi;
    const bool m_main;
    CoreWorker m_worker;
    SessionModel *m_model;
    QTimer m_background;
    bool m_reading = false;
    int m_backgroundReads = 0;
    uint m_clipHold = 0;
    QVariantMap m_row;
    QHash<QString, QVariantMap> m_stash;
    QString m_identity;
    std::atomic<bool> m_notifyQueued{ false };
};

#endif // DEVICESESSION_H
