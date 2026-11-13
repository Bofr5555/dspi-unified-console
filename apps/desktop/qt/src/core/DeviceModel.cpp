#include "DeviceModel.h"

#include "ChannelListModel.h"
#include "Core.h"
#include "PresetModel.h"

#include <QDebug>
#include <QJsonDocument>   // -- WP QP16 --
#include <QJsonObject>
#include <QJSEngine>
#include <QVector>
#include <mutex>
#include <vector>

namespace {
DeviceModel *s_instance = nullptr;

// What a worker job hands back after a call and the snapshot that follows it
struct Outcome {
    bool ok = false;
    uint flags = 0;
    DeviceSnapshot snap;
};

// The first DSPi a scan finds, "" for none. Worker thread.
QString firstSerial(FfiCore *c) {
    QVector<DeviceInfo> found(16);
    const uint32_t n = dspi_scan_devices(c, found.data(), uint32_t(found.size()));
    if (n == 0) return QString();
    const DeviceInfo &d = found[0];
    const uint32_t len = qMin<uint32_t>(d.serial_len, uint32_t(sizeof d.serial));
    return QString::fromUtf8(reinterpret_cast<const char *>(d.serial), int(len));
}
} // namespace

DeviceModel::DeviceModel(Core *core, QObject *parent)
    : QObject(parent), m_core(core), m_channels(new ChannelListModel(this)), m_presets(new PresetModel(this)) {
    s_instance = this;
    connect(core, &Core::notificationsWaiting, this, &DeviceModel::onNotifications);
    connect(core, &Core::deviceEvent, this, &DeviceModel::onDeviceEvent);
}

DeviceModel::~DeviceModel() {
    if (s_instance == this) s_instance = nullptr;
}

DeviceModel *DeviceModel::instance() { return s_instance; }

DeviceModel *DeviceModel::create(QQmlEngine *, QJSEngine *) {
    QJSEngine::setObjectOwnership(s_instance, QJSEngine::CppOwnership);
    return s_instance;
}

bool DeviceModel::readOnly() const {
    const int c = compat();
    return connected() && (c == COMPAT_FIRMWARE_TOO_OLD || c == COMPAT_FIRMWARE_TOO_NEW);
}

QString DeviceModel::deviceTitle() const {
    if (!m_snap.deviceName.isEmpty()) return m_snap.deviceName;
    if (m_demo) return tr("Demo DSPi");
    if (m_serial.isEmpty()) return QString();
    return QStringLiteral("DSPi (%1)").arg(m_serial.right(8));
}

QString DeviceModel::platformName() const {
    const DspState *s = state();
    if (!s) return QString();
    switch (s->platform_id) {
    case 1: return QStringLiteral("RP2350");
    case 2: return QStringLiteral("STM32H723");
    default: return QStringLiteral("RP2040");
    }
}

QString DeviceModel::firmwareVersion() const {
    const DspState *s = state();
    if (!s || (s->fw_major == 0 && s->fw_minor == 0 && s->fw_patch == 0)) return QString();
    QString v = QStringLiteral("%1.%2.%3").arg(s->fw_major).arg(s->fw_minor).arg(s->fw_patch);
    if (s->fw_beta) v += QStringLiteral("-beta%1").arg(s->fw_beta);
    return v;
}

int DeviceModel::wireFormat() const { return state() ? state()->format_version : 0; }
int DeviceModel::compat() const { return state() ? state()->compat : COMPAT_UNKNOWN; }

// The device's inputs (the wire channels before the outputs), at least the
// stereo pair and at most the matrix's. The live count from the host's audio
// format replaces it where the screens need it (HostAudioFormat).
int DeviceModel::numInputs() const {
    if (!state()) return 0;
    return qBound(2, int(state()->num_input_channels), MAX_INPUTS);
}

int DeviceModel::numOutputs() const { return state() ? qMin(int(state()->num_output_channels), MAX_OUTPUTS) : 0; }

void DeviceModel::track(int delta) {
    m_inFlight += delta;
    const bool busy = m_inFlight > 0;
    if (busy != m_busy) {
        m_busy = busy;
        emit busyChanged();
    }
}

void DeviceModel::apply(const DeviceSnapshot &snap, uint flags) {
    m_snap = snap;
    ++m_revision;
    m_channels->update(snap, platformName());
    m_presets->update(snap);
    emit snapshotApplied();
    emit stateChanged(flags);
}

