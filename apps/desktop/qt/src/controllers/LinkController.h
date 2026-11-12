#ifndef LINKCONTROLLER_H
#define LINKCONTROLLER_H

#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QThreadPool>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <atomic>
#include <functional>
#include <memory>

#include "CoreApi.h"

struct AppContext;
class DeviceModel;
class Poller;
class TokenStore;
class QQmlEngine;
class QJSEngine;

// A DSPi Link session as this app's client side: dial a hub, pair with its PIN,
// choose one of its devices, and what to tell the user meanwhile (the Mac's
// LinkClient). QML sees it as the singleton `LinkController`.
//
// Threads. Dialling, pairing, binding and the token store block (a keyring may
// wait for a person), so they run in order on this controller's own thread, not
// on the core's worker: a 15 s dial never holds up the open device's polls.
// Reading the session's status and device list takes the core's lock, so that
// runs on a second thread of its own. The GUI thread only posts and applies.
//
// Rules from the core: a lost session is kept up by the core (phase
// `reconnecting`), so there is no retry loop here and no write is queued. A
// token goes out only to the hub it was stored for (`expected_hub_id`) and is
// deleted only when a hub says `token_refused`.
class LinkController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // dspi_link_status's phase: idle, needs_pairing, no_device, choose_device,
    // bound, reconnecting, closed, refused
    Q_PROPERTY(QString phase READ phase NOTIFY statusChanged)
    // The core's wording of the phase (dspi_link_phase_text): its key, its text,
    // and the button it asks for, if any
    Q_PROPERTY(QString phaseKey READ phaseKey NOTIFY statusChanged)
    Q_PROPERTY(QString phaseText READ phaseText NOTIFY statusChanged)
    Q_PROPERTY(QString actionKey READ actionKey NOTIFY statusChanged)
    Q_PROPERTY(QString actionText READ actionText NOTIFY statusChanged)
    Q_PROPERTY(int reconnectAttempt READ reconnectAttempt NOTIFY statusChanged)
    Q_PROPERTY(int reconnectInMs READ reconnectInMs NOTIFY statusChanged)
    Q_PROPERTY(QString pinSource READ pinSource NOTIFY statusChanged)
    Q_PROPERTY(QString closeReason READ closeReason NOTIFY statusChanged)
    Q_PROPERTY(QString role READ role NOTIFY statusChanged)
    // This client's session id on the hub, 0 without one
    Q_PROPERTY(int session READ session NOTIFY statusChanged)
    // A call is queued or running on the controller's thread. A dial left with
    // closeSession() counts until it ends, though the phase is idle already.
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)
    Q_PROPERTY(QString hubId READ hubId NOTIFY hubChanged)
    Q_PROPERTY(QString hubName READ hubName NOTIFY hubChanged)
    // [{handle, serial, name, fw, fwBeta, platform, state, link, lockedBy}]
    Q_PROPERTY(QVariantList devices READ devices NOTIFY devicesChanged)
    // The serial of the hub's device this app is bound to, "" when none
    Q_PROPERTY(QString boundSerial READ boundSerial NOTIFY devicesChanged)
    // The last call's failure: the core's code (wrong_hub, wrong_pin,
    // rate_limited, ...) and its sentence. Empty after a call that worked.
    Q_PROPERTY(QString errorCode READ errorCode NOTIFY errorChanged)
    Q_PROPERTY(QString errorText READ errorText NOTIFY errorChanged)
    // Set when a token could only go to the file, because no secret store
    // answered; "" otherwise and once dismissed
    Q_PROPERTY(QString tokenWarning READ tokenWarning NOTIFY tokenWarningChanged)
    // While another console holds the bound device's lock: the sentence to show
    Q_PROPERTY(QString lockNotice READ lockNotice NOTIFY devicesChanged)
    // While a hub's device is bound: the device button's tooltip (its name via
    // the hub's, its serial, and where devices are chosen); "" otherwise
    Q_PROPERTY(QString sessionTip READ sessionTip NOTIFY devicesChanged)

