#include "HubController.h"

#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DevicesController.h"
#include "SettingsController.h"
#include "Snapshot.h"

#include <QCoreApplication>
#include <QDir>
#include <QHostAddress>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QNetworkInterface>
#include <QPair>
#include <QSettings>
#include <QStandardPaths>
#include <utility>

static const Registrar::Controller<HubController> hubRegistered;
// Settings > Networking (Application, after Advanced); the terms are its rows
// and what people look for there (behaviour.md §9: "pin", "tray")
static const SettingsRegistry::Page networkingPage{
    "networking", SettingsRegistry::Application, 40, QT_TRANSLATE_NOOP("SettingsPages", "Networking"), "network",
    "NetworkingPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "Share Devices on This Network"), QT_TRANSLATE_NOOP("SettingsPages", "Sharing"),
      QT_TRANSLATE_NOOP("SettingsPages", "Hub name"), QT_TRANSLATE_NOOP("SettingsPages", "Access"),
      QT_TRANSLATE_NOOP("SettingsPages", "Port"), QT_TRANSLATE_NOOP("SettingsPages", "Paired Clients"),
      QT_TRANSLATE_NOOP("SettingsPages", "Pair a New Client"), QT_TRANSLATE_NOOP("SettingsPages", "Pairing code PIN"),
      QT_TRANSLATE_NOOP("SettingsPages", "Other Hubs"), QT_TRANSLATE_NOOP("SettingsPages", "Find hubs automatically"),
      QT_TRANSLATE_NOOP("SettingsPages", "Connect by Address"), QT_TRANSLATE_NOOP("SettingsPages", "Running in the Background"),
      QT_TRANSLATE_NOOP("SettingsPages", "Show in the tray"), QT_TRANSLATE_NOOP("SettingsPages", "Start in the tray only"),
      QT_TRANSLATE_NOOP("SettingsPages", "Open at login"), QT_TRANSLATE_NOOP("SettingsPages", "Keep awake while sharing"),
      QT_TRANSLATE_NOOP("SettingsPages", "Windows Firewall"), QT_TRANSLATE_NOOP("SettingsPages", "DSPi Link"),
      // -- WP SECQT --
      QT_TRANSLATE_NOOP("SettingsPages", "Hub key"), QT_TRANSLATE_NOOP("SettingsPages", "Upstream compatibility mode"),
      QT_TRANSLATE_NOOP("SettingsPages", "Encryption TLS") } };

namespace {

const char *const kSharingKey = "hub/sharing";
const char *const kNameKey = "hub/name";
const char *const kPortKey = "hub/port";
const char *const kAccessKey = "hub/access";
const char *const kCompatKey = "hub/upstreamCompat";   // -- WP SECQT --
constexpr int kDefaultPort = 11915;   // the Mac's LinkDiscovery.defaultPort

QJsonObject parse(const QByteArray &reply) { return QJsonDocument::fromJson(reply).object(); }

// For the two calls that act (start, pairing): one buffer of the size they
// promise to fit, never a second call (fns_hub.rs, "Short buffers")
QJsonObject actOnce(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(int(HUB_REPLY_MIN), '\0');
    const uint32_t needed = call(buf.data(), HUB_REPLY_MIN);
    if (needed == 0 || needed > HUB_REPLY_MIN) return {};
    return parse(QByteArray(buf.constData(), int(needed) - 1));
}

QString errorOf(const QJsonObject &reply) { return reply.value(QLatin1String("error")).toString(); }

// The address a client on the LAN would type: the first IPv4 address of an
// interface that is up, preferring the private ranges (the Mac's
// primaryLANAddress)
QString lanAddress() {
    QString fallback;
    for (const QNetworkInterface &nic : QNetworkInterface::allInterfaces()) {
        const auto flags = nic.flags();
        if (!(flags & QNetworkInterface::IsUp) || !(flags & QNetworkInterface::IsRunning) ||
            (flags & QNetworkInterface::IsLoopBack))
            continue;
        for (const QNetworkAddressEntry &entry : nic.addressEntries()) {
            const QHostAddress ip = entry.ip();
            if (ip.protocol() != QAbstractSocket::IPv4Protocol) continue;
            const QString text = ip.toString();
            if (text.startsWith(QLatin1String("192.168.")) || text.startsWith(QLatin1String("10.")) ||
                text.startsWith(QLatin1String("172.")))
                return text;
            if (fallback.isEmpty()) fallback = text;
        }
    }
    return fallback;
}

QVariantList clientsOf(const QJsonArray &list) {
    QVariantList out;
    for (const QJsonValue &v : list) {
        const QJsonObject c = v.toObject();
        out.append(QVariantMap{
            { QStringLiteral("cid"), c.value(QLatin1String("cid")).toInt() },
            { QStringLiteral("name"), c.value(QLatin1String("name")).toString() },
            { QStringLiteral("role"), c.value(QLatin1String("role")).toString() },
            { QStringLiteral("lastSeen"), c.value(QLatin1String("lastSeen")).toDouble(0) },
            { QStringLiteral("online"), c.value(QLatin1String("online")).toBool() },
        });
    }
    return out;
}

} // namespace