void DeviceModel::start(bool demo) {
    if (demo) {
        openDemo();
    } else {
        track(1);
        m_core->worker()->call(
            this,
            [](FfiCore *c) {
                const QString serial = firstSerial(c);
                const bool ok = !serial.isEmpty() && dspi_select_device(c, serial.toUtf8().constData()) &&
                                dspi_fetch_all(c);
                return std::make_pair(serial, Outcome{ ok, 0, readSnapshot(c) });
            },
            [this](std::pair<QString, Outcome> r) {
                m_serial = r.first;
                apply(r.second.snap, 0);
                track(-1);
            });
    }
    m_core->startHotplug();
}

void DeviceModel::openDemo() {
    track(1);
    m_core->worker()->call(
        this,
        [](FfiCore *c) {
            const bool ok = dspi_open_demo(c) && dspi_fetch_all(c);
            return Outcome{ ok, 0, readSnapshot(c) };
        },
        [this](Outcome r) {
            if (!r.ok) qWarning("DeviceModel: the demo device did not open");
            m_demo = r.ok;
            m_serial = r.ok ? QStringLiteral("demo") : QString();
            apply(r.snap, 0);
            track(-1);
        });
}

void DeviceModel::selectDevice(const QString &serial) {
    if (serial.isEmpty()) return;
    track(1);
    const QByteArray s = serial.toUtf8();
    m_core->worker()->call(
        this,
        [s](FfiCore *c) {
            const bool ok = dspi_select_device(c, s.constData()) && dspi_fetch_all(c);
            return Outcome{ ok, 0, readSnapshot(c) };
        },
        [this, serial](Outcome r) {
            if (!r.ok) qWarning("DeviceModel: %s did not open", qPrintable(serial));
            m_demo = false;
            m_serial = serial;
            apply(r.snap, 0);
            track(-1);
        });
}

void DeviceModel::reconnect() {
    if (m_remote) return;   // -- WP QP13 -- the hub's session reconnects by itself
    if (m_demo) {
        openDemo();
        return;
    }
    track(1);
    const QByteArray s = m_serial.toUtf8();
    m_core->worker()->call(
        this,
        [s](FfiCore *c) {
            dspi_disconnect(c);
            const bool ok = !s.isEmpty() && dspi_select_device(c, s.constData()) && dspi_fetch_all(c);
            return Outcome{ ok, 0, readSnapshot(c) };
        },
        [this](Outcome r) {
            apply(r.snap, 0);
            track(-1);
        });
}

void DeviceModel::refresh() {
    track(1);
    m_core->worker()->call(
        this,
        [](FfiCore *c) {
            const bool ok = dspi_fetch_all(c);
            return Outcome{ ok, 0, readSnapshot(c) };
        },
        [this](Outcome r) {
            apply(r.snap, 0);
            track(-1);
        });
}

// A write posted and not started yet. A later write with the same `what`
// replaces its setter (latest wins) and adds its `done`, so a slider dragged
// over a slow link sends the newest value, not a queue of old ones.
struct DeviceModel::PendingWrite {
    std::mutex lock;                            // the setter and `started`: GUI and worker
    std::function<bool(FfiCore *)> setter;
    bool started = false;
    std::vector<std::function<void(bool)>> done; // GUI thread only
    quint64 drag = 0;                           // -- WP QTGAP1 -- the drag it was asked in (0: none)
};

void DeviceModel::write(const QString &what, std::function<bool(FfiCore *)> setter,
                        std::function<void(bool)> done) {
    post(what, std::move(setter), std::move(done), false);   // -- WP QP16 --
}

// -- WP QP16 --
void DeviceModel::writeEvenReadOnly(const QString &what, std::function<bool(FfiCore *)> setter,
                                    std::function<void(bool)> done) {
    post(what, std::move(setter), std::move(done), true);
}

