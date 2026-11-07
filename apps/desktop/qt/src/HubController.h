#ifndef HUBCONTROLLER_H
#define HUBCONTROLLER_H

#include <QDeadlineTimer>
#include <QJsonObject>
#include <QString>
#include <QThreadPool>
#include <QTimer>
#include <QVariantList>
#include <atomic>
#include <functional>

#include "TrayController.h"

extern "C" {
#include "dspi_core.h"
}

class DSPiBridge;

// WP QD14: sharing this machine's devices on the network (HB6's dspi_hub_*), and the
// hubs found on it, for Settings > Networking (NetworkingSettingsView.swift). QML sees
// it as `hub`; the tray's Share and Allow items come through TrayHub.
//
// A hub runs exactly while sharing is on. Its settings (name, access, port) are what
// a start is given, so changing one while it runs restarts it: the clients are
// dropped and come back by themselves, except after a change of access, which
// revokes every token. Ask revokeCost() first and warn when it is not 0.
//
// Every call that acts on the hub goes through apply(), in order on one worker: start
// and stop block for seconds, the rest write the hub's store. Reads of the running
// hub are quick and stay on the UI thread. The hub's events and the browser's wake-ups
// come from their own threads and only schedule a read here.
//
// The user's choices (sharing, access, port, looking for hubs) are QSettings under
// "hub"; the hub's own store (its id, paired clients, name) is under the app's data
// directory.
class HubController : public TrayHub
{
    Q_OBJECT
    // The user's switch: on also after a start that failed (lastError says why)
    Q_PROPERTY(bool sharing READ sharing NOTIFY stateChanged)
    Q_PROPERTY(bool running READ running NOTIFY stateChanged)
    // A start, stop or restart is on its way
    Q_PROPERTY(bool busy READ busy NOTIFY stateChanged)
    Q_PROPERTY(QString hubId READ hubId NOTIFY stateChanged)
    Q_PROPERTY(QString hubName READ hubName NOTIFY stateChanged)
    // "pin" (the default: a new client pairs with a code) or "open"
    Q_PROPERTY(QString access READ access NOTIFY stateChanged)
    // The port to listen on; boundPort is the one in use while running
    Q_PROPERTY(int port READ port NOTIFY stateChanged)
    Q_PROPERTY(int boundPort READ boundPort NOTIFY stateChanged)
    // What a client dials while running, host:port; "" when stopped
    Q_PROPERTY(QString address READ address NOTIFY stateChanged)
    // Sockets open on the hub, signed in or not
    Q_PROPERTY(int connections READ connections NOTIFY stateChanged)
    // Devices the hub's last scan could not open: another program, or a permission
    Q_PROPERTY(int inaccessible READ inaccessible NOTIFY stateChanged)
    // [{cid, name, role, created, lastSeen, online}]; from the store while stopped
    Q_PROPERTY(QVariantList clients READ clients NOTIFY clientsChanged)
    // The pairing code while a window is open, and the seconds it has left
    Q_PROPERTY(QString pairingCode READ pairingCode NOTIFY pairingChanged)
    Q_PROPERTY(int pairingSecondsLeft READ pairingSecondsLeft NOTIFY pairingChanged)
    // The last failure, as a sentence; a note (not a failure) in lastNote
    Q_PROPERTY(QString lastError READ lastError NOTIFY stateChanged)
    Q_PROPERTY(QString lastNote READ lastNote NOTIFY stateChanged)

    // Other hubs: browsing runs while `watching` (a page or list that shows them
    // holds a watch) and lookForHubs
    Q_PROPERTY(bool lookForHubs READ lookForHubs WRITE setLookForHubs NOTIFY browseChanged)
    Q_PROPERTY(bool watching READ watching NOTIFY browseChanged)
    Q_PROPERTY(bool browsing READ browsing NOTIFY browseChanged)
    // [{id, name, kind, auth, deviceCount, host, port, url, ...}] as dspi_link_browse_hubs
    Q_PROPERTY(QVariantList hubs READ hubs NOTIFY browseChanged)
    Q_PROPERTY(QString browseError READ browseError NOTIFY browseChanged)

public:
    HubController(DSPiBridge *bridge, QObject *parent);
    ~HubController() override;

