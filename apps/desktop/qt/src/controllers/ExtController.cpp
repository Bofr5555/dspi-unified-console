// -- WP QP7 -- The extension channel's status and the crossfeed pair map (ExtController.h).
#include "ExtController.h"

#include "Core.h"
#include "DeviceModel.h"
#include "Snapshot.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QVariantMap>

static const Registrar::Controller<ExtController> extController;

namespace {

constexpr int kMapBytes = 4;
// Where the map sits in the bulk image (Mac: BULK_CROSSFEED_PAIR_MAP_OFFSET)
constexpr int kBulkPairMap = OFF_CROSSFEED + 12;

uint8_t pairByte(int left, int right) { return uint8_t(((right & 0x0F) << 4) | (left & 0x0F)); }

} // namespace

ExtController::ExtController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    connect(m_ctx.device, &DeviceModel::stateChanged, this, [this]() {
        const DeviceSnapshot &snap = m_ctx.device->snapshot();
        if (!snap.connected) {
            const bool was = m_known || m_hasPairMap || m_hasCoreVreg;
            m_known = m_hasPairMap = m_hasCoreVreg = false;
            m_supported.clear();
            m_map = QByteArray(kMapBytes, '\0');
            m_open = false;
            if (was) emit changed();
            return;
        }
        if (!m_open || snap.generation != m_generation) {
            // A new connection: the core probed it while connecting
            m_open = true;
            m_generation = snap.generation;
            refresh();
        } else if (takeMap()) {
            emit changed();
        }
    });
}

bool ExtController::takeMap() {
    // The map rides in the bulk image (the crossfeed section's offset 12), so a
    // preset load or another console's change arrives with the next snapshot,
    // without asking the device. Stock firmware has zero there.
    const DspState *s = m_ctx.device->state();
    if (!m_hasPairMap || !s || !s->bulk_valid) return false;
    const QByteArray map = normalized(QByteArray(reinterpret_cast<const char *>(s->bulk_raw) + kBulkPairMap, kMapBytes));
    if (map == m_map) return false;
    m_map = map;
    return true;
}

int ExtController::pairCount() const {
    const DspState *s = m_ctx.device->state();
    if (!s) return 0;
    return s->platform_id == 0 ? 2 : 4;
}

QVariantList ExtController::pairs() const {
    QVariantList out;
    for (int p = 0; p < pairCount() && p < m_map.size(); ++p) {
        const uint8_t b = uint8_t(m_map.at(p));
        const int left = b == 0 ? 2 * p : b & 0x0F;
        const int right = b == 0 ? 2 * p + 1 : b >> 4;
        out << QVariantMap{ { QStringLiteral("left"), left }, { QStringLiteral("right"), right } };
    }
    return out;
}

QByteArray ExtController::normalized(const QByteArray &map) {
    QByteArray out(kMapBytes, '\0');
    for (int p = 0; p < kMapBytes && p < map.size(); ++p) {
        const uint8_t b = uint8_t(map.at(p));
        out[p] = char(b == pairByte(2 * p, 2 * p + 1) ? 0 : b);
    }
    return out;
}

QString ExtController::choice() const {
    const QByteArray mine = normalized(m_map);
    if (mine == QByteArray(kMapBytes, '\0')) return QStringLiteral("default");
    QByteArray k340(kMapBytes, '\0');
    if (dspi_crossfeed_k340_pair_map(reinterpret_cast<uint8_t *>(k340.data()), kMapBytes) && mine == normalized(k340))
        return QStringLiteral("k340");
    return QStringLiteral("custom");
}

int ExtController::k340PairMask() { return dspi_crossfeed_k340_pair_mask(); }

QByteArray ExtController::encode(const QVariantList &pairs) const {
    const int n = pairCount();
    if (n < 1 || pairs.size() != n) return {};
    QByteArray map(kMapBytes, '\0');
    for (int p = 0; p < n; ++p) {
        const QVariantMap pair = pairs.at(p).toMap();
        bool okL = false, okR = false;
        const int left = pair.value(QStringLiteral("left")).toInt(&okL);
        const int right = pair.value(QStringLiteral("right")).toInt(&okR);
        // Checked on the pairs too: (0, 0) encodes as 0, the default pair
        if (!okL || !okR || left == right || left < 0 || left > 15 || right < 0 || right > 15) return {};
        map[p] = char(left == 2 * p && right == 2 * p + 1 ? 0 : pairByte(left, right));
    }
    if (!dspi_crossfeed_pair_map_valid(reinterpret_cast<const uint8_t *>(map.constData()), kMapBytes, uint8_t(n)))
        return {};
    return map;
}

bool ExtController::pairsValid(const QVariantList &pairs) const { return !encode(pairs).isEmpty(); }

bool ExtController::applyChoice(const QString &choice) {
    if (choice == QLatin1String("default")) return write(QByteArray(kMapBytes, '\0'));
    if (choice == QLatin1String("k340")) {
        QByteArray map(kMapBytes, '\0');
        return dspi_crossfeed_k340_pair_map(reinterpret_cast<uint8_t *>(map.data()), kMapBytes) && write(map);
    }
    return false;
}

bool ExtController::applyPairs(const QVariantList &pairs) {
    const QByteArray map = encode(pairs);
    return !map.isEmpty() && write(map);
}

bool ExtController::write(const QByteArray &map) {
    if (!m_hasPairMap || !m_ctx.device->connected() || m_ctx.device->readOnly()) return false;
    const int n = pairCount();
    if (!dspi_crossfeed_pair_map_valid(reinterpret_cast<const uint8_t *>(map.constData()), kMapBytes, uint8_t(n)))
        return false;
    const QByteArray bytes = normalized(map);
    ++m_busy;
    emit changed();
    m_ctx.device->write(
        QStringLiteral("crossfeed pair map"),
        [bytes](FfiCore *c) {
            return dspi_set_crossfeed_pair_map(c, reinterpret_cast<const uint8_t *>(bytes.constData()), kMapBytes) == 0;
        },
        [this](bool ok) {
            --m_busy;
            if (!ok) emit writeFailed();
            // What the device kept, either way: the core read it back into the image
            takeMap();
            emit changed();
        });
    return true;
}

void ExtController::refresh() {
    if (!m_ctx.device->connected()) return;
    ++m_busy;
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return readSized([c](char *b, uint32_t n) { return dspi_ext_status(c, b, n); }); },
        [this](const QByteArray &status) {
            --m_busy;
            readStatus(m_override.isEmpty() ? status : m_override);
            takeMap();
            emit changed();
        });
}

void ExtController::readStatus(const QByteArray &json) {
    const QJsonObject o = QJsonDocument::fromJson(json).object();
    m_known = o.value(QLatin1String("ok")).toBool();
    m_hasPairMap = m_known && o.value(QLatin1String("hasPairMap")).toBool();
    m_hasCoreVreg = m_known && o.value(QLatin1String("hasCoreVreg")).toBool();
    m_supported = o.value(QLatin1String("supported")).toArray().toVariantList();
    if (!m_hasPairMap) m_map = QByteArray(kMapBytes, '\0');
}

void ExtController::setStatusOverride(const QByteArray &json) {
    m_override = json;
    refresh();
}