struct HubController::StartResult {
    QJsonObject reply;
    int demo = -1;
};

HubController::HubController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_core(ctx.core), m_device(ctx.device) {
    m_thread.setMaxThreadCount(1);
    m_thread.setExpiryTimeout(-1);
    m_storeDir = QDir(QStandardPaths::writableLocation(QStandardPaths::AppDataLocation))
                     .filePath(QStringLiteral("hub"));
    QSettings s;
    m_sharing = s.value(QLatin1String(kSharingKey), false).toBool();
    m_nameSetting = s.value(QLatin1String(kNameKey)).toString();
    m_hubName = m_nameSetting;
    m_port = s.value(QLatin1String(kPortKey), kDefaultPort).toInt();
    const QString access = s.value(QLatin1String(kAccessKey)).toString();
    if (access == QLatin1String("pin") || access == QLatin1String("none")) m_access = access;
    // -- WP SECQT -- compatibility mode, and the hub's list of what it turns off
    m_upstreamCompat = s.value(QLatin1String(kCompatKey), false).toBool();
    const QJsonObject effects =
        parse(readSized([](char *b, uint32_t n) { return dspi_hub_upstream_compat_effects(b, n); }));
    for (const QJsonValue &line : effects.value(QLatin1String("effects")).toArray())
        m_compatEffects << line.toString();

    m_hub = dspi_hub_new();
    if (m_hub) dspi_hub_set_listener(m_hub, &HubController::eventThunk, this);

    m_pairingTick.setInterval(1000);
    connect(&m_pairingTick, &QTimer::timeout, this, [this]() {
        if (pairingSecondsLeft() <= 0) closePairing();
        emit pairingChanged();
    });
    // The demo opened after sharing began: the hub gets its own as well
    connect(m_device, &DeviceModel::snapshotApplied, this, [this]() {
        if (!m_running || m_busy || m_ownDemo >= 0 || !m_device->demo()) return;
        FfiHub *hub = m_hub;
        onHubThread([hub]() { return dspi_hub_attach_demo(hub); },
                    [this](int handle) {
                        if (handle < 0 || !m_running) return;
                        m_ownDemo = handle;
                        emit stateChanged();
                    });
    });
    // The hub must stop before the core is freed, which main() does after the
    // event loop: stop it on the way out
    connect(QCoreApplication::instance(), &QCoreApplication::aboutToQuit, this, &HubController::shutdown);

    readSaved();
    if (m_sharing) QTimer::singleShot(0, this, &HubController::reconcile);
}

HubController::~HubController() { shutdown(); }

void HubController::shutdown() {
    if (m_shutDown) return;
    m_shutDown = true;
    m_pairingTick.stop();
    m_thread.waitForDone();
    if (!m_hub) return;
    // Waits for the last callback; the queued ones die with this object
    dspi_hub_stop(m_hub);
    dspi_hub_set_listener(m_hub, nullptr, nullptr);
    dspi_hub_free(m_hub);
    m_hub = nullptr;
}

template <class Job, class Done>
void HubController::onHubThread(Job job, Done done) {
    if (m_shutDown || !m_hub) return;
    m_thread.start([this, job = std::move(job), done = std::move(done)]() mutable {
        auto result = job();
        QMetaObject::invokeMethod(
            this, [done = std::move(done), result = std::move(result)]() mutable { done(std::move(result)); },
            Qt::QueuedConnection);
    });
}

// ---- Events -------------------------------------------------------------------

// The hub's event thread: schedule and return
void HubController::eventThunk(uint8_t event, int32_t code, const char *text, void *self) {
    auto *hub = static_cast<HubController *>(self);
    const QString words = text ? QString::fromUtf8(text) : QString();
    QMetaObject::invokeMethod(hub, [hub, event, code, words]() { hub->onEvent(event, code, words); },
                              Qt::QueuedConnection);
}

