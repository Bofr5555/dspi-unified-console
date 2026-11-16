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
    // -- WP SECQT -- The session's encryption (dspi_link_security, PLAN-SEC 4.3).
    // `encrypted`: TLS with a verified hub key. `unsecured`: a session is open,
    // its encryption has been read, and it is not encrypted (a plaintext or
    // upstream hub, or one in compatibility mode): the connection line turns red
    // and says `unsecuredLabel` (the core's notice row dlink.unsecured).
    Q_PROPERTY(bool encrypted READ encrypted NOTIFY securityChanged)
    Q_PROPERTY(bool unsecured READ unsecured NOTIFY securityChanged)
    // -- WP SECFIX -- not encrypted, but the hub is this machine (127.0.0.1,
    // ::1, localhost: the core's `local`, the web's rule): nothing crosses a
    // network, so it is not unsecured (no red, no "(unsecured)")
    Q_PROPERTY(bool local READ local NOTIFY securityChanged)
    // -- WP SECFIX -- A `dspi://pair` link is being followed (pasted where an
    // address goes; openPairLink): the PIN it carries, for the PIN prompt to
    // show filled in, so pairing still takes the user's tap on Pair after
    // the hub's name and key are on screen. "" otherwise.
    Q_PROPERTY(QString pairLinkPin READ pairLinkPin NOTIFY pairLinkChanged)
    // The hub key's fingerprint, "SHA256:K7Q2-M9XD-…", "" when none (shown on the
    // PIN prompt, before the PIN goes out)
    Q_PROPERTY(QString hubKey READ hubKey NOTIFY securityChanged)
    // The core's words: the label after the hub's name, and the info button's
    // explanation (dlink.unsecured, dlink.unsecured.info)
    Q_PROPERTY(QString unsecuredLabel READ unsecuredLabel CONSTANT)
    Q_PROPERTY(QString unsecuredInfoTitle READ unsecuredInfoTitle CONSTANT)
    Q_PROPERTY(QString unsecuredInfoText READ unsecuredInfoText CONSTANT)
    // "Encrypted connection set up; hub key …": once per new key (first use of a
    // pairing made before TLS, or a rotation the old key signed), until OK
    Q_PROPERTY(QString keyNotice READ keyNotice NOTIFY keyNoticeChanged)
    // The last failure is one only pairing again can answer (offersRePair)
    Q_PROPERTY(bool rePairOffered READ rePairOffered NOTIFY errorChanged)

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
    // A session is open (any phase but idle, closed, refused)
    bool inSession() const;

    // Open a session to `url`, replacing any other. `expectedHubId` is the hub
    // the address should be ("" if not known): its stored token goes out only
    // to that hub. With no id, no token goes out until the hub has named itself.
    // -- WP SECQT -- `tlsAdvertised`: the browsed record's tlsAdvertised (its
    // tls=1): TLS is then required, and its failure is tls_refused, never a
    // plaintext retry (dspi_link_connect_advertised)
    Q_INVOKABLE void connectToHub(const QString &url, const QString &expectedHubId = QString(), bool tlsAdvertised = false);
    // The same from what a person typed: host, host:port, [v6]:port. False when
    // it is not an address; errorText says why.
    // -- WP SECFIX -- A `dspi://pair` link given here goes to openPairLink.
    Q_INVOKABLE bool connectToAddress(const QString &address, const QString &expectedHubId = QString());
    // -- WP SECFIX -- Follow a `dspi://pair?h=&a=&k=&p=` link (a hub's QR code,
    // pasted) through the core's parser (dspi_link_parse_pair_link), as the
    // Terminal and Android do: the dial is pinned to its k=, so a hub with
    // another key fails it before any PIN goes out; then the PIN prompt shows
    // the hub's name and key with the link's PIN filled in, and pairs only on
    // the user's tap. False (errorText says why, never echoing the link) when
    // it is not a valid pairing link.
    Q_INVOKABLE bool openPairLink(const QString &text);
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
    // -- WP SECQT -- Dial the hub of the last failure again with no stored
    // credential, so it asks for its PIN and shows its key; the credential is
    // replaced only by the new pairing. Only while rePairOffered.
    Q_INVOKABLE void rePair();
    // Leaves the failure that offered re-pairing without pairing again
    Q_INVOKABLE void dismissRePair();
    Q_INVOKABLE void dismissKeyNotice();
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
    void setError(const QString &code, const QString &text);
    void applyHello(const QJsonObject &hello);
    void applyStatus(const Status &s);
    void storeToken(const QString &hubId, const QString &token);
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

#endif // LINKCONTROLLER_H
