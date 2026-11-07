#include "HubController.h"
#include "DSPiBridge.h"
#include "DemoDevice.h"
#include "Registrar.h"

#include <QDir>
#include <QHostAddress>
#include <QJsonArray>
#include <QJsonDocument>
#include <QMetaObject>
#include <QNetworkInterface>
#include <QSettings>
#include <QStandardPaths>
#include <QSysInfo>
#include <utility>

static const Registrar::Controller<HubController> hubRegistered("hub");

namespace {

// A reply from the hub's C ABI, which answers in JSON (fns_hub.rs). An empty object
// for a panic. HUB_REPLY_MIN is what start and begin_pairing act on at all.
QJsonObject hubReply(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(int(HUB_REPLY_MIN), '\0');
    uint32_t needed = call(buf.data(), HUB_REPLY_MIN);
    if (needed > HUB_REPLY_MIN) {
        buf.resize(int(needed));
        needed = call(buf.data(), needed);
    }
    if (needed == 0 || needed > uint32_t(buf.size())) return {};
    return QJsonDocument::fromJson(QByteArray(buf.constData(), int(needed) - 1)).object();
}

QString failureOf(const QJsonObject &reply, const QString &fallback) {
    const QString text = reply.value(QStringLiteral("error")).toString();
    return text.isEmpty() ? fallback : text;
}

QString keyOf(const char *name) { return QStringLiteral("hub/") + QLatin1String(name); }

// Ports below 1024 need privileges on Linux; the Mac's page takes the same range
constexpr int kMinPort = 1024;
constexpr int kMaxPort = 65535;

} // namespace

HubController::HubController(DSPiBridge *bridge, QObject *parent)
    : TrayHub(parent), m_bridge(bridge), m_core(bridge->core()), m_hub(dspi_hub_new())
{
    m_worker.setMaxThreadCount(1);
    m_store = QDir(QStandardPaths::writableLocation(QStandardPaths::AppDataLocation)).filePath(QStringLiteral("hub")).toUtf8();

    QSettings s;
    m_sharing = s.value(keyOf("sharing"), false).toBool();
    m_access = s.value(keyOf("access"), QStringLiteral("pin")).toString() == QLatin1String("open") ? QStringLiteral("open")
                                                                                                    : QStringLiteral("pin");
    m_port = s.value(keyOf("port"), 0).toInt();
    m_lookForHubs = s.value(keyOf("lookForHubs"), true).toBool();
    m_name = s.value(keyOf("name")).toString();

    m_pinTick.setInterval(1000);
    connect(&m_pinTick, &QTimer::timeout, this, [this]() {
        if (m_pinExpiry.hasExpired()) refresh();
        emit pairingChanged();
    });

    if (m_hub) dspi_hub_set_listener(m_hub, &HubController::hubEvent, this);
    TrayController::setHub(this);
    readStore();
    // Sharing was on when the app last quit: the hub comes back with it
    if (m_sharing) start();
}

HubController::~HubController() {
    TrayController::setHub(nullptr);
    if (m_browsing) dspi_link_browse_stop(m_core);
    m_worker.waitForDone();
    if (m_hub) {
        // Waits for an event in progress; after it, none reaches this object
        dspi_hub_set_listener(m_hub, nullptr, nullptr);
        // Stops a running hub, which gives the app's device back: before the bridge's core goes
        dspi_hub_free(m_hub);
    }
}

void HubController::setStoreDir(const QString &dir) {
    m_store = dir.toUtf8();
    readStore();
}

// ── Reads ─────────────────────────────────────────────────────────

QString HubController::address() const {
    if (!m_running || m_boundPort <= 0) return {};
    QString host;
    if (m_extraOptions & HUB_OPT_LOOPBACK) {
        host = QStringLiteral("127.0.0.1");
    } else {
        for (const QNetworkInterface &i : QNetworkInterface::allInterfaces()) {
            const auto flags = i.flags();
            if (!(flags & QNetworkInterface::IsUp) || !(flags & QNetworkInterface::IsRunning) || (flags & QNetworkInterface::IsLoopBack))
                continue;
            for (const QNetworkAddressEntry &e : i.addressEntries())
                if (e.ip().protocol() == QAbstractSocket::IPv4Protocol && !e.ip().isLinkLocal()) {
                    host = e.ip().toString();
                    break;
                }
            if (!host.isEmpty()) break;
        }
        if (host.isEmpty()) host = QSysInfo::machineHostName();
    }
    return host + QLatin1Char(':') + QString::number(m_boundPort);
}

