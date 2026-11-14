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

// What one read of the session gives (the reader thread hands it over whole)
struct LinkController::Status {
    QString phase = QStringLiteral("idle");
    int reconnectAttempt = 0;
    int reconnectInMs = 0;
    QString pinSource, closeReason, role;
    int session = 0;
    bool tokenRefused = false;
    QVariantList devices;
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
        QMetaObject::invokeMethod(link, [link, hubId, issued] { link->storeToken(hubId, issued); },
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

void LinkController::connectToHub(const QString &url, const QString &expectedHubId) {
    m_holdEnd = false;
    const unsigned generation = ++m_generation;
    m_closeWanted = false;
    setError(QString(), QString());
    const QByteArray u = url.toUtf8(), name = clientName(), version = AppIdentity::version().toUtf8();
    post([this, generation, u, name, version, expectedHubId] {
        FfiCore *c = m_core;
        const auto dial = [&](const QString &token, const QString &expected) {
            const QByteArray t = token.toUtf8(), e = expected.toUtf8();
            return actOnce([&](char *b, uint32_t n) {
                return dspi_link_connect(c, u.constData(), name.constData(), version.constData(), t.constData(),
                                         e.constData(), b, n);
            });
        };
        QJsonObject hello = dial(expectedHubId.isEmpty() ? QString() : m_store->load(expectedHubId), expectedHubId);
        // Dialled by address: the hub has named itself now, so a token stored
        // for that id may go to it, held to that id
        if (isOk(hello) && expectedHubId.isEmpty() && hello.value(QLatin1String("needsPairing")).toBool()
            && generation == m_generation) {
            const QString named = str(hello, "hubId");
            const QString stored = m_store->load(named);
            if (!stored.isEmpty()) hello = dial(stored, named);
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
        if (ok && hello.value(QLatin1String("tokenRefused")).toBool()) m_store->forget(str(hello, "hubId"));
        QMetaObject::invokeMethod(
            this,
            [this, hello, ok, expectedHubId] {
                track(-1);
                if (ok) {
                    applyHello(hello);
                    setError(QString(), QString());
                    if (hello.value(QLatin1String("tokenRefused")).toBool()) emit tokensChanged(m_hubId);
                } else {
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
                    if (!str(reply, "token").isEmpty()) storeToken(hub, str(reply, "token"));
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
