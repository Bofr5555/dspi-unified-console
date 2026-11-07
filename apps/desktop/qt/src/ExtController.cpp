#include "ExtController.h"
#include "DSPiBridge.h"
#include "Registrar.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QMetaObject>
#include <QVariantMap>
#include <memory>
#include <utility>

static const Registrar::Controller<ExtController> extRegistered("ext");

namespace {

// How long changes gather before one re-read; a slider's commits come in bursts
constexpr int kRefreshDelayMs = 150;

// Byte p is pair p: 0 is the default pairing (2p, 2p+1), else (right << 4) | left
void decodePair(const QByteArray &map, int p, int *left, int *right) {
    const auto b = uint8_t(map.at(p));
    *left = b == 0 ? 2 * p : b & 0x0F;
    *right = b == 0 ? 2 * p + 1 : b >> 4;
}

// The core's rule, for telling the editor as the pairs are picked
bool mapValid(const QByteArray &map, int pairs) {
    return dspi_crossfeed_pair_map_valid(reinterpret_cast<const uint8_t *>(map.constData()), uint32_t(map.size()), uint8_t(pairs));
}

QByteArray defaultMap() { return QByteArray(4, '\0'); }

// One DAC per ear: woofers 3/1, tweeters 4/2
QByteArray k340Map() {
    QByteArray map = defaultMap();
    dspi_crossfeed_k340_pair_map(reinterpret_cast<uint8_t *>(map.data()), uint32_t(map.size()));
    return map;
}

// Explicit default bytes as 0, as the device reports them
QByteArray normalized(QByteArray map) {
    for (int p = 0; p < map.size(); p++)
        if (uint8_t(map.at(p)) == (((2 * p + 1) << 4) | (2 * p))) map[p] = 0;
    return map;
}

} // namespace

ExtController::ExtController(DSPiBridge *bridge, QObject *parent)
    : QObject(parent), m_bridge(bridge), m_core(bridge->core()), m_map(defaultMap())
{
    m_worker.setMaxThreadCount(1);
    m_refreshTimer.setSingleShot(true);
    m_refreshTimer.setInterval(kRefreshDelayMs);
    connect(&m_refreshTimer, &QTimer::timeout, this, &ExtController::refresh);
    connect(m_bridge, &DSPiBridge::stateChanged, this, &ExtController::syncStatus);
    connect(m_bridge, &DSPiBridge::devicesChanged, this, &ExtController::syncStatus);
    // Not every batch changes the state the bridge shows, and the map isn't in it
    connect(m_bridge, &DSPiBridge::notificationsArrived, this, &ExtController::refreshSoon);
    // -- WP QD6 -- Before the next device opens, not after: stateChanged comes too late
    connect(m_bridge, &DSPiBridge::deviceChanging, this, [this]() { m_generation++; });
    syncStatus();
}

ExtController::~ExtController() {
    // A result still on its way is dropped with this object
    m_worker.waitForDone();
}

QVariantList ExtController::pairs() const {
    QVariantList list;
    for (int p = 0; p < m_pairCount; p++) {
        int left, right;
        decodePair(m_map, p, &left, &right);
        QVariantMap pair;
        pair.insert(QStringLiteral("left"), left);
        pair.insert(QStringLiteral("right"), right);
        list.append(pair);
    }
    return list;
}

QString ExtController::choice() const {
    if (m_map == defaultMap()) return QStringLiteral("default");
    if (m_map == k340Map()) return QStringLiteral("k340");
    return QStringLiteral("custom");
}

void ExtController::setWatching(bool on) {
    if (on == m_watching) return;
    m_watching = on;
    emit watchingChanged();
    if (on) refresh();
    else m_refreshTimer.stop();
}

// A map for these pairs, whether or not the device would take it. A pair on one
// output twice stops here: (0, 0) would encode as 0, the default pair.
QByteArray ExtController::pairsToMap(const QVariantList &pairs, bool *ok) const {
    QByteArray map = defaultMap();
    *ok = pairs.size() == m_pairCount;
    for (int p = 0; *ok && p < m_pairCount; p++) {
        const QVariantMap pair = pairs.at(p).toMap();
        bool leftOk = false, rightOk = false;
        const int left = pair.value(QStringLiteral("left")).toInt(&leftOk);
        const int right = pair.value(QStringLiteral("right")).toInt(&rightOk);
        *ok = leftOk && rightOk && left >= 0 && left < 16 && right >= 0 && right < 16 && left != right;
        if (*ok) map[p] = char((right << 4) | left);
    }
    return normalized(map);
}

bool ExtController::pairsValid(const QVariantList &pairs) const {
    bool ok = false;
    const QByteArray map = pairsToMap(pairs, &ok);
    return ok && mapValid(map, m_pairCount);
}

