#include "SessionModel.h"

#include "DeviceRegistry.h"   // -- WP MDQT1 --
#include "DeviceSession.h"

#include <QDebug>
#include <QJsonDocument>   // -- WP QP16 --
#include <QJsonObject>
#include <QVector>
#include <mutex>
#include <vector>

namespace {
// What a worker job hands back after a call and the snapshot that follows it
struct Outcome {
    bool ok = false;
    uint flags = 0;
    DeviceSnapshot snap;
    QString refusal;   // -- WP MDQT1 -- why nothing was tried, "" otherwise
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

SessionModel::SessionModel(DeviceSession *session) : QObject(session), m_session(session) {
    connect(session, &DeviceSession::notificationsWaiting, this, &SessionModel::onNotifications);
    connect(session, &DeviceSession::deviceEvent, this, &SessionModel::onDeviceEvent);
}

bool SessionModel::readOnly() const {
    const int c = compat();
    return connected() && (c == COMPAT_FIRMWARE_TOO_OLD || c == COMPAT_FIRMWARE_TOO_NEW);
}

QString SessionModel::deviceTitle() const {
    if (!m_snap.deviceName.isEmpty()) return m_snap.deviceName;
    if (m_demo) return tr("Demo DSPi");
    if (m_serial.isEmpty()) return QString();
    return QStringLiteral("DSPi (%1)").arg(m_serial.right(8));
}

QString SessionModel::platformName() const {
    const DspState *s = state();
    if (!s) return QString();
    switch (s->platform_id) {
    case 1: return QStringLiteral("RP2350");
    case 2: return QStringLiteral("STM32H723");
    default: return QStringLiteral("RP2040");
    }
}

QString SessionModel::firmwareVersion() const {
    const DspState *s = state();
    if (!s || (s->fw_major == 0 && s->fw_minor == 0 && s->fw_patch == 0)) return QString();
    QString v = QStringLiteral("%1.%2.%3").arg(s->fw_major).arg(s->fw_minor).arg(s->fw_patch);
    if (s->fw_beta) v += QStringLiteral("-beta%1").arg(s->fw_beta);
    return v;
}

int SessionModel::wireFormat() const { return state() ? state()->format_version : 0; }
int SessionModel::compat() const { return state() ? state()->compat : COMPAT_UNKNOWN; }

// The device's inputs (the wire channels before the outputs), at least the
// stereo pair and at most the matrix's. The live count from the host's audio
// format replaces it where the screens need it (HostAudioFormat).
int SessionModel::numInputs() const {
    if (!state()) return 0;
    return qBound(2, int(state()->num_input_channels), MAX_INPUTS);
}

int SessionModel::numOutputs() const { return state() ? qMin(int(state()->num_output_channels), MAX_OUTPUTS) : 0; }

void SessionModel::track(int delta) {
    m_inFlight += delta;
    const bool busy = m_inFlight > 0;
    if (busy != m_busy) {
        m_busy = busy;
        emit busyChanged();
    }
}

void SessionModel::apply(const DeviceSnapshot &snap, uint flags) {
    m_snap = snap;
    emit applied(flags);
}

void SessionModel::start(bool demo) {
    if (demo) {
        openDemo();
    } else {
        track(1);
        m_session->worker()->call(
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
}

void SessionModel::openDemo() {
    if (m_snap.connected && !m_demo) emit replacing();   // -- WP MDQT2c --
    track(1);
    m_session->worker()->call(
        this,
        [](FfiCore *c) {
            const bool ok = dspi_open_demo(c) && dspi_fetch_all(c);
            return Outcome{ ok, 0, readSnapshot(c) };
        },
        [this](Outcome r) {
            if (!r.ok) qWarning("DeviceModel: the demo device did not open");
            m_demo = r.ok;
            m_document = false;   // -- WP P1QT1 --
            m_serial = r.ok ? QStringLiteral("demo") : QString();
            apply(r.snap, 0);
            track(-1);
        });
}

void SessionModel::selectDevice(const QString &serial) {
    if (serial.isEmpty()) return;
    if (m_snap.connected && serial != m_serial) emit replacing();   // -- WP MDQT2c --
    track(1);
    const QByteArray s = serial.toUtf8();
    m_session->worker()->call(
        this,
        [s](FfiCore *c) {
            // -- WP MDQT1 -- a refusal (another session has the unit) leaves what
            // is open as it is
            const uint64_t before = dspi_connection_generation(c);
            readSized([](char *b, uint32_t n) { return dspi_take_failure(b, n); }, 256);
            const bool ok = dspi_select_device(c, s.constData()) && dspi_fetch_all(c);
            Outcome o{ ok, 0, readSnapshot(c) };
            if (!ok && o.snap.connected && o.snap.generation == before)
                o.refusal = QJsonDocument::fromJson(readSized([](char *b, uint32_t n) { return dspi_take_failure(b, n); }, 256))
                                .object()
                                .value(QLatin1String("error"))
                                .toString(QStringLiteral("refused"));
            return o;
        },
        [this, serial](Outcome r) {
            if (!r.ok) qWarning("DeviceModel: %s did not open", qPrintable(serial));
            if (!r.refusal.isEmpty()) {
                apply(r.snap, 0);
                track(-1);
                qInfo("DeviceModel: %s", qPrintable(r.refusal));
                if (auto *registry = DeviceRegistry::instance()) registry->reportRefused(serial);
                return;
            }
            m_demo = false;
            m_document = false;   // -- WP P1QT1 --
            m_serial = serial;
            apply(r.snap, 0);
            track(-1);
        });
}

void SessionModel::reconnect() {
    if (m_remote) return;   // -- WP QP13 -- the hub's session reconnects by itself
    if (m_document) {       // -- WP P1QT1 -- a document has nothing to reopen
        refresh();
        return;
    }
    if (m_demo) {
        openDemo();
        return;
    }
    track(1);
    const QByteArray s = m_serial.toUtf8();
    m_session->worker()->call(
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

// -- WP SHOWFIX --
void SessionModel::reread() {
    m_session->worker()->call(
        this, [](FfiCore *c) { return readSnapshot(c); }, [this](const DeviceSnapshot &snap) { apply(snap, 0); });
}

void SessionModel::refresh() {
    track(1);
    m_session->worker()->call(
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
struct SessionModel::PendingWrite {
    std::mutex lock;                            // the setter and `started`: GUI and worker
    std::function<bool(FfiCore *)> setter;
    bool started = false;
    std::vector<std::function<void(bool)>> done; // GUI thread only
    quint64 drag = 0;                           // -- WP QTGAP1 -- the drag it was asked in (0: none)
};

void SessionModel::write(const QString &what, std::function<bool(FfiCore *)> setter,
                        std::function<void(bool)> done) {
    post(what, std::move(setter), std::move(done), false);   // -- WP QP16 --
}

// -- WP QP16 --
void SessionModel::writeEvenReadOnly(const QString &what, std::function<bool(FfiCore *)> setter,
                                    std::function<void(bool)> done) {
    post(what, std::move(setter), std::move(done), true);
}

void SessionModel::post(const QString &what, std::function<bool(FfiCore *)> setter, std::function<void(bool)> done,
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
    m_session->worker()->call(
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
            // -- WP P1QT2 --
            if (!r.ok && reason->contains(QStringLiteral("noticeKey")))
                emit writeRefused(what, reason->value(QStringLiteral("noticeKey")).toString(),
                                  reason->value(QStringLiteral("error")).toString());
            for (const std::function<void(bool)> &d : pending->done) d(r.ok);
            track(-1);
        });
}

// -- WP QTGAP1 --
void SessionModel::beginDrag() {
    m_drag = ++m_drags;
    m_session->worker()->post([](FfiCore *c) { dspi_set_dragging(c, true); });
}

void SessionModel::endDrag() {
    if (m_drag == 0) return;
    m_drag = 0;
    m_session->worker()->post([](FfiCore *c) { dspi_set_dragging(c, false); });
}

bool SessionModel::setChannelName(int channel, const QString &name) {
    const QByteArray utf8 = name.toUtf8();
    if (channel < 0 || channel >= MAX_CHANNELS || utf8.size() >= CHANNEL_NAME_LEN || !connected()) return false;
    const auto ch = uint8_t(channel);
    write(QStringLiteral("channel name %1").arg(channel), [ch, utf8](FfiCore *c) { return dspi_set_channel_name(c, ch, utf8.constData()); });
    return true;
}

void SessionModel::onNotifications() {
    track(1);
    DeviceSession *session = m_session;
    m_session->worker()->call(
        this,
        [session](FfiCore *c) {
            // Packets arriving from here on schedule another batch
            session->takeNotifications();
            NotifyResult r{};
            dspi_process_notifications(c, &r);
            return Outcome{ true, r.flags, r.flags ? readSnapshot(c) : DeviceSnapshot{} };
        },
        [this](Outcome r) {
            if (r.flags) apply(r.snap, r.flags);
            track(-1);
        });
}

void SessionModel::onDeviceEvent(int event, const QString &serial) {
    if (event == DeviceSession::Arrived) {
        if (m_remote) return;   // -- WP QP13 -- a hub's device stays open
        // Nothing open, nothing chosen: take the DSPi that arrived. The core
        // reopens a chosen one by itself; a fresh read follows.
        // -- WP MDQT1 -- on the main session, with nothing open beside it
        DeviceRegistry *registry = DeviceRegistry::instance();
        if (!connected() && !m_demo && m_serial.isEmpty() && m_session->isMain() && registry && registry->isEmpty())
            selectDevice(serial);
        else if (serial == m_serial) refresh();
    } else {
        track(1);
        m_session->worker()->call(
            this, [](FfiCore *c) { return readSnapshot(c); },
            [this](DeviceSnapshot snap) {
                apply(snap, 0);
                track(-1);
            });
    }
}

// -- WP QP13 -- Link sessions (LinkSession)
void SessionModel::adoptRemote(const QString &serial) {
    m_remote = true;
    m_demo = false;
    m_document = false;   // -- WP P1QT1 --
    m_serial = serial;
    refresh();
}

void SessionModel::leaveRemote() {
    if (!m_remote) return;
    m_remote = false;
    m_serial.clear();
    track(1);
    m_session->worker()->call(
        this, [](FfiCore *c) { return readSnapshot(c); },
        [this](DeviceSnapshot snap) {
            apply(snap, 0);
            track(-1);
        });
}

// -- WP P1QT1 -- an offline document (FilesController's Open File…)
void SessionModel::adoptDocument(const QString &serial) {
    m_remote = false;
    m_demo = false;
    m_document = true;
    m_serial = serial;
    refresh();
}

// -- WP MDQT1 --
void SessionModel::adoptOpened(const QString &kind, const QString &serial, const DeviceSnapshot &snap) {
    m_demo = kind == QLatin1String("demo");
    m_document = kind == QLatin1String("document");
    m_remote = kind == QLatin1String("hub");
    m_serial = m_demo ? QStringLiteral("demo") : m_document ? QStringLiteral("OFFLINE-DOCUMENT") : serial;
    apply(snap, 0);
    // What came in before the session's callbacks were set
    onNotifications();
}

void SessionModel::closed(std::function<void(FfiCore *)> close) {
    track(1);
    m_session->worker()->call(
        this,
        [close = std::move(close)](FfiCore *c) {
            close(c);
            return readSnapshot(c);
        },
        [this](DeviceSnapshot snap) {
            m_demo = false;
            m_document = false;
            m_remote = false;
            m_serial.clear();
            apply(snap, 0);
            track(-1);
        });
}
