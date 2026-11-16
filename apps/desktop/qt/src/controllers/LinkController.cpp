#include "LinkController.h"

#include "AppIdentity.h"
#include "Core.h"
#include "DeviceModel.h"
#include "Poller.h"
#include "Registrar.h"
#include "Snapshot.h"
#include "TokenStore.h"

#include <QCoreApplication>
#include <QJSEngine>
#include <QJsonArray>
#include <QJsonDocument>
#include <QRegularExpression>
#include <QSysInfo>

static const Registrar::Controller<LinkController> linkController;
// The banners of a Link session, under the firmware banners and above the demo's
static const Registrar::Banner linkBanner("LinkBanner", 20);
static const Registrar::Banner lockNotice("LockNotice", 25);
static const Registrar::Banner tokenNotice("TokenNotice", 40);
// -- WP SECQT -- a new hub key, once, beside the token notice
static const Registrar::Banner keyNoticeBanner("KeyNotice", 41);

namespace {

// The calls that act on the hub (connect, pair, select) are made once, with a
// buffer far beyond any reply: a reply that did not fit has still happened, and
// a PIN is good only once (fns_link.rs, "Short buffers").
constexpr uint32_t kActBuffer = 64 * 1024;

QJsonObject actOnce(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(int(kActBuffer), '\0');
    const uint32_t need = call(buf.data(), kActBuffer);
    if (need == 0 || need > kActBuffer) return QJsonObject();
    buf.truncate(int(need) - 1);
    return QJsonDocument::fromJson(buf).object();
}

QJsonObject readObject(const std::function<uint32_t(char *, uint32_t)> &fill) {
    return QJsonDocument::fromJson(readSized(fill, 4096)).object();
}

bool isOk(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }
QString str(const QJsonObject &o, const char *key) { return o.value(QLatin1String(key)).toString(); }

// How this computer introduces itself to a hub (its paired-clients list)
QByteArray clientName() {
    const QString host = QSysInfo::machineHostName();
    return (host.isEmpty() ? AppIdentity::displayName() : host).toUtf8();
}

// A failure that may mean the hub went away or moved (the browser checks again)
bool unreachable(const QString &code) {
    return code == QLatin1String("io") || code == QLatin1String("timeout") || code == QLatin1String("handshake");
}

} // namespace

// -- WP SECQT -- The session's encryption (dspi_link_security)
struct LinkController::Security {
    bool read = false;   // the call answered
    bool encrypted = false;
    bool hubKeyNew = false;
    QString hubKey, credential;
};

// What one read of the session gives (the reader thread hands it over whole)
struct LinkController::Status {
    QString phase = QStringLiteral("idle");
    int reconnectAttempt = 0;
    int reconnectInMs = 0;
    QString pinSource, closeReason, role;
    int session = 0;
    bool tokenRefused = false;
    QVariantList devices;
    Security security;   // -- WP SECQT --
};

LinkController::LinkController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_core(ctx.core->handle()), m_device(ctx.device), m_poller(ctx.poller),
      m_store(TokenStore::forThisUser(&m_closing)) {
    init();
    // Before the core goes (main() frees it after the event loop ends)
    connect(qApp, &QCoreApplication::aboutToQuit, this, &LinkController::shutdown);
}

LinkController::LinkController(FfiCore *core, std::unique_ptr<TokenStore> store, QObject *parent)
    : QObject(parent), m_core(core), m_store(std::move(store)) {
    init();
}

LinkController::~LinkController() { shutdown(); }