int HubController::pairingSecondsLeft() const {
    if (m_pin.isEmpty()) return 0;
    return int((m_pinExpiry.remainingTime() + 999) / 1000);
}

int HubController::revokeCost(const QString &mode) const {
    return mode == QLatin1String("open") ? m_revokeIfOpen : m_revokeIfPin;
}

TrayHub::State HubController::trayState() const {
    State s;
    s.running = m_running;
    s.sharing = m_sharing;
    s.address = address();
    s.pairingCode = m_pin;
    s.pinAccess = m_access == QLatin1String("pin");
    s.clients = m_connections;
    return s;
}

// The store, while no hub runs: name, id, clients, and what a mode change would cost.
// A file read, so on the worker.
void HubController::readStore() {
    const QByteArray store = m_store;
    apply([this, store]() {
        const QJsonObject saved = hubReply([&](char *b, uint32_t l) { return dspi_hub_saved_settings(store.constData(), b, l); });
        QMetaObject::invokeMethod(this, [this, saved]() { applySaved(saved); }, Qt::QueuedConnection);
    });
}

void HubController::applySaved(const QJsonObject &saved) {
    if (!saved.value(QStringLiteral("ok")).toBool()) return;
    m_revokeIfPin = saved.value(QStringLiteral("tokensRevokedIfPin")).toInt();
    m_revokeIfOpen = saved.value(QStringLiteral("tokensRevokedIfOpen")).toInt();
    if (m_port <= 0) m_port = saved.value(QStringLiteral("defaultPort")).toInt();
    if (m_name.isEmpty()) m_name = saved.value(QStringLiteral("name")).toString();
    if (m_name.isEmpty()) m_name = QSysInfo::machineHostName();
    if (!m_running) {
        m_hubId = saved.value(QStringLiteral("hubId")).toString();
        m_clients = saved.value(QStringLiteral("clients")).toArray().toVariantList();
        emit clientsChanged();
    }
    emitAll();
    syncBrowse();
}

void HubController::refresh() {
    m_readQueued = false;
    if (!m_hub) return;
    const QJsonObject status = hubReply([this](char *b, uint32_t l) { return dspi_hub_status(m_hub, b, l); });
    const bool running = status.value(QStringLiteral("running")).toBool();
    if (!running) {
        const bool was = m_running;
        m_running = false;
        m_boundPort = 0;
        m_connections = 0;
        setPairing(QString(), 0);
        if (was) readStore();
        emitAll();
        return;
    }
    m_running = true;
    m_hubId = status.value(QStringLiteral("hubId")).toString();
    m_boundPort = status.value(QStringLiteral("port")).toInt();
    m_connections = status.value(QStringLiteral("connections")).toInt();
    m_inaccessible = status.value(QStringLiteral("inaccessible")).toInt();
    if (!status.value(QStringLiteral("pairingOpen")).toBool()) setPairing(QString(), 0);
    const QJsonObject clients = hubReply([this](char *b, uint32_t l) { return dspi_hub_clients(m_hub, b, l); });
    const QVariantList list = clients.value(QStringLiteral("value")).toArray().toVariantList();
    if (list != m_clients) {
        m_clients = list;
        emit clientsChanged();
    }
    emitAll();
    syncBrowse();
}

void HubController::emitAll() {
    emit stateChanged();
    emit trayStateChanged();
}

void HubController::setError(const QString &text) {
    m_lastError = text;
    emitAll();
}

void HubController::dismissError() {
    m_lastError.clear();
    m_lastNote.clear();
    emitAll();
}

void HubController::setPairing(const QString &pin, int expiresInMs) {
    if (pin.isEmpty() && m_pin.isEmpty()) return;
    m_pin = pin;
    m_pinExpiry = QDeadlineTimer(pin.isEmpty() ? 0 : expiresInMs);
    if (pin.isEmpty()) m_pinTick.stop();
    else m_pinTick.start();
    emit pairingChanged();
    emit trayStateChanged();
}

// The hub's thread: schedule a read, once however many events come
void HubController::hubEvent(uint8_t, int32_t, const char *, void *userData) {
    auto *self = static_cast<HubController *>(userData);
    if (self->m_readQueued.exchange(true)) return;
    QMetaObject::invokeMethod(self, &HubController::refresh, Qt::QueuedConnection);
}

// ── The one apply path ────────────────────────────────────────────

void HubController::apply(std::function<void()> job) {
    m_calls++;
    emit stateChanged();
    m_worker.start([this, job = std::move(job)]() {
        job();
        QMetaObject::invokeMethod(this, [this]() {
            m_calls--;
            emit stateChanged();
        }, Qt::QueuedConnection);
    });
}

