#ifndef SNAPSHOT_H
#define SNAPSHOT_H

#include <QByteArray>
#include <QJsonArray>
#include <QString>
#include <cstdint>
#include <functional>
#include <memory>

#include "CoreApi.h"

// What the GUI knows of the device, read on the worker thread in one go and
// handed over whole. The worker is the only caller of dspi_get_state, so the
// header's rule (every reader of the record on the thread that calls it) holds
// without the GUI thread ever taking the core's lock.
struct DeviceSnapshot {
    bool connected = false;
    uint64_t generation = 0;               // dspi_connection_generation
    std::shared_ptr<const DspState> state; // a copy of the record; null when not connected
    QString deviceName;                    // dspi_cached_device_name, "" when unnamed
    QJsonArray outputLabels;               // dspi_output_labels' value array
    uint32_t wireFeatures = 0;             // dspi_wire_features
};

// Worker thread only.
DeviceSnapshot readSnapshot(FfiCore *core);

// For the dspi_* calls that fill a buffer and return the length they need
// (NUL included): calls `fill` until the buffer is large enough. "" on failure.
// Worker thread only, like the calls it wraps.
using SizedFill = std::function<uint32_t(char *buf, uint32_t capacity)>;
QByteArray readSized(const SizedFill &fill, uint32_t first = 1024);

// A {ok, value} reply's value; an empty value when ok is false or the text is
// not JSON.
QJsonValue jsonValue(const QByteArray &reply);

// A NUL-padded UTF-8 field of the state record.
QString fieldText(const uint8_t *bytes, int size);

#endif // SNAPSHOT_H
