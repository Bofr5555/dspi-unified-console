#ifndef DEVICEREGISTRY_H
#define DEVICEREGISTRY_H

#include <QHash>
#include <QJsonObject>
#include <QList>
#include <QObject>
#include <QString>
#include <QStringList>
#include <QThreadPool>
#include <QTimer>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <atomic>
#include <functional>

class DeviceSession;
class QQmlEngine;
class QJSEngine;
struct FfiCore;   // -- WP XDEV1 --
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
    // -- WP XDEV1 -- Turning it off keeps what is open: see kOthersStayOpenInClassic.
    Q_PROPERTY(bool keepOpen READ keepOpen WRITE setKeepOpen NOTIFY keepOpenChanged)
public:
    // -- WP XDEV1 -- POLICY POINT (review RB F2; the coordinator's default, the
    // owner may change it): leaving Modern (keepOpen off) leaves the other
    // devices open in the background. Classic shows only the active one and
    // lists the others under Open devices; back in Modern their tabs are there
    // again. Made false, setKeepOpen(false) closes every device but the active
    // one (as close() does: no unsaved question, none while a job pins it:
    // DevicesController::closeOpen is the place to ask, if that is wanted).
    static constexpr bool kOthersStayOpenInClassic = true;

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
    // -- WP MCP4 -- For the AI agents' server (dspi_mcp_new), which is over every
    // open device; null after shutdown()
    const FfiCoreSet *set() const { return m_set; }

    // Opens `target` (dspi_core_set_open's JSON) on a core of its own, beside
    // what is open, and calls `done` with its session, or null and the core's
    // code and sentence (`already_open`, `full`, ...; openFailed() too). The new
    // one is active before `done` runs unless `activate` is false.
    using Done = std::function<void(DeviceSession *session, const QString &code, const QString &error)>;
    void open(const QJsonObject &target, Done done = {}, bool activate = true);

    // For the device tabs: open beside what is open, choose
    Q_INVOKABLE void openDemo();
    Q_INVOKABLE void openUsb(const QString &serial);
    // False for an id that is not open
    Q_INVOKABLE bool activate(const QString &id);
    void activate(DeviceSession *session);
    // The device is closed and its session goes; the main one stays, with no
    // device. With the active one closed, a neighbour becomes active.
    // -- WP LAY5b -- (review RB B2) Not for QML: it asks nothing and heeds no pin.
    // A person closes a device through DevicesController::closeOpen.
    void close(const QString &id);

    // Opens under way, for the smoke driver
    int opening() const { return m_opening; }
    // A unit another session has, refused on a session's own core
    // (SessionModel): openFailed()
    void reportRefused(const QString &serial);

    // -- WP XDEV1 -- A core of the set with no session, tab or worker: a unit
    // measured across units that is not open in a tab (MPLAN 1.2 item 6). It
    // holds its serial as any open device does, so the unit is not opened a
    // second time while it is measured. `done` gets its core and id, or null
    // and the core's code and sentence; openFailed() is not emitted. The core
    // stays valid after closeExtra() (retired, answering as one with no device)
    // until the set is freed; shutdown() closes what is left.
    using ExtraDone = std::function<void(FfiCore *core, const QString &id, const QString &code, const QString &error)>;
    void openExtra(const QJsonObject &target, ExtraDone done);
    void closeExtra(const QString &id);
    // The serials the extra cores hold (open, or on their way)
    QStringList extraSerials() const { return m_extras.values() + m_extraOpening; }

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
    // -- WP XDEV1 -- (review RB B3) Never for the main session, which stays with
    // no device when its device is closed: that is sessionStopping() and the
    // model's own signals (SessionModel::replacing, a new generation). Who keeps
    // something of the main session's checks the serial or generation again.
    void sessionClosing(DeviceSession *session);
    // -- WP MDQT2c -- The session's device is about to be closed, or replaced on
    // its core (the main one's too, and before sessionClosing()): what plays on
    // it is stopped now, posted to its worker (DeviceSession::drainFirst), so
    // that the Stop goes out while the device is still there.
    void sessionStopping(DeviceSession *session);
    void openFailed(const QString &code, const QString &error);
    void configFailed(const QString &error);
    // -- WP XDEV1 -- (review RB B4) A write the core refused on a device that
    // is not the active one (a limit, the lock...: SessionModel::writeRefused),
    // with that device's title; the active one's is DeviceModel::writeRefused
    void backgroundRefused(const QString &device, const QString &what, const QString &noticeKey, const QString &message);

private:
    static QString alreadyOpen(const QString &serial);
    QString refusalFor(const QString &serial) const;   // -- WP XDEV1 --
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
    // -- WP XDEV1 -- the extra cores: id -> serial, and the serials on their way
    QHash<QString, QString> m_extras;
    QStringList m_extraOpening;
};

#endif // DEVICEREGISTRY_H
