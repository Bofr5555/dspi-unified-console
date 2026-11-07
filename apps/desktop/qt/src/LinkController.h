#ifndef LINKCONTROLLER_H
#define LINKCONTROLLER_H

#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QThreadPool>
#include <QVariantList>
#include <atomic>
#include <functional>
#include <memory>

extern "C" {
#include "dspi_core.h"
}

class DSPiBridge;
class TokenStore;

// A DSPi Link session: dial, pair, choose a device, and what to tell the user.
// QML sees it as `linkSession`. Everything that blocks runs in order on one worker
// thread, token storage included; the core's events come back to the UI thread.
//
// The core keeps a lost session up (LinkPhase::Reconnecting): there is no retry
// loop here and no write is queued. A token goes out only to the hub it was stored
// for (`expected_hub_id`), and is deleted only when a hub says `token_refused`.
class LinkController : public QObject
{
    Q_OBJECT
    // dspi_link_status's name for it: idle, needs_pairing, no_device,
    // choose_device, bound, reconnecting, closed, refused
    Q_PROPERTY(QString phase READ phase NOTIFY statusChanged)
    // CG10's wording of the phase: a key to localise by, its English, and its button if it has one
    Q_PROPERTY(QString phaseKey READ phaseKey NOTIFY statusChanged)
    Q_PROPERTY(QString phaseText READ phaseText NOTIFY statusChanged)
    Q_PROPERTY(QString actionKey READ actionKey NOTIFY statusChanged)
    Q_PROPERTY(QString actionText READ actionText NOTIFY statusChanged)
    Q_PROPERTY(int reconnectAttempt READ reconnectAttempt NOTIFY statusChanged)
    Q_PROPERTY(int reconnectInMs READ reconnectInMs NOTIFY statusChanged)
    Q_PROPERTY(QString pinSource READ pinSource NOTIFY statusChanged)
    Q_PROPERTY(QString closeReason READ closeReason NOTIFY statusChanged)
    Q_PROPERTY(QString role READ role NOTIFY statusChanged)
    // The worker has calls in hand. A dial left with closeSession counts until it
    // times out, though phase is already idle.
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)
    Q_PROPERTY(QString hubId READ hubId NOTIFY hubChanged)
    Q_PROPERTY(QString hubName READ hubName NOTIFY hubChanged)
    // [{handle, serial, name, fw, fwBeta, platform, state, link, lockedBy}]
    Q_PROPERTY(QVariantList devices READ devices NOTIFY devicesChanged)
    // The last call's failure: the core's code (wrong_hub, wrong_pin, rate_limited,
    // ...) and a sentence to show. Empty after a call that worked.
    Q_PROPERTY(QString errorCode READ errorCode NOTIFY errorChanged)
    Q_PROPERTY(QString errorText READ errorText NOTIFY errorChanged)
    // A token went to the file, because no secret store answered. "" otherwise.
    Q_PROPERTY(QString tokenWarning READ tokenWarning NOTIFY tokenWarningChanged)

public:
    // What Registrar makes: on the bridge's core, with this user's tokens
    LinkController(DSPiBridge *bridge, QObject *parent);
    // For the smoke driver: any core, any store. The core must outlive this.
    LinkController(FfiCore *core, std::unique_ptr<TokenStore> store, QObject *parent);
    ~LinkController() override;

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
    bool busy() const { return m_busy > 0; }
    QString hubId() const { return m_hubId; }
    QString hubName() const { return m_hubName; }
    QVariantList devices() const { return m_devices; }
    QString errorCode() const { return m_errorCode; }
    QString errorText() const { return m_errorText; }
    QString tokenWarning() const { return m_tokenWarning; }

    // Open a session, replacing any other. `expectedHubId` is the hub this address
    // should be, "" if not known: no token goes out until a hub has named itself.
    Q_INVOKABLE void connectToHub(const QString &url, const QString &expectedHubId = QString());
    // The same from what a person typed: host, host:port, [v6]:port. False: not an
    // address; errorText says why.
    Q_INVOKABLE bool connectToAddress(const QString &address, const QString &expectedHubId = QString());
    Q_INVOKABLE void pair(const QString &pin);
    Q_INVOKABLE void bindDevice(int handle, const QString &serial);
    // Leave the hub, also during a dial. Never blocks: the banner's button.
    Q_INVOKABLE void closeSession();
    // Reconnect now rather than after the backoff
    Q_INVOKABLE void retryNow();
    Q_INVOKABLE void dismissTokenWarning();

signals:
    void statusChanged();
    void busyChanged();
    void hubChanged();
    void devicesChanged();
    void errorChanged();
    void tokenWarningChanged();
    // A device was bound and read: the mirror holds it (`serial`). The bridge has
    // no way to adopt it yet; see the WP's report.
    void deviceBound(const QString &serial);
    // Back after a loss, and the analyser was started again if it had been running
    void reconnected();
    // A token is stored or was refused. For the smoke driver and a list of hubs.
    void tokensChanged(const QString &hubId);

private:
    static void eventCallback(uint8_t event, int32_t code, const char *text, const char *token, void *userData);

    void post(unsigned generation, std::function<void()> job);
    void setError(const QString &code, const QString &text);
    void applyHello(const QJsonObject &hello);
    void refreshStatus();
    void refreshDevices();
    void saveToken(const QString &hubId, const QString &token);
    void setBusy(int delta);

    FfiCore *m_core;
    std::unique_ptr<TokenStore> m_store;     // on the worker only
    std::atomic<bool> m_closing { false };   // ends a wait for a keyring's prompt
    std::atomic<unsigned> m_generation { 0 };   // a call stale at this number is thrown away

    QString m_phase = QStringLiteral("idle");
    QString m_phaseKey, m_phaseText, m_actionKey, m_actionText;
    int m_reconnectAttempt = 0;
    int m_reconnectInMs = 0;
    QString m_pinSource, m_closeReason, m_role;
    int m_busy = 0;
    QString m_hubId, m_hubName;
    QString m_sessionHubId;                  // who the hello of this session named
    QVariantList m_devices;
    QString m_errorCode, m_errorText;
    QString m_tokenWarning;
    QString m_lastSavedToken;
    bool m_refusalHandled = false;
    bool m_analyserWanted = false;           // ran on this session: start it after a loss
    QThreadPool m_worker;                    // one thread, joined by the destructor
};

#endif
