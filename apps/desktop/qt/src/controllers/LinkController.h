#ifndef LINKCONTROLLER_H
#define LINKCONTROLLER_H

#include <QHash>
#include <QObject>
#include <QString>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <functional>
#include <memory>

#include "CoreApi.h"
#include "LinkSession.h"

struct AppContext;
class DeviceRegistry;
class DeviceSession;
class Poller;
class TokenStore;
class QQmlEngine;
class QJSEngine;

// The DSPi Link session of the active device as QML and the controllers see it:
// the singleton `LinkController`. -- WP MDQT1 -- A facade: every open device has
// a Link session of its own (LinkSession, where it all happens), and this shows
// and drives the active device's. When another device becomes active, every
// property is that one's.
//
// Dialling. With DeviceRegistry.keepOpen off (Classic) a hub is dialled on the
// active device's own core and its unit replaces that device, as ever. With it
// on (device tabs), the dial has a core of its own (the lobby) while the user
// pairs and chooses, shown here meanwhile, and the chosen unit opens on its own
// core beside the open devices, with the credential the dial got.
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
    // What Registrar makes: the app's open devices, this user's tokens
    LinkController(const AppContext &ctx, QObject *parent);
    // For the smoke driver: any core, any store, no app around it. The core
    // must outlive this.
    LinkController(FfiCore *core, std::unique_ptr<TokenStore> store, QObject *parent);
    ~LinkController() override;

    static LinkController *create(QQmlEngine *, QJSEngine *);

    QString phase() const;
    QString phaseKey() const;
    QString phaseText() const;
    QString actionKey() const;
    QString actionText() const;
    int reconnectAttempt() const;
    int reconnectInMs() const;
    QString pinSource() const;
    QString closeReason() const;
    QString role() const;
    int session() const;
    bool busy() const;
    QString hubId() const;
    QString hubName() const;
    QVariantList devices() const;
    QString boundSerial() const;
    QString errorCode() const;
    QString errorText() const;
    QString tokenWarning() const;
    QString lockNotice() const;
    QString sessionTip() const;
    // -- WP SECQT --
    bool encrypted() const;
    bool unsecured() const;
    bool local() const;   // -- WP SECFIX --
    QString pairLinkPin() const;   // -- WP SECFIX --
    QString hubKey() const;
    QString unsecuredLabel() const;
    QString unsecuredInfoTitle() const;
    QString unsecuredInfoText() const;
    QString keyNotice() const;
    // -- WP SECFIX -- not after a pairing link whose key the hub did not
    // present: the link was wrong, not the pairing
    bool rePairOffered() const;
    // The codes whose only answer is pairing again (PLAN-SEC 4.3): no
    // "connect anyway" exists for any of them
    static bool offersRePair(const QString &code) { return LinkSession::offersRePair(code); }
    // A session is open (any phase but idle, closed, refused)
    bool inSession() const;
    // -- WP ATQT -- the session's address, credential and TLS as dialled: the
    // Measure window's "Several units" step opens a connection of its own to
    // the same hub for each unit of the group (each judged by its own core)
    QString sessionUrl() const;
    QString sessionCredential() const;
    bool sessionTls() const;

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
    // token, so it asks for its PIN and shows its key. Only while rePairOffered.
    // -- WP PINTLS -- A hub that was pinned stays TLS-only (the core's unpaired
    // record replaces the stored one): a plaintext hub then fails with
    // tls_required_pinned before any PIN goes out. After tls_required_pinned
    // this is Forget Hub: the record is removed, the only way back to an
    // unencrypted pairing (owner decision 2026-11-17).
    Q_INVOKABLE void rePair();
    // Leaves the failure that offered re-pairing without pairing again
    Q_INVOKABLE void dismissRePair();
    Q_INVOKABLE void dismissKeyNotice();
    // Reads the session's state again (no traffic); the core's events do this.
    Q_INVOKABLE void refreshStatus();

    // For the smoke driver: runs on the controller's thread when a dial ends
    // that was given up meanwhile, with its session up, just before it is closed
    void setStaleDialHook(std::function<void()> hook);
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
    // -- WP MDQT1 -- Another session's device is shown now: every property may
    // have changed (the signals above follow)
    void subjectChanged();

private:
    // The session shown: the lobby's while a hub is dialled for another device,
    // else the active device's
    LinkSession *subject() const { return m_subject; }
    void addSession(DeviceSession *session);
    void dropSession(DeviceSession *session);
    void showSubject();
    // Where the next dial goes
    LinkSession *dialer();
    void leaveLobby();
    void openBeside(const QString &serial);

    DeviceRegistry *m_registry = nullptr;   // null in the smoke driver's own runs
    Poller *m_poller = nullptr;
    QHash<DeviceSession *, LinkSession *> m_links;
    LinkSession *m_subject = nullptr;
    FfiCore *m_lobbyCore = nullptr;
    LinkSession *m_lobby = nullptr;
    bool m_inLobby = false;
    bool m_shutDown = false;
};

#endif // LINKCONTROLLER_H