void HubController::setSharing(bool on) {
    if (on == m_sharing && (on ? m_running || busy() : !m_running)) return;
    m_sharing = on;
    QSettings().setValue(keyOf("sharing"), on);
    m_lastError.clear();
    m_lastNote.clear();
    if (on) start();
    else stop();
    emitAll();
}

void HubController::start() {
    if (!m_hub) {
        setError(QStringLiteral("Sharing is not available in this build."));
        return;
    }
    const QByteArray name = m_name.toUtf8(), store = m_store;
    const int port = m_port;
    const uint8_t access = m_access == QLatin1String("open") ? HUB_ACCESS_OPEN : HUB_ACCESS_PIN;
    const uint32_t options = HUB_OPT_USB | HUB_OPT_APP | m_extraOptions;
    // The demo device shown here goes on the hub as its own demo, to try sharing with
    // no hardware: the app's demo is not a USB device, which is all attach_app shares
    const bool demo = m_bridge->selectedSerial() == QLatin1String(DemoDevice::kSerial);
    FfiHub *hub = m_hub;
    FfiCore *core = m_core;
    apply([this, hub, core, name, store, port, access, options, demo]() {
        QJsonObject reply = hubReply([&](char *b, uint32_t l) {
            return dspi_hub_start(hub, name.constData(), store.constData(), uint16_t(port), access, options, b, l);
        });
        if (reply.value(QStringLiteral("ok")).toBool()) {
            if (!dspi_hub_attach_app(hub, core)) reply.insert(QStringLiteral("attachFailed"), true);
            if (demo) dspi_hub_attach_demo(hub);
        }
        QMetaObject::invokeMethod(this, [this, reply]() { onStarted(reply); }, Qt::QueuedConnection);
    });
}

void HubController::onStarted(const QJsonObject &reply) {
    if (!reply.value(QStringLiteral("ok")).toBool()) {
        // The switch stays on, as the Mac's does: the user sees why and turns it off
        if (reply.value(QStringLiteral("code")).toString() == QLatin1String("io"))
            setError(QStringLiteral("Could not share on port %1: %2").arg(m_port).arg(failureOf(reply, QStringLiteral("the port is in use"))));
        else
            setError(QStringLiteral("Could not start sharing: ") + failureOf(reply, QStringLiteral("the hub did not answer")));
        refresh();
        return;
    }
    const int revoked = reply.value(QStringLiteral("tokensRevoked")).toInt();
    if (revoked > 0)
        m_lastNote = revoked == 1 ? QStringLiteral("The access mode changed: 1 paired client must pair again.")
                                  : QStringLiteral("The access mode changed: %1 paired clients must pair again.").arg(revoked);
    if (reply.value(QStringLiteral("attachFailed")).toBool())
        m_lastError = QStringLiteral("Sharing started, but this app's own device could not be put on the hub.");
    // Switched off while starting: the stop queued behind this start comes next
    refresh();
}

void HubController::stop() {
    if (!m_hub) return;
    FfiHub *hub = m_hub;
    setPairing(QString(), 0);
    apply([this, hub]() {
        dspi_hub_stop(hub);
        QMetaObject::invokeMethod(this, &HubController::refresh, Qt::QueuedConnection);
    });
}

void HubController::restartIfRunning() {
    if (!m_sharing) {
        readStore();
        return;
    }
    stop();
    start();
}

// ── Settings ──────────────────────────────────────────────────────

bool HubController::setHubName(const QString &name) {
    QString clean;
    for (const QChar c : name)
        if (!c.isNonCharacter() && c.category() != QChar::Other_Control) clean.append(c);
    clean = clean.trimmed().left(64);
    if (clean.isEmpty()) {
        setError(QStringLiteral("A hub needs a name."));
        return false;
    }
    if (clean == m_name) return true;
    m_name = clean;
    QSettings().setValue(keyOf("name"), clean);
    emitAll();
    restartIfRunning();
    return true;
}

bool HubController::setAccess(const QString &mode) {
    if (mode != QLatin1String("pin") && mode != QLatin1String("open")) return false;
    if (mode == m_access) return true;
    m_access = mode;
    QSettings().setValue(keyOf("access"), mode);
    m_lastError.clear();
    emitAll();
    restartIfRunning();
    return true;
}

