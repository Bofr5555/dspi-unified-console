#ifndef LINKSESSION_H
#define LINKSESSION_H

#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QThreadPool>
#include <QVariantList>
#include <atomic>
#include <functional>
#include <memory>

#include "CoreApi.h"

class SessionModel;
class TokenStore;

// A DSPi Link session as this app's client side: dial a hub, pair with its PIN,
// choose one of its devices, and what to tell the user meanwhile (the Mac's
// LinkClient). -- WP MDQT1 -- One per open device (LinkController until then):
// each core has a Link session of its own. QML sees the active device's as the
// singleton `LinkController`, whose header says what each member is.
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
class LinkSession : public QObject {
    Q_OBJECT
public:
    // An open device's: its core, its model, this user's tokens
    LinkSession(FfiCore *core, SessionModel *device, QObject *parent);
    // No device around it: any core, any store (a smoke run's own; null for
    // this user's). The core must outlive this.
    LinkSession(FfiCore *core, std::unique_ptr<TokenStore> store, QObject *parent);
    ~LinkSession() override;

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
    // -- WP SECQT --
    bool encrypted() const { return m_encrypted; }
    bool unsecured() const { return m_unsecured; }
    bool local() const { return m_local; }   // -- WP SECFIX --
    QString pairLinkPin() const { return m_linkPin; }   // -- WP SECFIX --
    QString hubKey() const { return m_hubKey; }
    QString unsecuredLabel() const { return m_unsecuredLabel; }
    QString unsecuredInfoTitle() const { return m_unsecuredInfoTitle; }
    QString unsecuredInfoText() const { return m_unsecuredInfoText; }
    QString keyNotice() const { return m_keyNotice; }
    // -- WP SECFIX -- not after a pairing link whose key the hub did not
    // present: the link was wrong, not the pairing
    bool rePairOffered() const { return offersRePair(m_errorCode) && !m_linkRefused; }
    // The codes whose only answer is pairing again (PLAN-SEC 4.3): no
    // "connect anyway" exists for any of them
    static bool offersRePair(const QString &code);
    // How this computer introduces itself to a hub (its paired-clients list)
    static QString clientName();   // -- WP MDQT1 --
    // A session is open (any phase but idle, closed, refused)
    bool inSession() const;
    // -- WP ATQT -- the session's address, credential and TLS as dialled: the
    // Measure window's "Several units" step opens a connection of its own to
    // the same hub for each unit of the group (each judged by its own core)
    QString sessionUrl() const { return m_lastUrl; }
    QString sessionCredential() const { return m_credential; }
    bool sessionTls() const { return m_lastTls; }