LinkController *LinkController::create(QQmlEngine *, QJSEngine *) {
    LinkController *self = Registrar::instance<LinkController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

void LinkController::init() {
    m_worker.setMaxThreadCount(1);
    m_worker.setExpiryTimeout(-1);
    m_reader.setMaxThreadCount(1);
    m_reader.setExpiryTimeout(-1);
    dspi_link_set_listener(m_core, &LinkController::eventThunk, this);
    // -- WP SECQT -- the core's words for an unencrypted session (a pure
    // function of the build: read once)
    const auto row = [](const char *key) {
        return readObject([key](char *b, uint32_t n) { return dspi_notice_for(key, nullptr, b, n); });
    };
    const QJsonObject label = row("dlink.unsecured"), info = row("dlink.unsecured.info");
    m_unsecuredLabel = str(label, "key") == QLatin1String("dlink.unsecured") ? str(label, "title") : tr("unsecured");
    m_unsecuredInfoTitle = str(info, "title");
    m_unsecuredInfoText = str(info, "reason") + QStringLiteral("\n\n") + str(info, "action");
    applyStatus(Status{});
}

void LinkController::shutdown() {
    if (m_shutDown) return;
    m_shutDown = true;
    m_closing = true;
    ++m_generation;
    // Waits for an event in progress, so none reaches this object afterwards
    dspi_link_set_listener(m_core, nullptr, nullptr);
    dspi_link_close(m_core);   // ends a dial in progress
    m_worker.waitForDone();
    m_reader.waitForDone();
}

// Core thread, no core lock held: schedule and return
void LinkController::eventThunk(uint8_t event, int32_t, const char *text, const char *token, void *self) {
    auto *link = static_cast<LinkController *>(self);
    if (event == LINK_EVENT_TOKEN_ISSUED) {
        const QString hubId = QString::fromUtf8(text);
        const QString issued = QString::fromUtf8(token);
        QMetaObject::invokeMethod(
            link,
            [link, hubId, issued] {
                // -- WP SECQT -- the credential, pinned when it came over TLS
                if (hubId == link->m_hubId) link->m_credential = issued;
                link->storeToken(hubId, issued);
            },
            Qt::QueuedConnection);
        return;
    }
    QMetaObject::invokeMethod(link, &LinkController::refreshStatus, Qt::QueuedConnection);
}

void LinkController::post(std::function<void()> job) {
    if (m_shutDown) return;
    track(1);
    m_worker.start(std::move(job));
}

void LinkController::track(int delta) {
    const bool before = busy();
    m_busy += delta;
    if (busy() != before) emit busyChanged();
}

void LinkController::setError(const QString &code, const QString &text) {
    if (code == m_errorCode && text == m_errorText) return;
    m_errorCode = code;
    m_errorText = text;
    emit errorChanged();
}

// ---- Status ------------------------------------------------------------------

void LinkController::refreshStatus() {
    if (m_shutDown || m_readQueued.exchange(true)) return;
    m_reader.start([this] {
        m_readQueued = false;
        FfiCore *c = m_core;
        const QJsonObject o = readObject([c](char *b, uint32_t n) { return dspi_link_status(c, b, n); });
        Status s;
        if (isOk(o)) {
            s.phase = str(o, "phase");
            s.reconnectAttempt = o.value(QLatin1String("reconnectAttempt")).toInt();
            s.reconnectInMs = o.value(QLatin1String("reconnectInMs")).toInt();
            s.pinSource = str(o, "pinSource");
            s.closeReason = str(o, "closeReason");
            s.role = str(o, "role");
            s.session = o.value(QLatin1String("session")).toInt();
            s.tokenRefused = o.value(QLatin1String("tokenRefused")).toBool();
        }
        s.devices = jsonValue(readSized([c](char *b, uint32_t n) { return dspi_link_devices(c, b, n); }))
                        .toArray()
                        .toVariantList();
        s.security = readSecurity(c);   // -- WP SECQT --
        QMetaObject::invokeMethod(this, [this, s] { applyStatus(s); }, Qt::QueuedConnection);
    });
}

void LinkController::applyStatus(const Status &s) {
    const QString before = m_phase;
    const QString phase = s.phase.isEmpty() ? QStringLiteral("idle") : s.phase;
    // An ended session's phase (closed, refused) stays on show until the user
    // dials or leaves: reopening the local device afterwards clears the core's
    if (m_holdEnd && phase == QLatin1String("idle")) return;
    m_holdEnd = phase == QLatin1String("closed") || phase == QLatin1String("refused");
    // The core's sentence for it, never one of ours
    const QByteArray p = phase.toUtf8(), source = s.pinSource.toUtf8();
    const QJsonObject words = readObject([&](char *b, uint32_t n) {
        return dspi_link_phase_text(p.constData(), source.isEmpty() ? nullptr : source.constData(), b, n);
    });
    m_phase = phase;
    m_phaseKey = str(words, "key");
    m_phaseText = str(words, "text");
    m_actionKey = str(words, "actionKey");
    m_actionText = str(words, "action");
    m_reconnectAttempt = s.reconnectAttempt;
    m_reconnectInMs = s.reconnectInMs;
    m_pinSource = s.pinSource;
    m_closeReason = s.closeReason;
    m_role = s.role;
    m_session = s.session;
    // While reconnecting the hub lists nothing: its devices stay on show, greyed
    if (phase != QLatin1String("reconnecting") || !s.devices.isEmpty()) m_devices = s.devices;
    emit statusChanged();
    emit devicesChanged();
    // -- WP SECQT -- the encryption with it; none outside a session. While
    // reconnecting the core has no connection to read it from: the last
    // reading stands until it is back.
    if (!inSession()) clearSecurity();
    else if (phase != QLatin1String("reconnecting")) applySecurity(s.security);
    updateUnsecured();

    if (s.tokenRefused && !m_hubId.isEmpty()) {
        const QString hub = m_hubId;
        post([this, hub] {
            m_store->forget(hub);
            QMetaObject::invokeMethod(this, [this, hub] { track(-1); emit tokensChanged(hub); }, Qt::QueuedConnection);
        });
    }
    if (m_boundSerial.isEmpty()) return;
    if (before == QLatin1String("reconnecting") && phase == QLatin1String("bound")) {
        // The core bound the device again; the mirror is read again, and the
        // analyser, whose worker went with the old connection, starts again:
        // RtaController's on reconnected(), this one only without the app's
        // device model
        if (m_device) {
            m_device->refresh();
        } else if (m_analyserWanted) {
            post([this] {
                dspi_rta_start(m_core);
                QMetaObject::invokeMethod(this, [this] { track(-1); }, Qt::QueuedConnection);
            });
        }
        // -- WP MERGEP1D -- the hub's lock, read again after the reconnect
        post([this] {
            actOnce([this](char *b, uint32_t n) { return dspi_installer_lock_refresh(m_core, b, n); });
            QMetaObject::invokeMethod(this, [this] { track(-1); }, Qt::QueuedConnection);
        });
        emit reconnected();
    } else if (!inSession()) {
        endSession(true);
    }
}

bool LinkController::inSession() const {
    return m_phase != QLatin1String("idle") && m_phase != QLatin1String("closed") && m_phase != QLatin1String("refused");
}

QString LinkController::lockNotice() const {
    if (m_boundSerial.isEmpty()) return QString();
    for (const QVariant &d : m_devices) {
        const QVariantMap device = d.toMap();
        if (device.value(QStringLiteral("serial")).toString() != m_boundSerial) continue;
        const int holder = device.value(QStringLiteral("lockedBy")).toInt();
        if (holder != 0 && holder != m_session) return tr("Another console is applying changes");
    }
    return QString();
}

QString LinkController::sessionTip() const {
    if (m_boundSerial.isEmpty()) return QString();
    QString name;
    for (const QVariant &d : m_devices) {
        const QVariantMap device = d.toMap();
        if (device.value(QStringLiteral("serial")).toString() == m_boundSerial)
            name = device.value(QStringLiteral("name")).toString();
    }
    if (name.isEmpty()) name = tr("DSPi");
    const QString where = tr("Choose devices in Device > Devices…");
    if (m_hubName.isEmpty()) return tr("%1 (%2)").arg(name, m_boundSerial) + QLatin1Char('\n') + where;
    return tr("%1 via %2 (%3)").arg(name, m_hubName, m_boundSerial) + QLatin1Char('\n') + where;
}

void LinkController::applyHello(const QJsonObject &hello) {
    const QString id = str(hello, "hubId"), name = str(hello, "hubName");
    if (id == m_hubId && name == m_hubName) return;
    m_hubId = id;
    m_hubName = name;
    emit hubChanged();
}

// ---- Calls -------------------------------------------------------------------

bool LinkController::connectToAddress(const QString &address, const QString &expectedHubId) {
    const QByteArray text = address.trimmed().toUtf8();
    const QJsonObject parsed = readObject([&](char *b, uint32_t n) { return dspi_link_parse_address(text.constData(), b, n); });
    if (!isOk(parsed)) {
        setError(str(parsed, "code").isEmpty() ? QStringLiteral("invalid") : str(parsed, "code"),
                 str(parsed, "error").isEmpty() ? tr("That is not a hub address.") : str(parsed, "error"));
        return false;
    }
    const QByteArray host = str(parsed, "host").toUtf8();
    const auto port = uint16_t(parsed.value(QLatin1String("port")).toInt());
    const QByteArray url = readSized([&](char *b, uint32_t n) { return dspi_link_ws_url(host.constData(), port, nullptr, b, n); });
    if (url.isEmpty()) {
        setError(QStringLiteral("invalid"), tr("That is not a hub address."));
        return false;
    }
    connectToHub(QString::fromUtf8(url), expectedHubId);
    return true;
}

void LinkController::connectToHub(const QString &url, const QString &expectedHubId, bool tlsAdvertised) {
    dial(url, expectedHubId, tlsAdvertised, true);
}

// -- WP SECQT -- `useStored` false: no credential goes out (rePair), so the hub
// asks for its PIN and the prompt shows the key it presents
void LinkController::dial(const QString &url, const QString &expectedHubId, bool tlsAdvertised, bool useStored) {
    m_holdEnd = false;
    const unsigned generation = ++m_generation;
    m_closeWanted = false;
    setError(QString(), QString());
    clearSecurity();
    m_lastUrl = url;
    m_lastExpected = expectedHubId;
    m_lastTls = tlsAdvertised;
    const QByteArray u = url.toUtf8(), name = clientName(), version = AppIdentity::version().toUtf8();
    post([this, generation, u, name, version, expectedHubId, tlsAdvertised, useStored] {
        FfiCore *c = m_core;
        const auto dial = [&](const QString &token, const QString &expected) {
            const QByteArray t = token.toUtf8(), e = expected.toUtf8();
            return actOnce([&](char *b, uint32_t n) {
                return dspi_link_connect_advertised(c, u.constData(), name.constData(), version.constData(),
                                                    t.constData(), e.constData(), tlsAdvertised, b, n);
            });
        };
        QString used = expectedHubId.isEmpty() || !useStored ? QString() : m_store->load(expectedHubId);
        QString dialledFor = expectedHubId;
        QJsonObject hello = dial(used, expectedHubId);
        // Dialled by address: the hub has named itself now, so a token stored
        // for that id may go to it, held to that id
        if (useStored && isOk(hello) && expectedHubId.isEmpty() && hello.value(QLatin1String("needsPairing")).toBool()
            && generation == m_generation) {
            const QString named = str(hello, "hubId");
            const QString stored = m_store->load(named);
            dialledFor = named;
            if (!stored.isEmpty()) {
                used = stored;
                hello = dial(stored, named);
            }
        }
        const bool ok = isOk(hello);
        if (generation != m_generation) {
            // Given up while it dialled. A dial replaced by another is replaced
            // by the core; one the user left from must not stay up.
            if (ok && m_closeWanted) {
                if (m_staleDialHook) m_staleDialHook();
                dspi_link_close(c);
            }
            QMetaObject::invokeMethod(this, [this] { track(-1); }, Qt::QueuedConnection);
            return;
        }
        if (ok && hello.value(QLatin1String("tokenRefused")).toBool()) {
            m_store->forget(str(hello, "hubId"));
            used.clear();
        }
        // -- WP SECQT -- the store already holds what went out: the save below
        // is only for a credential the core changed (a v1 token now pinned)
        if (ok && !used.isEmpty()) m_lastSaved = str(hello, "hubId") + QLatin1Char('\n') + used;
        const Security security = ok ? readSecurity(c) : Security{};
        QMetaObject::invokeMethod(
            this,
            [this, hello, ok, expectedHubId, used, security, dialledFor] {
                track(-1);
                if (ok) {
                    applyHello(hello);
                    setError(QString(), QString());
                    m_failedHubId.clear();
                    m_credential = used;   // -- WP SECQT --
                    // A pairing to come shows the key on its prompt: no notice
                    // for it afterwards
                    if (hello.value(QLatin1String("needsPairing")).toBool()) m_noticedKey = security.hubKey;
                    applySecurity(security);
                    if (hello.value(QLatin1String("tokenRefused")).toBool()) emit tokensChanged(m_hubId);
                } else {
                    m_failedHubId = dialledFor;   // -- WP SECQT --
                    const QString code = str(hello, "code");
                    setError(code.isEmpty() ? QStringLiteral("io") : code,
                             str(hello, "error").isEmpty() ? tr("The hub did not answer.") : str(hello, "error"));
                    if (unreachable(m_errorCode)) emit dialFailed(expectedHubId, m_errorCode);
                }
                refreshStatus();
            },
            Qt::QueuedConnection);
    });
}

void LinkController::pair(const QString &pin) {
    // Six digits or nothing: a typo costs no try against the hub's limit
    static const QRegularExpression sixDigits(QStringLiteral("^[0-9]{6}$"));
    if (!sixDigits.match(pin).hasMatch()) return;
    setError(QString(), QString());
    const QByteArray p = pin.toUtf8();
    const QString hub = m_hubId;
    post([this, p, hub] {
        FfiCore *c = m_core;
        const QJsonObject reply = actOnce([&](char *b, uint32_t n) { return dspi_link_pair(c, p.constData(), b, n); });
        QMetaObject::invokeMethod(
            this,
            [this, reply, hub] {
                track(-1);
                if (isOk(reply)) {
                    setError(QString(), QString());
                    // -- WP SECQT -- the credential, pinned when the pairing ran
                    // over TLS; the key was on the prompt
                    m_noticedKey = m_hubKey;
                    if (!str(reply, "token").isEmpty()) {
                        if (hub == m_hubId) m_credential = str(reply, "token");
                        storeToken(hub, str(reply, "token"));
                    }
                } else {
                    setError(str(reply, "code").isEmpty() ? QStringLiteral("io") : str(reply, "code"),
                             str(reply, "error").isEmpty() ? tr("The hub did not answer.") : str(reply, "error"));
                }
                refreshStatus();
            },
            Qt::QueuedConnection);
    });
}

void LinkController::storeToken(const QString &hubId, const QString &token) {
    if (hubId.isEmpty() || token.isEmpty()) return;
    // The pair reply and the hub's token event both bring it: save it once
    post([this, hubId, token] {
        TokenStore::Saved saved = TokenStore::Saved::InSecretStore;
        const QString entry = hubId + QLatin1Char('\n') + token;
        const bool fresh = entry != m_lastSaved;
        if (fresh) {
            saved = m_store->save(hubId, token);
            if (saved != TokenStore::Saved::Nowhere) m_lastSaved = entry;
        }
        const QString file = m_store->filePath();
        QMetaObject::invokeMethod(
            this,
            [this, hubId, saved, fresh, file] {
                track(-1);
                if (!fresh) return;
                if (saved == TokenStore::Saved::InFile) {
                    m_tokenWarning = tr("No secret store answered, so this hub's pairing is saved in a file only "
                                        "your account can read (%1).")
                                         .arg(file);
                    emit tokenWarningChanged();
                }
                emit tokensChanged(hubId);
            },
            Qt::QueuedConnection);
    });
}

void LinkController::bindDevice(int handle, const QString &serial) {
    if (handle < 0) return;
    setError(QString(), QString());
    // A local device chosen just before (the demo, a USB unit) opens on the
    // core's worker; the bind waits for that, or the late open would take the
    // connection from the hub's device
    if (m_device) {
        track(1);
        m_device->core()->worker()->call(this, [](FfiCore *) { return 0; }, [this, handle, serial](int) {
            track(-1);
            postBind(handle, serial);
        });
    } else {
        postBind(handle, serial);
    }
}

void LinkController::postBind(int handle, const QString &serial) {
    const QByteArray s = serial.toUtf8();
    const bool ownMirror = m_device == nullptr;
    post([this, handle, s, serial, ownMirror] {
        FfiCore *c = m_core;
        const QJsonObject reply =
            actOnce([&](char *b, uint32_t n) { return dspi_link_select_device(c, uint32_t(handle), s.constData(), b, n); });
        const bool ok = isOk(reply);
        // Without the app's device model (the smoke driver's own runs) the
        // device is read here, as DeviceModel would on its worker
        if (ok && ownMirror) {
            dspi_fetch_all(c);
            dspi_rta_start(c);
        }
        // -- WP MERGEP1D -- the hub's installer lock for this device (LockState
        // heldBy `hub`), so the core refuses in the hub's words; the prompt is P1-E
        if (ok) actOnce([&](char *b, uint32_t n) { return dspi_installer_lock_refresh(c, b, n); });
        QMetaObject::invokeMethod(
            this,
            [this, reply, ok, serial] {
                track(-1);
                if (ok) afterBind(serial);
                else setError(str(reply, "code").isEmpty() ? QStringLiteral("io") : str(reply, "code"), str(reply, "error"));
                refreshStatus();
            },
            Qt::QueuedConnection);
    });
}

void LinkController::afterBind(const QString &serial) {
    if (m_boundSerial.isEmpty() && m_device && !m_device->remote())
        m_localBefore = m_device->demo() ? QStringLiteral("demo") : m_device->serial();
    m_boundSerial = serial;
    m_analyserWanted = m_device == nullptr;
    // The new connection starts the analyser (RtaController)
    if (m_device) m_device->adoptRemote(serial);
    if (m_poller) m_poller->setSlowTransport(true);
    emit devicesChanged();
    emit deviceBound(serial);
}

void LinkController::closeSession() {
    m_holdEnd = false;
    ++m_generation;
    m_closeWanted = true;
    dspi_link_close(m_core);   // returns within about 100 ms, also during a dial
    setError(QString(), QString());
    endSession(true);
    refreshStatus();
}

void LinkController::leaveForLocal() {
    m_holdEnd = false;
    ++m_generation;
    m_closeWanted = true;
    dspi_link_close(m_core);
    setError(QString(), QString());
    endSession(false);
    refreshStatus();
}

void LinkController::endSession(bool reopenLocal) {
    if (m_boundSerial.isEmpty()) return;
    m_boundSerial.clear();
    m_analyserWanted = false;
    if (m_poller) m_poller->setSlowTransport(false);
    if (m_device) {
        m_device->leaveRemote();
        if (reopenLocal && m_localBefore == QLatin1String("demo")) m_device->openDemo();
        else if (reopenLocal && !m_localBefore.isEmpty()) m_device->selectDevice(m_localBefore);
    }
    m_localBefore.clear();
    emit devicesChanged();
    emit sessionEnded();
}

void LinkController::retryNow() {
    if (m_shutDown) return;
    m_reader.start([this] { dspi_link_retry_now(m_core); });
    refreshStatus();
}

void LinkController::dismissTokenWarning() {
    if (m_tokenWarning.isEmpty()) return;
    m_tokenWarning.clear();
    emit tokenWarningChanged();
}

// ---- Encryption (-- WP SECQT --) ----------------------------------------------

bool LinkController::offersRePair(const QString &code) {
    return code == QLatin1String("hub_key_changed") || code == QLatin1String("hub_key_unverified")
           || code == QLatin1String("tls_refused") || code == QLatin1String("tls_required");
}

LinkController::Security LinkController::readSecurity(FfiCore *core) {
    const QJsonObject o = readObject([core](char *b, uint32_t n) { return dspi_link_security(core, b, n); });
    Security s;
    s.read = isOk(o);
    s.encrypted = o.value(QLatin1String("encrypted")).toBool();
    s.hubKeyNew = o.value(QLatin1String("hubKeyNew")).toBool();
    s.hubKey = str(o, "hubKey");
    s.credential = str(o, "credential");
    return s;
}

void LinkController::applySecurity(const Security &s) {
    if (!s.read) return;
    const bool changed = !m_securityKnown || s.encrypted != m_encrypted || s.hubKey != m_hubKey;
    m_securityKnown = true;
    m_encrypted = s.encrypted;
    m_hubKey = s.hubKey;
    if (changed) emit securityChanged();
    updateUnsecured();
    // The core changed the credential (a v1 token pinned at its first TLS
    // use, a rotation the old key signed): keep the new one
    if (!s.credential.isEmpty() && s.credential != m_credential && !m_hubId.isEmpty()) {
        m_credential = s.credential;
        storeToken(m_hubId, s.credential);
    }
    // A key the user has not seen yet: say so once
    if (s.encrypted && s.hubKeyNew && !s.hubKey.isEmpty() && s.hubKey != m_noticedKey) {
        m_noticedKey = s.hubKey;
        //: %1 is the hub key's fingerprint, SHA256:K7Q2-M9XD-…
        m_keyNotice = tr("Encrypted connection set up; hub key %1").arg(s.hubKey);
        emit keyNoticeChanged();
    }
}

void LinkController::clearSecurity() {
    if (!m_securityKnown && m_hubKey.isEmpty() && !m_encrypted) return;
    m_securityKnown = false;
    m_encrypted = false;
    m_hubKey.clear();
    emit securityChanged();
    updateUnsecured();
}

void LinkController::updateUnsecured() {
    const bool now = inSession() && m_securityKnown && !m_encrypted;
    if (now == m_unsecured) return;
    m_unsecured = now;
    emit securityChanged();
}

void LinkController::rePair() {
    if (!rePairOffered() || m_lastUrl.isEmpty()) return;
    const QString hub = m_failedHubId.isEmpty() ? m_lastExpected : m_failedHubId;
    dial(m_lastUrl, hub, m_lastTls, false);
}

void LinkController::dismissRePair() {
    if (rePairOffered()) setError(QString(), QString());
}

void LinkController::dismissKeyNotice() {
    if (m_keyNotice.isEmpty()) return;
    m_keyNotice.clear();
    emit keyNoticeChanged();
}