void HubController::onEvent(int event, int code, const QString &text) {
    switch (event) {
    case HUB_EVENT_TLS_KEY:   // -- WP SECQT -- a new key or certificate: its fingerprint
        if (!text.isEmpty() && text != m_tlsFingerprint) {
            m_tlsFingerprint = text;
            emit stateChanged();
        }
        return;
    case HUB_EVENT_SESSION_OPENED:
        m_sessions.insert(code);
        emit stateChanged();
        // -- WP QPI2 -- a client that just paired holds a token: count it in
        // what a change of access would cost
        readSaved();
        break;
    case HUB_EVENT_SESSION_CLOSED:
        m_sessions.remove(code);
        emit stateChanged();
        break;
    case HUB_EVENT_STOPPED:
        m_sessions.clear();
        emit stateChanged();
        return;
    default:
        break;
    }
    // A session opening is also how a finished pairing shows
    readLive();
}

// ---- Starting and stopping ---------------------------------------------------------

void HubController::setSharing(bool on) {
    if (on == m_sharing) return;
    m_sharing = on;
    QSettings().setValue(QLatin1String(kSharingKey), on);
    if (!on) m_lastError.clear();   // a failed start is tried again at the next switch-on
    emit sharingChanged();
    emit stateChanged();
    reconcile();
}

void HubController::reconcile() {
    if (m_shutDown || m_busy || !m_hub) return;
    if (m_running && (!m_sharing || m_restart)) stopHub();
    else if (!m_running && m_sharing && m_lastError.isEmpty()) startHub();
}

void HubController::startHub() {
    m_busy = true;
    m_restart = false;
    emit stateChanged();
    FfiHub *hub = m_hub;
    const QByteArray name = m_nameSetting.toUtf8();
    const QByteArray dir = QDir::toNativeSeparators(m_storeDir).toUtf8();
    const uint16_t port = uint16_t(m_port);
    const uint8_t access = m_access == QLatin1String("none") ? HUB_ACCESS_OPEN : HUB_ACCESS_PIN;
    const uint32_t options = startOptions(m_extraOptions, m_upstreamCompat);   // -- WP SECQT --
    const bool demo = m_device->demo();
    onHubThread(
        [=]() {
            StartResult r;
            r.reply = actOnce([&](char *b, uint32_t n) {
                return dspi_hub_start(hub, name.constData(), dir.constData(), port, access, options, b, n);
            });
            if (r.reply.value(QLatin1String("ok")).toBool() && demo) r.demo = dspi_hub_attach_demo(hub);
            return r;
        },
        [this](StartResult r) {
            m_busy = false;
            if (!r.reply.value(QLatin1String("ok")).toBool()) {
                // The core's sentence names the port and the system's reason
                const QString why = errorOf(r.reply);
                m_lastError = why.isEmpty() ? tr("Could not start on port %1.").arg(m_port) : why;
                emit stateChanged();
                readSaved();
                return;
            }
            m_running = true;
            m_lastError.clear();
            m_hubId = r.reply.value(QLatin1String("hubId")).toString();
            m_boundPort = r.reply.value(QLatin1String("port")).toInt();
            m_lanAddress = lanAddress(); // -- WP QPI2 -- once per start, not per tray refresh
            m_ownDemo = r.demo;
            // -- WP SECQT -- the TLS mode in force and the key clients pin
            m_tlsMode = r.reply.value(QLatin1String("tls")).toString();
            m_tlsFingerprint = r.reply.value(QLatin1String("tlsFingerprint")).toString();
            const QString name = r.reply.value(QLatin1String("name")).toString();
            if (!name.isEmpty() && name != m_hubName) {
                m_hubName = name;
                emit settingsChanged();
            }
            emit stateChanged();
            // This app's own hub is not one of the "other" hubs
            if (auto *devices = Registrar::instance<DevicesController>()) devices->setOwnHubId(m_hubId);
            // The app's devices go through the hub from now on; the core reopens the
            // open one, so it is read again
            FfiHub *hub = m_hub;
            m_core->worker()->call(this, [hub](FfiCore *c) { return dspi_hub_attach_app(hub, c); },
                                   [this](bool attached) {
                                       if (!attached) qWarning("HubController: the app did not attach to the hub");
                                       m_device->refresh();
                                   });
            readSaved();
            readLive();
            reconcile();   // switched off or renamed meanwhile
        });
}