    // Open a session to `url`, replacing any other. `expectedHubId` is the hub
    // the address should be ("" if not known): its stored token goes out only
    // to that hub. With no id, no token goes out until the hub has named itself.
    // -- WP SECQT -- `tlsAdvertised`: the browsed record's tlsAdvertised (its
    // tls=1): TLS is then required, and its failure is tls_refused, never a
    // plaintext retry (dspi_link_connect_advertised)
    void connectToHub(const QString &url, const QString &expectedHubId = QString(), bool tlsAdvertised = false);
    // The same from what a person typed: host, host:port, [v6]:port. False when
    // it is not an address; errorText says why.
    // -- WP SECFIX -- A `dspi://pair` link given here goes to openPairLink.
    bool connectToAddress(const QString &address, const QString &expectedHubId = QString());
    // -- WP SECFIX -- Follow a `dspi://pair?h=&a=&k=&p=` link (a hub's QR code,
    // pasted) through the core's parser (dspi_link_parse_pair_link), as the
    // Terminal and Android do: the dial is pinned to its k=, so a hub with
    // another key fails it before any PIN goes out; then the PIN prompt shows
    // the hub's name and key with the link's PIN filled in, and pairs only on
    // the user's tap. False (errorText says why, never echoing the link) when
    // it is not a valid pairing link.
    bool openPairLink(const QString &text);
    void pair(const QString &pin);
    void bindDevice(int handle, const QString &serial);
    // Leave the hub, also during a dial. Returns at once. The device that was
    // open before the session opens again.
    void closeSession();
    // Leave the hub for a local device the caller opens next: nothing reopens.
    void leaveForLocal();
    // -- WP MDQT1 -- The set opened a hub's unit on this core: show it as bound,
    // through the hub another session dialled (its address and credential)
    void adoptBound(const LinkSession &dialled, const QString &serial);
    void setError(const QString &code, const QString &text);
    // Reconnect now instead of after the backoff
    void retryNow();
    void dismissTokenWarning();
    // -- WP SECQT -- Dial the hub of the last failure again with no stored
    // token, so it asks for its PIN and shows its key. Only while rePairOffered.
    // -- WP PINTLS -- A hub that was pinned stays TLS-only (the core's unpaired
    // record replaces the stored one): a plaintext hub then fails with
    // tls_required_pinned before any PIN goes out. After tls_required_pinned
    // this is Forget Hub: the record is removed, the only way back to an
    // unencrypted pairing (owner decision 2026-11-17).
    void rePair();
    // Leaves the failure that offered re-pairing without pairing again
    void dismissRePair();
    void dismissKeyNotice();
    // Reads the session's state again (no traffic); the core's events do this.
    void refreshStatus();

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
    void securityChanged();      // -- WP SECQT --
    void pairLinkChanged();      // -- WP SECFIX --
    void keyNoticeChanged();     // -- WP SECQT --
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
    struct Security;   // -- WP SECQT --
    static void eventThunk(uint8_t event, int32_t code, const char *text, const char *token, void *self);

    void init();
    void post(std::function<void()> job);
    void track(int delta);
    void applyHello(const QJsonObject &hello);
    void applyStatus(const Status &s);
    void storeToken(const QString &hubId, const QString &token);
    QString load(const QString &hubId);   // -- WP MDQT1 -- on the worker
    // -- WP PINTLS -- on the worker: drop the token, keep a hub once pinned TLS-only
    void dropPairing(const QString &hubId);
    // -- WP SECQT --
    static Security readSecurity(FfiCore *core);
    void applySecurity(const Security &s);
    void clearSecurity();
    void updateUnsecured();
    // -- WP SECFIX -- `credential`: dial with it instead of the stored one
    // (a pairing link's, pinned to its key, no token)
    void dial(const QString &url, const QString &expectedHubId, bool tlsAdvertised, bool useStored,
              const QString &credential = QString());
    void setPairLinkPin(const QString &pin);   // -- WP SECFIX --
    void endSession(bool reopenLocal);
    void postBind(int handle, const QString &serial);
    void afterBind(const QString &serial);

    FfiCore *m_core;
    SessionModel *m_device = nullptr;   // null with no device around it
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
    // -- WP SECQT --
    bool m_encrypted = false;
    bool m_securityKnown = false;  // read since the last dial
    bool m_unsecured = false;      // what unsecured() says, kept to notify its changes
    QString m_hubKey;
    QString m_credential;          // the credential this session holds, as last stored or dialled with
    QString m_noticedKey;          // a key already shown (the PIN prompt, a key notice)
    QString m_keyNotice;
    QString m_unsecuredLabel, m_unsecuredInfoTitle, m_unsecuredInfoText;
    QString m_lastUrl, m_lastExpected;   // the last dial, for rePair()
    bool m_lastTls = false;
    QString m_failedHubId;         // the hub the last failed dial was for
    // -- WP SECFIX --
    bool m_local = false;          // what local() says
    QString m_linkPin;             // a pairing link's PIN until paired or left; never logged
    bool m_linkRefused = false;    // the last failure was a pairing link's key not met
    QThreadPool m_worker;   // one thread: dial, pair, bind, tokens
    QThreadPool m_reader;   // one thread: status and device list
};

#endif // LINKSESSION_H