bool HubController::setPort(int port) {
    if (port < kMinPort || port > kMaxPort) {
        setError(QStringLiteral("A port from %1 to %2.").arg(kMinPort).arg(kMaxPort));
        return false;
    }
    if (m_running) {
        setError(QStringLiteral("Turn sharing off to change the port."));
        return false;
    }
    if (port == m_port) return true;
    m_port = port;
    QSettings().setValue(keyOf("port"), port);
    emitAll();
    return true;
}

// ── Pairing and clients ───────────────────────────────────────────

void HubController::allowNewClient() {
    if (!m_hub || !m_running || m_access != QLatin1String("pin")) return;
    FfiHub *hub = m_hub;
    apply([this, hub]() {
        const QJsonObject reply = hubReply([&](char *b, uint32_t l) { return dspi_hub_begin_pairing(hub, b, l); });
        QMetaObject::invokeMethod(this, [this, reply]() {
            if (reply.value(QStringLiteral("ok")).toBool())
                setPairing(reply.value(QStringLiteral("pin")).toString(), reply.value(QStringLiteral("expiresInMs")).toInt());
            else
                setError(QStringLiteral("Could not open pairing: ") + failureOf(reply, QStringLiteral("the hub did not answer")));
        }, Qt::QueuedConnection);
    });
}

void HubController::cancelPairing() {
    if (!m_hub) return;
    setPairing(QString(), 0);
    FfiHub *hub = m_hub;
    apply([hub]() { dspi_hub_cancel_pairing(hub); });
}

void HubController::setRole(int cid, const QString &role) {
    if (!m_hub || !m_running || cid < 0) return;
    FfiHub *hub = m_hub;
    const QByteArray r = role.toUtf8();
    apply([this, hub, cid, r]() {
        const bool ok = dspi_hub_set_role(hub, uint32_t(cid), r.constData());
        QMetaObject::invokeMethod(this, [this, ok]() {
            if (!ok) setError(QStringLiteral("Could not change that client's role."));
            refresh();
        }, Qt::QueuedConnection);
    });
}

void HubController::revoke(int cid) {
    if (!m_hub || !m_running || cid < 0) return;
    FfiHub *hub = m_hub;
    apply([this, hub, cid]() {
        const bool ok = dspi_hub_revoke(hub, uint32_t(cid));
        QMetaObject::invokeMethod(this, [this, ok]() {
            if (!ok) setError(QStringLiteral("Could not remove that client."));
            refresh();
        }, Qt::QueuedConnection);
    });
}

// ── Other hubs ────────────────────────────────────────────────────

void HubController::setLookForHubs(bool on) {
    if (on == m_lookForHubs) return;
    m_lookForHubs = on;
    QSettings().setValue(keyOf("lookForHubs"), on);
    emit browseChanged();
    syncBrowse();
}

void HubController::watch() {
    if (m_watchers++ > 0) return;
    emit browseChanged();
    refresh();
    syncBrowse();
}

void HubController::unwatch() {
    if (m_watchers == 0 || --m_watchers > 0) return;
    emit browseChanged();
    syncBrowse();
}

// Browse while the page shows and the user wants it, leaving this hub out. Starting
// returns at once; stopping waits only for a wake-up in progress, which schedules.
void HubController::syncBrowse() {
    const bool want = watching() && m_lookForHubs;
    if (m_browsing && (!want || m_browseOwnId != m_hubId)) {
        dspi_link_browse_stop(m_core);
        m_browsing = false;
        m_hubs.clear();
        m_browseError.clear();
        emit browseChanged();
    }
    if (want && !m_browsing) {
        m_browseOwnId = m_hubId;
        const QByteArray own = m_hubId.toUtf8();
        m_browsing = dspi_link_browse_start(m_core, &HubController::browseWake, this, own.constData());
        if (!m_browsing) m_browseError = QStringLiteral("Looking for hubs is not available in this build.");
        emit browseChanged();
        readHubs();
    }
}

void HubController::browseWake(void *userData) {
    auto *self = static_cast<HubController *>(userData);
    if (self->m_hubsQueued.exchange(true)) return;
    QMetaObject::invokeMethod(self, &HubController::readHubs, Qt::QueuedConnection);
}

void HubController::readHubs() {
    m_hubsQueued = false;
    if (!m_browsing) return;
    const QJsonObject reply = hubReply([this](char *b, uint32_t l) { return dspi_link_browse_hubs(m_core, b, l); });
    m_hubs = reply.value(QStringLiteral("hubs")).toArray().toVariantList();
    m_browseError = reply.value(QStringLiteral("error")).toString();
    emit browseChanged();
}