bool ExtController::applyPairs(const QVariantList &pairs) {
    if (!m_bridge->writable()) return false;   // -- WP QD6 --
    bool ok = false;
    const QByteArray map = pairsToMap(pairs, &ok);
    if (!m_hasPairMap || !ok || !mapValid(map, m_pairCount)) {
        setError(QStringLiteral("Each pair needs two different outputs, and no output can be in two pairs."));
        return false;
    }
    applyMap(map);
    return true;
}

bool ExtController::applyPreset(const QString &name) {
    if (!m_bridge->writable()) return false;   // -- WP QD6 --
    if (m_hasPairMap && name == QLatin1String("default")) applyMap(defaultMap());
    else if (m_hasPairMap && name == QLatin1String("k340")) applyMap(k340Map());
    else return false;
    return true;
}

// Every write to the device. It is read back whatever the answer, so the window
// shows the map the device kept.
void ExtController::applyMap(const QByteArray &map) {
    m_lastError.clear();
    m_writes++;
    emit changed();
    FfiCore *core = m_core;
    const quint64 generation = m_generation;
    const auto wrote = std::make_shared<bool>(false);
    QByteArray bytes = map;
    onWorker([this, core, bytes, wrote, generation](uint8_t *out) {
        const bool hookOk = !m_writeHook || m_writeHook();
        // A changed device gets none of the old one's map, however long this waited
        if (m_generation != generation) return false;
        if (hookOk)
            *wrote = dspi_set_crossfeed_pair_map(core, reinterpret_cast<const uint8_t *>(bytes.constData()), kMapLen) == 0;
        return dspi_fetch_crossfeed_pair_map(core, out, kMapLen);
    }, [this, wrote, generation](bool read, const QByteArray &now) {
        m_writes--;
        if (generation == m_generation) {
            if (read) setMap(now);
            if (!*wrote) setError(QStringLiteral("The pair map was not applied."));
        }
        emit changed();
        emit applyFinished(*wrote);
    });
}

void ExtController::refresh() {
    if (!m_hasPairMap || m_refreshPending) return;
    m_refreshPending = true;
    FfiCore *core = m_core;
    const quint64 generation = m_generation;
    onWorker([core](uint8_t *out) {
        return dspi_fetch_crossfeed_pair_map(core, out, kMapLen);
    }, [this, generation](bool read, const QByteArray &now) {
        m_refreshPending = false;
        if (generation != m_generation) refresh();
        else if (read) setMap(now);
    });
}

void ExtController::refreshSoon() {
    if (m_watching && m_hasPairMap && !m_refreshTimer.isActive()) m_refreshTimer.start();
}

void ExtController::onWorker(std::function<bool(uint8_t *)> job, std::function<void(bool, QByteArray)> done) {
    m_worker.start([this, job = std::move(job), done = std::move(done)]() {
        uint8_t out[kMapLen] = {};
        const bool ok = job(out);
        const QByteArray bytes(reinterpret_cast<const char *>(out), kMapLen);
        QMetaObject::invokeMethod(this, [done, ok, bytes]() { done(ok, bytes); }, Qt::QueuedConnection);
    });
}

void ExtController::setMap(const QByteArray &map) {
    if (map == m_map) return;
    m_map = map;
    emit changed();
}

void ExtController::setError(const QString &error) {
    if (error == m_lastError) return;
    m_lastError = error;
    emit changed();
}

void ExtController::clearError() { setError(QString()); }

void ExtController::setStatusOverride(const QByteArray &json) {
    m_statusOverride = json;
    syncStatus();
}

// What the core found on connect; no traffic. Follows the bridge's signals, since
// nothing else says a device came or went.
void ExtController::syncStatus() {
    bool has = false;
    if (m_bridge->connected()) {
        QByteArray reply = m_statusOverride;
        if (reply.isEmpty()) {
            reply.resize(256);
            uint32_t needed = dspi_ext_status(m_core, reply.data(), uint32_t(reply.size()));
            if (needed > uint32_t(reply.size())) {
                reply.resize(int(needed));
                needed = dspi_ext_status(m_core, reply.data(), uint32_t(reply.size()));
            }
            reply.truncate(needed > 0 ? int(needed) - 1 : 0);
        }
        const QJsonObject status = QJsonDocument::fromJson(reply).object();
        has = status.value(QStringLiteral("ok")).toBool() && status.value(QStringLiteral("hasPairMap")).toBool();
    }

    const QString serial = m_bridge->selectedSerial();
    const int count = qBound(1, m_bridge->numOutputChannels() / 2, kMapLen);
    const bool sameDevice = serial == m_serial;
    if (sameDevice && has == m_hasPairMap && count == m_pairCount) {
        refreshSoon();
        return;
    }
    if (!sameDevice) {
        m_generation++;
        m_serial = serial;
        m_map = defaultMap();
        m_lastError.clear();
    }
    const bool appeared = has && (!m_hasPairMap || !sameDevice);
    m_hasPairMap = has;
    m_pairCount = count;
    emit changed();
    if (appeared) refresh();
}