public:
    // What Registrar makes: the app's core, this user's tokens
    LinkController(const AppContext &ctx, QObject *parent);
    // For the smoke driver: any core, any store, no app around it. The core
    // must outlive this.
    LinkController(FfiCore *core, std::unique_ptr<TokenStore> store, QObject *parent);
    ~LinkController() override;

    static LinkController *create(QQmlEngine *, QJSEngine *);

    QString phase() const { return m_phase; }
    QString phaseKey() const { return m_phaseKey; }
    QString phaseText() const { return m_phaseText; }
    QString actionKey() const { return m_actionKey; }
    QString actionText() const { return m_actionText; }
    int reconnectAttempt() const { return m_reconnectAttempt; }
    int reconnectInMs() const { return m_reconnectInMs; }
    QString pinSource() const { return m_pinSource; }
    QString closeReason() const { return m_closeReason; }
    QString role() const { return m_role; }
    int session() const { return m_session; }
    bool busy() const { return m_busy > 0; }
    QString hubId() const { return m_hubId; }
    QString hubName() const { return m_hubName; }
    QVariantList devices() const { return m_devices; }
    QString boundSerial() const { return m_boundSerial; }
    QString errorCode() const { return m_errorCode; }
    QString errorText() const { return m_errorText; }
    QString tokenWarning() const { return m_tokenWarning; }
    QString lockNotice() const;
    QString sessionTip() const;
    // A session is open (any phase but idle, closed, refused)
    bool inSession() const;

    // Open a session to `url`, replacing any other. `expectedHubId` is the hub
    // the address should be ("" if not known): its stored token goes out only
    // to that hub. With no id, no token goes out until the hub has named itself.
    Q_INVOKABLE void connectToHub(const QString &url, const QString &expectedHubId = QString());
    // The same from what a person typed: host, host:port, [v6]:port. False when
    // it is not an address; errorText says why.
    Q_INVOKABLE bool connectToAddress(const QString &address, const QString &expectedHubId = QString());
    Q_INVOKABLE void pair(const QString &pin);
    Q_INVOKABLE void bindDevice(int handle, const QString &serial);
    // Leave the hub, also during a dial. Returns at once. The device that was
    // open before the session opens again.
    Q_INVOKABLE void closeSession();
    // Leave the hub for a local device the caller opens next: nothing reopens.
    void leaveForLocal();
    // Reconnect now instead of after the backoff
    Q_INVOKABLE void retryNow();
    Q_INVOKABLE void dismissTokenWarning();
    // Reads the session's state again (no traffic); the core's events do this.
    Q_INVOKABLE void refreshStatus();

    // For the smoke driver: runs on the controller's thread when a dial ends
    // that was given up meanwhile, with its session up, just before it is closed
    void setStaleDialHook(std::function<void()> hook) { m_staleDialHook = std::move(hook); }
    // Ends the threads and the listener; the app's quit does it before the core
    // goes. Safe to call twice.
    void shutdown();

signals:
    void statusChanged();
    void busyChanged();
    void hubChanged();
    void devicesChanged();
    void errorChanged();
    void tokenWarningChanged();
    // A hub's device was bound and the app shows it
    void deviceBound(const QString &serial);
    // Back after a lost connection; the analyser was started again if it ran
    void reconnected();
    // A session ended (closed, refused or left) that had a device bound
    void sessionEnded();
    // A token was stored or forgotten for this hub id
    void tokensChanged(const QString &hubId);
    // A dial failed for want of an answer: the hub may have gone or moved.
    // `hubId` is the one the dial expected, "" if none.
    void dialFailed(const QString &hubId, const QString &code);

private:
    struct Status;
    static void eventThunk(uint8_t event, int32_t code, const char *text, const char *token, void *self);

    void init();
    void post(std::function<void()> job);
    void track(int delta);
    void setError(const QString &code, const QString &text);
    void applyHello(const QJsonObject &hello);
    void applyStatus(const Status &s);
    void storeToken(const QString &hubId, const QString &token);
    void endSession(bool reopenLocal);
    void postBind(int handle, const QString &serial);
    void afterBind(const QString &serial);

    FfiCore *m_core;
    DeviceModel *m_device = nullptr;   // null in the smoke driver's own runs
    Poller *m_poller = nullptr;
    std::unique_ptr<TokenStore> m_store;   // on the controller's thread only
    QString m_lastSaved;                   // hub id and token last stored; that thread too
    std::atomic<bool> m_closing{ false };  // ends a wait for a keyring's prompt
    std::atomic<unsigned> m_generation{ 0 };   // a dial stale at this number is given up
    std::atomic<bool> m_closeWanted{ false };  // the latest intent was to leave
    std::atomic<bool> m_readQueued{ false };
    bool m_shutDown = false;

    QString m_phase = QStringLiteral("idle");
    QString m_phaseKey, m_phaseText, m_actionKey, m_actionText;
    int m_reconnectAttempt = 0;
    int m_reconnectInMs = 0;
    QString m_pinSource, m_closeReason, m_role;
    int m_session = 0;
    int m_busy = 0;
    QString m_hubId, m_hubName;
    QVariantList m_devices;
    QString m_boundSerial;       // bound and shown by the app
    QString m_localBefore;       // what was open before the bind: a serial, "demo" or ""
    QString m_errorCode, m_errorText;
    QString m_tokenWarning;
    bool m_analyserWanted = false;   // without the app's device model: restart the analyser after a reconnect
    bool m_holdEnd = false;      // closed or refused stays shown until the next dial or leave
    std::function<void()> m_staleDialHook;
    QThreadPool m_worker;   // one thread: dial, pair, bind, tokens
    QThreadPool m_reader;   // one thread: status and device list
};

#endif // LINKCONTROLLER_H