void HubController::stopHub() {
    m_busy = true;
    emit stateChanged();
    FfiHub *hub = m_hub;
    onHubThread([hub]() { return dspi_hub_stop(hub); },
                [this](bool) {
                    m_busy = false;
                    m_running = false;
                    m_ownDemo = -1;
                    m_boundPort = 0;
                    m_sessions.clear();
                    closePairing();
                    emit stateChanged();
                    if (auto *devices = Registrar::instance<DevicesController>()) devices->setOwnHubId(QString());
                    // The app has its direct path back, and its device reopened
                    m_device->refresh();
                    readSaved();
                    reconcile();   // a restart, or switched on again meanwhile
                });
}

void HubController::dismissError() {
    if (m_lastError.isEmpty()) return;
    m_lastError.clear();
    emit stateChanged();
}

void HubController::setError(const QString &error) {
    m_lastError = error;
    emit stateChanged();
}

// ---- Settings -----------------------------------------------------------------

void HubController::setHubName(const QString &name) {
    const QString clean = name.trimmed();
    if (clean.isEmpty() || clean == m_hubName) return;
    m_nameSetting = m_hubName = clean;
    QSettings().setValue(QLatin1String(kNameKey), clean);
    emit settingsChanged();
    if (m_running || m_busy) m_restart = true;
    reconcile();
}

void HubController::setAccess(const QString &mode) {
    if ((mode != QLatin1String("pin") && mode != QLatin1String("none")) || mode == m_access) return;
    m_access = mode;
    QSettings().setValue(QLatin1String(kAccessKey), mode);
    emit settingsChanged();
    if (m_running || m_busy) m_restart = true;
    reconcile();
}

// -- WP SECQT --
void HubController::setUpstreamCompat(bool on) {
    if (on == m_upstreamCompat) return;
    m_upstreamCompat = on;
    QSettings().setValue(QLatin1String(kCompatKey), on);
    emit settingsChanged();
    if (m_running || m_busy) m_restart = true;
    reconcile();
}

uint32_t HubController::startOptions(uint32_t extra, bool upstreamCompat) {
    const uint32_t tlsBits = HUB_OPT_TLS_ON | HUB_OPT_TLS_REQUIRE | HUB_OPT_UPSTREAM_COMPAT;
    const uint32_t options = HUB_OPT_USB | HUB_OPT_APP | extra;
    if (upstreamCompat) return (options & ~tlsBits) | HUB_OPT_UPSTREAM_COMPAT;
    if (options & tlsBits) return options;
    if (options & HUB_OPT_LOOPBACK) return options;
    return options | HUB_OPT_TLS_REQUIRE;
}

int HubController::revokeCost(const QString &mode) const { return m_revokeCosts.value(mode).toInt(); }

bool HubController::setPort(int port) {
    if (m_running || m_busy) {
        setError(tr("Turn sharing off to change the port."));
        return false;
    }
    if (port < 1024 || port > 65535) return false;
    if (port != m_port) {
        m_port = port;
        QSettings().setValue(QLatin1String(kPortKey), port);
        emit settingsChanged();
    }
    return true;
}

void HubController::setStoreDir(const QString &dir) {
    m_storeDir = dir;
    readSaved();
}

// ---- Reads ----------------------------------------------------------------------

void HubController::refresh() {
    readSaved();
    if (m_running) readLive();
}

// The store, with or without a hub on it: the name, access and clients while
// stopped, and always what a change of access would cost
void HubController::readSaved() {
    const QByteArray dir = QDir::toNativeSeparators(m_storeDir).toUtf8();
    onHubThread(
        [dir]() {
            return parse(readSized([&](char *b, uint32_t n) { return dspi_hub_saved_settings(dir.constData(), b, n); }));
        },
        [this](QJsonObject o) {
            if (!o.value(QLatin1String("ok")).toBool()) return;
            m_revokeCosts = { { QStringLiteral("pin"), o.value(QLatin1String("tokensRevokedIfPin")).toInt() },
                              { QStringLiteral("none"), o.value(QLatin1String("tokensRevokedIfOpen")).toInt() } };
            if (!m_running && !m_busy) {
                const QString name = o.value(QLatin1String("name")).toString();
                if (m_nameSetting.isEmpty() && name != m_hubName) {
                    m_hubName = name;
                    emit settingsChanged();
                }
                const QString access = o.value(QLatin1String("access")).toString();
                if (!QSettings().contains(QLatin1String(kAccessKey)) &&
                    (access == QLatin1String("pin") || access == QLatin1String("none")) && access != m_access) {
                    m_access = access;
                    emit settingsChanged();
                }
                m_hubId = o.value(QLatin1String("hubId")).toString();
                m_clients = clientsOf(o.value(QLatin1String("clients")).toArray());
            }
            emit clientsChanged();
        });
}

