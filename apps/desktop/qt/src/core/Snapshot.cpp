#include "Snapshot.h"

#include <QByteArrayView>
#include <QJsonDocument>
#include <QJsonObject>
#include <array>

QByteArray readSized(const SizedFill &fill, uint32_t first) {
    // The call answers with the length it needs, NUL included. A reply that
    // grew in between is asked for again, three times at most, and nothing
    // beyond 64 MiB is believed.
    constexpr uint32_t ceiling = 64u << 20;
    QByteArray out;
    uint32_t room = first;
    for (int tries = 3; tries > 0 && room <= ceiling; --tries) {
        out.resize(qsizetype(room));
        const uint32_t wanted = fill(out.data(), room);
        if (wanted == 0) break;
        if (wanted <= room) {
            out.resize(qsizetype(wanted) - 1);
            return out;
        }
        room = wanted;
    }
    return {};
}

QJsonValue jsonValue(const QByteArray &reply) {
    const QJsonObject envelope = QJsonDocument::fromJson(reply).object();
    return envelope.value(QStringLiteral("ok")).toBool() ? envelope.value(QStringLiteral("value")) : QJsonValue();
}

QString fieldText(const uint8_t *bytes, int size) {
    const QByteArrayView field(bytes, size);
    const qsizetype end = field.indexOf('\0');
    return QString::fromUtf8(end < 0 ? field : field.first(end));
}

DeviceSnapshot readSnapshot(FfiCore *core) {
    DeviceSnapshot snap;
    snap.connected = dspi_is_connected(core);
    snap.generation = dspi_connection_generation(core);
    if (snap.connected) {
        // A copy: the record is the core's and changes under the next call.
        if (const DspState *live = dspi_get_state(core)) snap.state = std::make_shared<const DspState>(*live);
        std::array<char, 64> name{};
        if (dspi_cached_device_name(core, name.data(), uint32_t(name.size()))) snap.deviceName = QString::fromUtf8(name.data());
        snap.outputLabels =
            jsonValue(readSized([core](char *b, uint32_t n) { return dspi_output_labels(core, b, n); })).toArray();
        // -- WP SHOWFIX -- the names a person knows the channels by
        if (const QJsonObject words = QJsonDocument::fromJson(readSized([core](char *b, uint32_t n) {
                                          return dspi_channel_words(core, b, n);
                                      })).object();
            words.value(QStringLiteral("ok")).toBool())
            snap.channelWords = words;
        snap.wireFeatures = dspi_wire_features(core);
    }
    return snap;
}
