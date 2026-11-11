#include "Snapshot.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <cstring>

QByteArray readSized(const std::function<uint32_t(char *, uint32_t)> &fill, uint32_t first) {
    QByteArray buf(int(first), '\0');
    // A reply that grows between two calls asks again; three tries is plenty.
    for (int attempt = 0; attempt < 3; ++attempt) {
        const uint32_t need = fill(buf.data(), uint32_t(buf.size()));
        if (need == 0) return QByteArray();
        if (need <= uint32_t(buf.size())) {
            buf.truncate(int(need) - 1);
            return buf;
        }
        if (need > (64u << 20)) return QByteArray();
        buf = QByteArray(int(need), '\0');
    }
    return QByteArray();
}

QJsonValue jsonValue(const QByteArray &reply) {
    const QJsonObject o = QJsonDocument::fromJson(reply).object();
    if (!o.value(QLatin1String("ok")).toBool()) return QJsonValue();
    return o.value(QLatin1String("value"));
}

QString fieldText(const uint8_t *bytes, int size) {
    int n = 0;
    while (n < size && bytes[n] != 0) ++n;
    return QString::fromUtf8(reinterpret_cast<const char *>(bytes), n);
}

DeviceSnapshot readSnapshot(FfiCore *core) {
    DeviceSnapshot s;
    s.connected = dspi_is_connected(core);
    s.generation = dspi_connection_generation(core);
    if (!s.connected) return s;
    if (const DspState *record = dspi_get_state(core)) {
        auto copy = std::make_shared<DspState>();
        std::memcpy(copy.get(), record, sizeof(DspState));
        s.state = std::move(copy);
    }
    char name[64] = {};
    if (dspi_cached_device_name(core, name, sizeof name)) s.deviceName = QString::fromUtf8(name);
    s.outputLabels = jsonValue(readSized([core](char *b, uint32_t n) { return dspi_output_labels(core, b, n); }))
                         .toArray();
    s.wireFeatures = dspi_wire_features(core);
    return s;
}
