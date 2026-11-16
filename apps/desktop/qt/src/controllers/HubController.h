#ifndef HUBCONTROLLER_H
#define HUBCONTROLLER_H

#include <QDateTime>
#include <QObject>
#include <QSet>
#include <QString>
#include <QStringList>
#include <QThreadPool>
#include <QTimer>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <atomic>

#include "CoreApi.h"
#include "Registrar.h"

class Core;
class DeviceModel;
class QQmlEngine;
class QJSEngine;

// Sharing this computer's devices on the local network (Settings > Networking,
// the Mac's LinkService): the core's hub (dspi_hub_*), which serves the app's
// devices to phones, browsers and other consoles, advertises itself (DISC2) and
// keeps its paired clients in its own store. QML sees it as the singleton
// `HubController`; the tray and SystemIntegration follow `running`.
//
// - The switch is `sharing` (hub/sharing); the hub runs while it is on and the
//   start did not fail. A failed start leaves the switch on with `lastError`.
// - A running hub keeps its name, port and access for life: a new name or
//   access restarts it, and the port is refused while it runs (dspi-capi's
//   fns_hub.rs).
// - The app's core is put on the hub (dspi_hub_attach_app) so its USB devices
//   are served; while the app shows the demo, the hub serves a demo of its own.
// - The blocking calls (start, stop, attaching, pairing, the store's writes)
//   run in order on a thread of this controller's; attaching the app runs on the
//   core's worker, which owns the core's device. The hub's events only schedule
//   a re-read here.
class HubController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // The page's switch, stored as hub/sharing
    Q_PROPERTY(bool sharing READ sharing WRITE setSharing NOTIFY sharingChanged)
    Q_PROPERTY(bool running READ running NOTIFY stateChanged)
    // Starting or stopping
    Q_PROPERTY(bool busy READ busy NOTIFY stateChanged)
    // Why the last start failed (the OS's reason), "" otherwise
    Q_PROPERTY(QString lastError READ lastError NOTIFY stateChanged)
    Q_PROPERTY(QString hubId READ hubId NOTIFY stateChanged)
    // The name other consoles see: the stored one, the core's default until named
    Q_PROPERTY(QString hubName READ hubName NOTIFY settingsChanged)
    // "pin" (Require Pairing, the default) or "none" (Open)
    Q_PROPERTY(QString access READ access NOTIFY settingsChanged)
    // The port to listen on (hub/port, 11915 by default)
    Q_PROPERTY(int port READ port NOTIFY settingsChanged)
    // While running: "192.168.1.20:11915", the address a client would type
    Q_PROPERTY(QString address READ address NOTIFY stateChanged)
    // Clients with a session now
    Q_PROPERTY(int sessions READ sessions NOTIFY stateChanged)
    // "192.168.1.20:11915 · 2 clients connected" (the Mac's status line)
    Q_PROPERTY(QString statusLine READ statusLine NOTIFY stateChanged)
    // While running, the hub serves a demo device of its own
    Q_PROPERTY(bool sharesOwnDemo READ sharesOwnDemo NOTIFY stateChanged)
    // [{cid, name, role, lastSeen (seconds since the epoch, 0 = never), online}]
    Q_PROPERTY(QVariantList clients READ clients NOTIFY clientsChanged)
    // The open pairing window's six digits, "" when none is open
    Q_PROPERTY(QString pairingCode READ pairingCode NOTIFY pairingChanged)
    Q_PROPERTY(int pairingSecondsLeft READ pairingSecondsLeft NOTIFY pairingChanged)
    // -- WP SECQT -- TLS on the hub's port (PLAN-SEC 4.1). A hub on the network
    // requires TLS (HUB_OPT_TLS_REQUIRE); one on loopback has it off.
    // `upstreamCompat` (hub/upstreamCompat, off by default) is the owner's
    // opt-in compatibility mode for upstream DSPi Link clients: a security
    // downgrade, switched on only after the warning listing `compatEffects`.
    Q_PROPERTY(bool upstreamCompat READ upstreamCompat NOTIFY settingsChanged)
    // The hub's own list of what compatibility mode turns off
    // (dspi_hub_upstream_compat_effects)
    Q_PROPERTY(QStringList compatEffects READ compatEffects CONSTANT)
    // While running: "off", "on" or "require", as the start reported it
    Q_PROPERTY(QString tlsMode READ tlsMode NOTIFY stateChanged)
    // While running with TLS: the key's fingerprint, "SHA256:K7Q2-M9XD-…",
    // shown with the pairing code
    Q_PROPERTY(QString tlsFingerprint READ tlsFingerprint NOTIFY stateChanged)

