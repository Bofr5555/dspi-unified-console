#ifndef DEVICEREGISTRY_H
#define DEVICEREGISTRY_H

#include <QJsonObject>
#include <QList>
#include <QObject>
#include <QString>
#include <QThreadPool>
#include <QTimer>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <atomic>
#include <functional>

class DeviceSession;
class QQmlEngine;
class QJSEngine;
struct FfiCoreSet;

// -- WP MDQT1 -- The open devices (MPLAN 1.2 item 5), over the core's CoreSet:
// a DeviceSession each, one of them active. GUI thread only; what blocks (an
// open, a hot-plug poll) runs on threads of its own. QML sees it as the
// singleton `DeviceRegistry`.
//
// The active session is what the app's base objects (Core, DeviceModel, Poller:
// facades over it) and so every controller show and write to. `activeChanged`
// is emitted with the new one in place and DeviceModel moved to it; the session
// left stays as it was, its jobs running on its own worker.
//
// There is always an active session: the main one, with no device, when none
// is open. A replaced device (Classic) is opened on the active session's own
// core, so a session may hold another device than before; its `id()` says which.
class DeviceRegistry : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // The open devices, the main session's first: [{ id, title, label,
    // transport, serial, state, active, clip, busy, job }]; `state` is "live",
    // "reconnecting" or "lost", `clip` whether one that is not active clips,
    // `busy` whether something runs on it (a measurement), `job` what, in words
    Q_PROPERTY(QVariantList devices READ devices NOTIFY devicesChanged)
    // The active device's id, "" with none
    Q_PROPERTY(QString activeId READ activeId NOTIFY devicesChanged)
    // Choosing a device keeps the others open (the Modern layout's tabs; its
    // shell sets this). Off, Classic's way: it replaces the active one.
    Q_PROPERTY(bool keepOpen READ keepOpen WRITE setKeepOpen NOTIFY keepOpenChanged)
public:
    // Takes ownership of `set` (from dspi_core_set_new, not null).
    explicit DeviceRegistry(FfiCoreSet *set, QObject *parent = nullptr);
    ~DeviceRegistry() override;
    static DeviceRegistry *instance();
    static DeviceRegistry *create(QQmlEngine *, QJSEngine *);

    DeviceSession *active() const { return m_active; }
    DeviceSession *mainSession() const { return m_main; }
    QList<DeviceSession *> sessions() const { return m_sessions; }
    // The session whose device has this id; null for none
    DeviceSession *session(const QString &id) const;
    // The session that holds this unit (not a demo's or a document's serial)
    DeviceSession *holder(const QString &serial) const;
    // Nothing is open, and nothing that was is to come back
    bool isEmpty() const;

    QVariantList devices() const;
    QString activeId() const;
    bool keepOpen() const { return m_keepOpen; }
    void setKeepOpen(bool keep);

    // The folders every core gets (dspi_core_set_config's JSON): call it before
    // a device opens. A failure is configFailed().
    void configure(const QJsonObject &config);
    // Scans once, then polls for hot-plug (the set's one scan for every core)
    void startHotplug();
    // Stops every worker and frees the set. Afterwards nothing reaches a core
    // again; safe to call twice.
    void shutdown();
    bool isShutDown() const { return m_set == nullptr; }

    // Opens `target` (dspi_core_set_open's JSON) on a core of its own, beside
    // what is open, and calls `done` with its session, or null and the core's
    // code and sentence (`already_open`, `full`, ...; openFailed() too). The new
    // one is active before `done` runs unless `activate` is false.
    using Done = std::function<void(DeviceSession *session, const QString &code, const QString &error)>;
    void open(const QJsonObject &target, Done done = {}, bool activate = true);

    // For the device tabs: open beside what is open, choose, close
    Q_INVOKABLE void openDemo();
    Q_INVOKABLE void openUsb(const QString &serial);
    // False for an id that is not open
    Q_INVOKABLE bool activate(const QString &id);
    void activate(DeviceSession *session);
    // The device is closed and its session goes; the main one stays, with no
    // device. With the active one closed, a neighbour becomes active.
    Q_INVOKABLE void close(const QString &id);

    // Opens under way, for the smoke driver
    int opening() const { return m_opening; }
    // A unit another session has, refused on a session's own core
    // (SessionModel): openFailed()
    void reportRefused(const QString &serial);

signals:
    void devicesChanged();
    // -- WP MDQT2a -- Another device is about to become the active one; the one
    // left still is, everywhere. What is half-edited (a held handle, a typed
    // field) is finished or let go of now: it must not land on the next one.
    void activeLeaving();
    void activeChanged();
    // Just before activeChanged(): DeviceModel's, which moves first
    void activeMoved();
    void keepOpenChanged();
    // A session was made for a device opened beside the others
    void sessionOpened(DeviceSession *session);
    // It is about to go: let go of it and its core now. Not the active one any more.
    void sessionClosing(DeviceSession *session);
    // -- WP MDQT2c -- The session's device is about to be closed, or replaced on
    // its core (the main one's too, and before sessionClosing()): what plays on
    // it is stopped now, posted to its worker (DeviceSession::drainFirst), so
    // that the Stop goes out while the device is still there.
    void sessionStopping(DeviceSession *session);
    void openFailed(const QString &code, const QString &error);
    void configFailed(const QString &error);

private:
    static QString alreadyOpen(const QString &serial);
    static void eventThunk(const char *eventJson, void *self);
    void track(DeviceSession *session);
    void readRows();
    void endSession(DeviceSession *session, const QByteArray &id);   // -- WP MDQT2c --
    void pollHotplug();
    // `job` on `pool`'s thread, `done` back here while this lives
    template <class Job, class After>
    void run(QThreadPool *pool, Job job, After after);

    FfiCoreSet *m_set;
    DeviceSession *m_main;
    DeviceSession *m_active;
    QList<DeviceSession *> m_sessions;
    QList<DeviceSession *> m_closing;   // closed, their workers running dry
    QList<DeviceSession *> m_stopping;   // -- WP MDQT2c -- their Stops on the way, closed next
    bool m_keepOpen = false;
    int m_opening = 0;
    QTimer m_hotplugTimer;
    std::atomic<bool> m_rowsQueued{ false };
    std::atomic<bool> m_pollInFlight{ false };
    QThreadPool *m_scanner;   // one thread: polls, the set's list
    QThreadPool *m_opener;    // one thread: opens and closes
};

#endif // DEVICEREGISTRY_H