void DeviceModel::post(const QString &what, std::function<bool(FfiCore *)> setter, std::function<void(bool)> done,
                       bool evenReadOnly) {
    if (!connected() || (readOnly() && !evenReadOnly)) {   // -- WP QP16 -- evenReadOnly
        qInfo("DeviceModel: %s not sent: %s", qPrintable(what), connected() ? "read-only" : "no device");
        emit writeFailed(what);
        if (done) done(false);
        return;
    }
    if (const std::shared_ptr<PendingWrite> queued = m_queuedWrites.value(what)) {
        std::lock_guard<std::mutex> hold(queued->lock);
        // -- WP QTGAP1 -- only within one drag, or outside any: a typed value
        // must not take the place of a drag's write queued before the drag's end
        if (!queued->started && queued->drag == m_drag) {
            queued->setter = std::move(setter);
            if (done) queued->done.push_back(std::move(done));
            return;
        }
    }
    auto pending = std::make_shared<PendingWrite>();
    pending->setter = std::move(setter);
    pending->drag = m_drag;   // -- WP QTGAP1 --
    if (done) pending->done.push_back(std::move(done));
    m_queuedWrites.insert(what, pending);
    track(1);
    std::atomic<int> *settersRun = &m_settersRun;
    // -- WP QP16 -- The device this write was asked for: a write the worker
    // reaches after another device opened is dropped, not sent (QD6), and a
    // refusal brings the core's reason along
    const uint64_t generation = m_snap.generation;
    auto reason = std::make_shared<QJsonObject>();
    m_core->worker()->call(
        this,
        [pending, settersRun, generation, reason](FfiCore *c) {
            std::function<bool(FfiCore *)> run;
            {
                std::lock_guard<std::mutex> hold(pending->lock);
                pending->started = true;
                run = std::move(pending->setter);
            }
            // -- WP QP16 --
            if (generation != 0 && dspi_connection_generation(c) != generation) {
                reason->insert(QStringLiteral("code"), QStringLiteral("dropped"));
                return Outcome{ false, 0, readSnapshot(c) };
            }
            readSized([](char *b, uint32_t n) { return dspi_take_failure(b, n); }, 256);
            settersRun->fetch_add(1);
            const bool ok = run(c);
            if (!ok) *reason = QJsonDocument::fromJson(readSized([](char *b, uint32_t n) { return dspi_take_failure(b, n); }, 256)).object();
            return Outcome{ ok, 0, readSnapshot(c) };
        },
        [this, what, pending, reason](Outcome r) {
            if (m_queuedWrites.value(what) == pending) m_queuedWrites.remove(what);
            apply(r.snap, 0);
            if (!r.ok) {
                qInfo("DeviceModel: %s refused", qPrintable(what));
                emit writeFailed(what);
            }
            // -- WP QP16 --
            emit writeFinished(what, r.ok, r.ok ? QString() : reason->value(QStringLiteral("code")).toString(),
                               r.ok ? QString() : reason->value(QStringLiteral("error")).toString());
            for (const std::function<void(bool)> &d : pending->done) d(r.ok);
            track(-1);
        });
}

// -- WP QTGAP1 --
void DeviceModel::beginDrag() {
    m_drag = ++m_drags;
    m_core->worker()->post([](FfiCore *c) { dspi_set_dragging(c, true); });
}

void DeviceModel::endDrag() {
    if (m_drag == 0) return;
    m_drag = 0;
    m_core->worker()->post([](FfiCore *c) { dspi_set_dragging(c, false); });
}

bool DeviceModel::setChannelName(int channel, const QString &name) {
    const QByteArray utf8 = name.toUtf8();
    if (channel < 0 || channel >= MAX_CHANNELS || utf8.size() >= CHANNEL_NAME_LEN || !connected()) return false;
    const auto ch = uint8_t(channel);
    write(QStringLiteral("channel name %1").arg(channel), [ch, utf8](FfiCore *c) { return dspi_set_channel_name(c, ch, utf8.constData()); });
    return true;
}

void DeviceModel::onNotifications() {
    track(1);
    Core *core = m_core;
    m_core->worker()->call(
        this,
        [core](FfiCore *c) {
            // Packets arriving from here on schedule another batch
            core->m_notifyQueued.store(false);
            NotifyResult r{};
            dspi_process_notifications(c, &r);
            return Outcome{ true, r.flags, r.flags ? readSnapshot(c) : DeviceSnapshot{} };
        },
        [this](Outcome r) {
            if (r.flags) apply(r.snap, r.flags);
            track(-1);
        });
}

void DeviceModel::onDeviceEvent(int event, const QString &serial) {
    if (event == Core::Arrived) {
        if (m_remote) return;   // -- WP QP13 -- a hub's device stays open
        // Nothing open, nothing chosen: take the DSPi that arrived. The core
        // reopens a chosen one by itself; a fresh read follows.
        if (!connected() && !m_demo && m_serial.isEmpty()) selectDevice(serial);
        else if (serial == m_serial) refresh();
    } else {
        track(1);
        m_core->worker()->call(
            this, [](FfiCore *c) { return readSnapshot(c); },
            [this](DeviceSnapshot snap) {
                apply(snap, 0);
                track(-1);
            });
    }
}

// -- WP QP13 -- Link sessions (LinkController)
void DeviceModel::adoptRemote(const QString &serial) {
    m_remote = true;
    m_demo = false;
    m_serial = serial;
    refresh();
}

void DeviceModel::leaveRemote() {
    if (!m_remote) return;
    m_remote = false;
    m_serial.clear();
    track(1);
    m_core->worker()->call(
        this, [](FfiCore *c) { return readSnapshot(c); },
        [this](DeviceSnapshot snap) {
            apply(snap, 0);
            track(-1);
        });
}