void HubController::readLive() {
    if (!m_running || m_readQueued.exchange(true)) return;
    FfiHub *hub = m_hub;
    onHubThread(
        [this, hub]() {
            m_readQueued = false;
            return QPair<QJsonObject, QJsonObject>(
                parse(readSized([hub](char *b, uint32_t n) { return dspi_hub_status(hub, b, n); })),
                parse(readSized([hub](char *b, uint32_t n) { return dspi_hub_clients(hub, b, n); })));
        },
        [this](QPair<QJsonObject, QJsonObject> r) {
            if (!m_running) return;
            const QJsonObject &status = r.first;
            if (status.value(QLatin1String("ok")).toBool() && !m_pairingCode.isEmpty() &&
                !status.value(QLatin1String("pairingOpen")).toBool())
                closePairing();   // used by a client that paired, or five wrong PINs
            const QVariantList clients = clientsOf(r.second.value(QLatin1String("value")).toArray());
            if (r.second.value(QLatin1String("ok")).toBool() && clients != m_clients) {
                m_clients = clients;
                emit clientsChanged();
            }
            if (m_hubId.isEmpty()) {
                m_hubId = status.value(QLatin1String("hubId")).toString();
                emit stateChanged();
            }
        });
}

// ---- Pairing and clients ---------------------------------------------------------

void HubController::allowNewClient() {
    if (!m_running) return;
    FfiHub *hub = m_hub;
    onHubThread([hub]() { return actOnce([hub](char *b, uint32_t n) { return dspi_hub_begin_pairing(hub, b, n); }); },
                [this](QJsonObject o) {
                    if (!o.value(QLatin1String("ok")).toBool()) {
                        setError(errorOf(o));
                        return;
                    }
                    m_pairingCode = o.value(QLatin1String("pin")).toString();
                    m_pairingEnds = QDateTime::currentDateTimeUtc().addMSecs(
                        qint64(o.value(QLatin1String("expiresInMs")).toDouble()));
                    m_pairingTick.start();
                    emit pairingChanged();
                });
}

void HubController::cancelPairing() {
    if (m_pairingCode.isEmpty()) return;
    closePairing();
    FfiHub *hub = m_hub;
    onHubThread([hub]() { return dspi_hub_cancel_pairing(hub); }, [](bool) {});
}

void HubController::closePairing() {
    m_pairingTick.stop();
    if (m_pairingCode.isEmpty()) return;
    m_pairingCode.clear();
    m_pairingEnds = QDateTime();
    emit pairingChanged();
}

int HubController::pairingSecondsLeft() const {
    if (m_pairingCode.isEmpty()) return 0;
    return int(qMax<qint64>(0, (QDateTime::currentDateTimeUtc().msecsTo(m_pairingEnds) + 999) / 1000));
}

void HubController::revoke(int cid) {
    if (!m_running || cid < 0) return;
    FfiHub *hub = m_hub;
    onHubThread([hub, cid]() { return dspi_hub_revoke(hub, uint32_t(cid)); }, [this](bool) {
        readLive();
        readSaved();
    });
}

void HubController::setRole(int cid, const QString &role) {
    if (!m_running || cid < 0) return;
    if (role != QLatin1String("viewer") && role != QLatin1String("control") && role != QLatin1String("admin")) return;
    FfiHub *hub = m_hub;
    const QByteArray r = role.toUtf8();
    onHubThread([hub, cid, r]() { return dspi_hub_set_role(hub, uint32_t(cid), r.constData()); },
                [this](bool) { readLive(); });
}

// ---- What the page and the tray show ---------------------------------------------

QString HubController::address() const {
    if (!m_running || m_boundPort <= 0) return QString();
    const QString ip = (m_extraOptions & HUB_OPT_LOOPBACK) ? QStringLiteral("127.0.0.1") : m_lanAddress;
    return ip.isEmpty() ? QString() : QStringLiteral("%1:%2").arg(ip).arg(m_boundPort);
}

QString HubController::statusLine() const {
    const int count = sessions();
    const QString clients = count == 0   ? tr("No clients connected")
                            : count == 1 ? tr("1 client connected")
                                         : tr("%1 clients connected").arg(count);
    const QString at = address();
    return at.isEmpty() ? clients : QStringLiteral("%1 · %2").arg(at, clients);
}