    bool sharing() const { return m_sharing; }
    bool running() const { return m_running; }
    bool busy() const { return m_calls > 0; }
    QString hubId() const { return m_hubId; }
    QString hubName() const { return m_name; }
    QString access() const { return m_access; }
    int port() const { return m_port; }
    int boundPort() const { return m_boundPort; }
    QString address() const;
    int connections() const { return m_connections; }
    int inaccessible() const { return m_inaccessible; }
    QVariantList clients() const { return m_clients; }
    QString pairingCode() const { return m_pin; }
    int pairingSecondsLeft() const;
    QString lastError() const { return m_lastError; }
    QString lastNote() const { return m_lastNote; }
    bool lookForHubs() const { return m_lookForHubs; }
    void setLookForHubs(bool on);
    // -- WP QD37 -- A count of watches (c4q N3): Settings > Networking and the Devices
    // list each hold one, and browsing stops when the last is let go. Balanced
    // calls; an unwatch with none held does nothing.
    bool watching() const { return m_watchers > 0; }
    Q_INVOKABLE void watch();
    Q_INVOKABLE void unwatch();
    bool browsing() const { return m_browsing; }
    QVariantList hubs() const { return m_hubs; }
    QString browseError() const { return m_browseError; }

    // TrayHub, and the page's switch
    State trayState() const override;
    Q_INVOKABLE void setSharing(bool on) override;
    Q_INVOKABLE void allowNewClient() override;
    Q_INVOKABLE void cancelPairing() override;

    // Settings. A running hub restarts with them. False: not taken (lastError says why).
    Q_INVOKABLE bool setHubName(const QString &name);
    Q_INVOKABLE bool setAccess(const QString &mode);
    Q_INVOKABLE bool setPort(int port);
    // How many paired clients a start in `mode` would make pair again
    Q_INVOKABLE int revokeCost(const QString &mode) const;

    Q_INVOKABLE void setRole(int cid, const QString &role);
    Q_INVOKABLE void revoke(int cid);
    Q_INVOKABLE void dismissError();
    // Reads the running hub, or the store; no traffic
    Q_INVOKABLE void refresh();

    // For the smoke driver: options added to every start (HUB_OPT_LOOPBACK), and
    // where the store is. Before the first start.
    void setExtraOptions(uint32_t options) { m_extraOptions = options; }
    void setStoreDir(const QString &dir);
    FfiHub *handle() const { return m_hub; }

signals:
    void stateChanged();
    void clientsChanged();
    void pairingChanged();
    void browseChanged();

private:
    static void hubEvent(uint8_t event, int32_t code, const char *text, void *userData);
    static void browseWake(void *userData);

    void apply(std::function<void()> job);
    void start();
    void stop();
    void restartIfRunning();
    void onStarted(const QJsonObject &reply);
    void readStore();
    void applySaved(const QJsonObject &saved);
    void readHubs();
    void syncBrowse();
    void setError(const QString &text);
    void setPairing(const QString &pin, int expiresInMs);
    void emitAll();

    DSPiBridge *m_bridge;
    FfiCore *m_core;
    FfiHub *m_hub;
    QByteArray m_store;
    uint32_t m_extraOptions = 0;

    bool m_sharing = false;
    bool m_running = false;
    int m_calls = 0;
    QString m_hubId, m_name, m_access = QStringLiteral("pin");
    int m_port = 0;
    int m_boundPort = 0;
    int m_connections = 0;
    int m_inaccessible = 0;
    int m_revokeIfPin = 0, m_revokeIfOpen = 0;
    QVariantList m_clients;
    QString m_pin;
    QDeadlineTimer m_pinExpiry;
    QTimer m_pinTick;
    QString m_lastError, m_lastNote;

    bool m_lookForHubs = true;
    int m_watchers = 0;
    bool m_browsing = false;
    QString m_browseOwnId;              // the id browsing was started with
    QVariantList m_hubs;
    QString m_browseError;

    std::atomic<bool> m_readQueued { false };
    std::atomic<bool> m_hubsQueued { false };
    QThreadPool m_worker;               // one thread, joined by the destructor
};

#endif // HUBCONTROLLER_H