public:
    HubController(const AppContext &ctx, QObject *parent);
    ~HubController() override;
    static HubController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<HubController>(); }

    bool sharing() const { return m_sharing; }
    void setSharing(bool on);
    bool running() const { return m_running; }
    bool busy() const { return m_busy; }
    QString lastError() const { return m_lastError; }
    QString hubId() const { return m_hubId; }
    QString hubName() const { return m_hubName; }
    QString access() const { return m_access; }
    int port() const { return m_port; }
    QString address() const;
    int sessions() const { return int(m_sessions.size()); }
    QString statusLine() const;
    bool sharesOwnDemo() const { return m_running && m_ownDemo >= 0; }
    QVariantList clients() const { return m_clients; }
    QString pairingCode() const { return m_pairingCode; }
    int pairingSecondsLeft() const;
    // -- WP SECQT --
    bool upstreamCompat() const { return m_upstreamCompat; }
    QStringList compatEffects() const { return m_compatEffects; }
    QString tlsMode() const { return m_running ? m_tlsMode : QString(); }
    QString tlsFingerprint() const { return m_running ? m_tlsFingerprint : QString(); }
    // Compatibility mode on or off, stored; restarts a running hub. The page
    // asks first, with the warning, before switching it on.
    Q_INVOKABLE void setUpstreamCompat(bool on);
    // The HUB_OPT_* a start passes: the app's, the smoke driver's `extra`, and
    // the TLS mode: compatibility mode when asked, else what `extra` names,
    // else off on loopback (nothing crosses a network) and required otherwise
    static uint32_t startOptions(uint32_t extra, bool upstreamCompat);

    // A new name (restarts a running hub); "" keeps the one there is
    Q_INVOKABLE void setHubName(const QString &name);
    // "pin" or "none"; restarts a running hub. Ask first when revokeCost() is not
    // 0: the paired clients of the other mode must pair again
    Q_INVOKABLE void setAccess(const QString &mode);
    // How many paired clients a start in `mode` would forget
    Q_INVOKABLE int revokeCost(const QString &mode) const;
    // 1024..65535, and only while stopped; false (and lastError) otherwise
    Q_INVOKABLE bool setPort(int port);
    Q_INVOKABLE void allowNewClient();
    Q_INVOKABLE void cancelPairing();
    Q_INVOKABLE void revoke(int cid);
    Q_INVOKABLE void setRole(int cid, const QString &role);
    Q_INVOKABLE void dismissError();
    // Reads the clients and the status again (the page's onVisible)
    Q_INVOKABLE void refresh();

    // For the smoke driver: the hub's own directory, extra HUB_OPT_* options
    // (HUB_OPT_LOOPBACK), the handle for direct reads
    void setStoreDir(const QString &dir);
    QString storeDir() const { return m_storeDir; }
    void setExtraOptions(uint32_t options) { m_extraOptions = options; }
    FfiHub *handle() const { return m_hub; }

signals:
    void sharingChanged();
    void stateChanged();
    void settingsChanged();
    void clientsChanged();
    void pairingChanged();

private:
    struct StartResult;
    static void eventThunk(uint8_t event, int32_t code, const char *text, void *self);
    void onEvent(int event, int code, const QString &text);
    void reconcile();
    void startHub();
    void stopHub();
    void readSaved();
    void readLive();
    void setError(const QString &error);
    void closePairing();
    void shutdown();
    // Runs `job` on the hub's thread, then `done` on the GUI thread
    template <class Job, class Done>
    void onHubThread(Job job, Done done);

    Core *m_core;
    DeviceModel *m_device;
    FfiHub *m_hub = nullptr;
    QString m_storeDir;
    uint32_t m_extraOptions = 0;

    bool m_sharing = false;
    bool m_running = false;
    bool m_busy = false;
    bool m_restart = false;      // a setting changed: stop, then start again
    QString m_lastError;
    QString m_hubId;
    QString m_hubName;
    QString m_nameSetting;       // hub/name, "" until the user names the hub
    QString m_access = QStringLiteral("pin");
    int m_port;
    int m_boundPort = 0;
    QString m_lanAddress; // -- WP QPI2 -- read when the hub starts
    int m_ownDemo = -1;
    QSet<int> m_sessions;
    QVariantList m_clients;
    QVariantMap m_revokeCosts;
    QString m_pairingCode;
    QDateTime m_pairingEnds;
    QTimer m_pairingTick;
    std::atomic<bool> m_readQueued{ false };
    bool m_shutDown = false;
    QThreadPool m_thread;        // one thread: the hub's blocking calls, in order
    // -- WP SECQT --
    bool m_upstreamCompat = false;
    QStringList m_compatEffects;
    QString m_tlsMode, m_tlsFingerprint;
};

#endif // HUBCONTROLLER_H
