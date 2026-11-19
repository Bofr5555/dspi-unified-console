#include "SystemController.h"

#include "Core.h"
#include "DeviceModel.h"
#include "Registrar.h"

#include <QCoreApplication>
#include <QJSEngine>
#include <QJsonArray>
#include <QJsonDocument>
#include <QRegularExpression>

extern "C" {
#include "dspi_core.h"
}

static const Registrar::Controller<SystemController> systemController;
static const Registrar::Banner systemStrip("SystemStatusStrip", 30);
static const Registrar::Window systemWindow("window.system", "SystemWindow");

namespace {

// The view of a big installation fits; a reply that does not is read again
// (the core then gives its last view, as the hub limits the status reads)
constexpr uint32_t kBuffer = 256 * 1024;
constexpr int kPollMs = 3000;

// A read: a reply too long for the buffer is asked for once more, with room for it
QJsonObject call(const std::function<uint32_t(char *, uint32_t)> &fill) {
    QByteArray buf(int(kBuffer), '\0');
    uint32_t need = fill(buf.data(), kBuffer);
    if (need > kBuffer) {
        buf.resize(int(need));
        need = fill(buf.data(), need);
    }
    if (need == 0 || need > uint32_t(buf.size())) return QJsonObject();
    buf.truncate(int(need) - 1);
    return QJsonDocument::fromJson(buf).object();
}

// -- fixup lk1 -- A call that acts on the hub (save, align, recall): called once, never
// again. A reply too long for the buffer means it has happened (fns_link.rs, "Short
// buffers"), so it is said and the view read again, not repeated
QJsonObject act(const std::function<uint32_t(char *, uint32_t)> &fill) {
    QByteArray buf(int(kBuffer), '\0');
    const uint32_t need = fill(buf.data(), kBuffer);
    if (need > kBuffer) {
        return QJsonObject{ { QStringLiteral("ok"), false },
                            { QStringLiteral("code"), QStringLiteral("reply_too_long") },
                            { QStringLiteral("error"),
                              SystemController::tr("The hub did it, but its answer was too long to show. The view is read again.") } };
    }
    if (need == 0) return QJsonObject();
    buf.truncate(int(need) - 1);
    return QJsonDocument::fromJson(buf).object();
}

bool isOk(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }
QString str(const QJsonObject &o, const char *key) { return o.value(QLatin1String(key)).toString(); }

// A whole number of µs as typed, "" for none; nullopt-like: ok false
QVariant parseUs(const QString &text, bool *ok) {
    const QString t = text.trimmed();
    *ok = true;
    if (t.isEmpty()) return QVariant();
    static const QRegularExpression whole(QStringLiteral("^-?\\d{1,7}$"));
    if (!whole.match(t).hasMatch()) {
        *ok = false;
        return QVariant();
    }
    return QVariant(t.toLongLong());
}

} // namespace

SystemController::SystemController(const AppContext &ctx, QObject *parent)
    : QObject(parent), m_core(ctx.core->handle()), m_device(ctx.device) {
    m_pool.setMaxThreadCount(1);
    m_pool.setExpiryTimeout(-1);
    m_timer.setInterval(kPollMs);
    connect(&m_timer, &QTimer::timeout, this, &SystemController::refresh);
    connect(m_device, &DeviceModel::snapshotApplied, this, &SystemController::updatePolling);
    // No wait here: a hub request may take its 15 s, and LinkController's quit
    // closes the session, which ends it; the destructor waits afterwards
    connect(qApp, &QCoreApplication::aboutToQuit, this, [this] {
        m_shutDown = true;
        m_timer.stop();
    });
    updatePolling();
}

SystemController::~SystemController() {
    m_shutDown = true;
    m_pool.waitForDone();
}

SystemController *SystemController::create(QQmlEngine *, QJSEngine *) {
    SystemController *self = Registrar::instance<SystemController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

// -- fixup lk1 --
bool SystemController::writable() const { return !m_readOnlyForTest && !m_device->readOnly(); }

QString SystemController::editNote() const {
    if (!writable()) return tr("This DSPi is read only on this console (its firmware is not this version's).");
    return text("editNote");
}

QString SystemController::recallNote() const {
    if (!writable()) return tr("This DSPi is read only on this console (its firmware is not this version's).");
    return text("recallNote");
}

QVariantList SystemController::list(const char *key) const {
    return m_view.value(QLatin1String(key)).toArray().toVariantList();
}

QStringList SystemController::issues() const {
    QStringList out;
    for (const QJsonValue &v : m_view.value(QLatin1String("issues")).toArray()) out << v.toString();
    return out;
}

// Polls only while a hub's device is open: a device the console holds itself
// has no installation to read
void SystemController::updatePolling() {
    // -- fixup lk1 -- the read-only state moves with the device's snapshot
    if (writable() != m_wasWritable) {
        m_wasWritable = writable();
        emit viewChanged();
    }
    const bool onHub = m_device->connected() && m_device->remote();
    if (onHub && !m_timer.isActive()) {
        m_timer.start();
        refresh();
    } else if (!onHub && m_timer.isActive()) {
        m_timer.stop();
        if (m_available) {
            m_available = false;
            m_view = QJsonObject();
            m_error.clear();
            setLists(QJsonObject());
            emit viewChanged();
        }
        // What this session did is not the next one's
        m_lastRecall.clear();
        m_recalledScene.clear();
        emit recallChanged();
        setAction(QString(), false);
        clearDraft();
    }
}

void SystemController::run(std::function<QJsonObject(FfiCore *)> job, std::function<void(const QJsonObject &)> done) {
    if (m_shutDown) return;
    ++m_busy;
    emit busyChanged();
    FfiCore *core = m_core;
    m_pool.start([this, core, job = std::move(job), done = std::move(done)] {
        const QJsonObject o = job(core);
        QMetaObject::invokeMethod(
            this,
            [this, o, done] {
                --m_busy;
                emit busyChanged();
                done(o);
            },
            Qt::QueuedConnection);
    });
}

void SystemController::refresh() {
    if (m_shutDown || m_reading.exchange(true)) return;
    FfiCore *core = m_core;
    m_pool.start([this, core] {
        const QJsonObject o = call([core](char *b, uint32_t n) { return dspi_alink_system_view(core, b, n); });
        QMetaObject::invokeMethod(
            this,
            [this, o] {
                m_reading = false;
                applyView(o);
            },
            Qt::QueuedConnection);
    });
}

void SystemController::applyView(const QJsonObject &o) {
    if (!isOk(o)) {
        // Not on a hub now (disconnected), or the hub did not answer: the last
        // view stays on show while a hub's device is open
        const QString code = str(o, "code");
        m_error = o.isEmpty() ? tr("The hub did not answer.") : str(o, "error");
        if (code == QLatin1String("disconnected")) {
            m_available = false;
            m_view = QJsonObject();
            setLists(QJsonObject());
        }
        emit viewChanged();
        return;
    }
    m_available = true;
    m_error.clear();
    m_view = o;
    m_view.remove(QStringLiteral("ok"));
    setLists(m_view);
    if (m_dirty && revision() != m_draftRevision && !m_stale) {
        m_stale = true;
        emit draftChanged();
    }
    if (!m_dirty) {
        m_draftAuto = systemDelayAuto();
        m_draftFrames = systemDelayFrames();
        emit draftChanged();
    }
    emit viewChanged();
}

void SystemController::setLists(const QJsonObject &o) {
    const auto update = [&o](const char *key, QVariantList &into, void (SystemController::*signal)(), SystemController *self) {
        const QVariantList now = o.value(QLatin1String(key)).toArray().toVariantList();
        if (now == into) return;
        into = now;
        emit(self->*signal)();
    };
    update("units", m_units, &SystemController::unitsChanged, this);
    update("rooms", m_rooms, &SystemController::roomsChanged, this);
    update("scenes", m_scenes, &SystemController::scenesChanged, this);
    QVariantList edit;
    for (const QVariant &v : m_units) {
        const QVariantMap u = v.toMap();
        if (u.value(QStringLiteral("kind")).toString() != QLatin1String("unit")) continue;
        edit.append(QVariantMap{ { QStringLiteral("id"), u.value(QStringLiteral("id")) },
                                 { QStringLiteral("name"), u.value(QStringLiteral("name")) },
                                 { QStringLiteral("latencyUs"), u.value(QStringLiteral("latencyUs")) } });
    }
    // -- WP AUSB2 -- the USB-direct units, for their Installer settings
    QVariantList usb;
    for (const QVariant &v : m_units) {
        const QVariantMap u = v.toMap();
        if (u.value(QStringLiteral("transport")).toString() != QLatin1String("usb_direct")) continue;
        usb.append(QVariantMap{ { QStringLiteral("id"), u.value(QStringLiteral("id")) },
                                { QStringLiteral("name"), u.value(QStringLiteral("name")) } });
    }
    if (edit != m_editUnits || usb != m_usbUnits) {
        m_editUnits = edit;
        m_usbUnits = usb;
        emit editUnitsChanged();
        emit spdifChanged();   // -- WP ATQT -- the candidates follow the units
    }
    // -- WP ATQT -- the S/PDIF units of the same reading
    const QVariantList spdif = o.value(QLatin1String("spdifUnits")).toArray().toVariantList();
    if (spdif != m_spdifUnits) {
        m_spdifUnits = spdif;
        emit spdifChanged();
    }
}

void SystemController::recall(const QString &id) {
    if (!mayRecall() || busy()) return;
    const QByteArray scene = id.toUtf8();
    QString name = id;
    for (const QVariant &s : scenes()) {
        const QVariantMap m = s.toMap();
        if (m.value(QStringLiteral("id")).toString() == id) name = m.value(QStringLiteral("name")).toString();
    }
    run([scene](FfiCore *c) { return act([&](char *b, uint32_t n) { return dspi_alink_system_recall(c, scene.constData(), b, n); }); },
        [this, name](const QJsonObject &o) {
            QVariantMap r;
            if (isOk(o)) {
                r = o.toVariantMap();
                r.remove(QStringLiteral("ok"));
                if (o.value(QLatin1String("taken")).toBool()) m_recalledScene = str(o, "name");
            } else {
                // Refused by role, with the hub's words naming the op
                r.insert(QStringLiteral("name"), name);
                r.insert(QStringLiteral("taken"), false);
                r.insert(QStringLiteral("message"),
                         o.isEmpty() ? tr("%1: the hub did not answer.").arg(name)
                                     : tr("%1 was refused: %2").arg(name, str(o, "error")));
            }
            m_lastRecall = r;
            emit recallChanged();
            refresh();
        });
}

void SystemController::align() {
    if (!mayEdit() || busy()) return;
    run([](FfiCore *c) { return act([&](char *b, uint32_t n) { return dspi_alink_system_align(c, b, n); }); },
        [this](const QJsonObject &o) {
            if (isOk(o)) setAction(str(o, "message"), !o.value(QLatin1String("applied")).toBool());
            else setAction(o.isEmpty() ? tr("The hub did not answer.") : str(o, "error"), true);
            refresh();
        });
}

void SystemController::setAction(const QString &message, bool failed) {
    m_actionMessage = message;
    m_actionFailed = failed;
    emit actionChanged();
}

// ---- The draft ----------------------------------------------------------------

void SystemController::setSystemDelay(bool automatic, int frames) {
    if (!mayEdit()) return;
    if (!m_dirty) m_draftRevision = revision();
    m_draftDelaySet = true;
    m_draftAuto = automatic;
    m_draftFrames = qBound(0, frames, 65535);
    m_dirty = true;
    emit draftChanged();
}

bool SystemController::setRoomLipSync(const QString &roomId, const QString &us) {
    bool ok = false;
    const QVariant v = parseUs(us, &ok);
    if (!ok || !mayEdit()) return false;
    if (!m_dirty) m_draftRevision = revision();
    m_draftRooms.insert(roomId, v);
    m_dirty = true;
    emit draftChanged();
    return true;
}

bool SystemController::setUnitLatency(const QString &unitId, const QString &us) {
    bool ok = false;
    const QVariant v = parseUs(us, &ok);
    if (!ok || !mayEdit()) return false;
    if (!m_dirty) m_draftRevision = revision();
    m_draftUnits.insert(unitId, v);
    m_dirty = true;
    emit draftChanged();
    return true;
}

// -- WP AUSB2 --
static QVariantMap unitById(const QVariantList &units, const QString &id) {
    for (const QVariant &u : units) {
        const QVariantMap m = u.toMap();
        if (m.value(QStringLiteral("id")).toString() == id) return m;
    }
    return QVariantMap();
}

QString SystemController::usbModeOf(const QString &unitId) const {
    if (m_draftUsb.contains(unitId)) return m_draftUsb.value(unitId).value(QStringLiteral("mode")).toString();
    return unitById(units(), unitId).value(QStringLiteral("usbModeWanted")).toString();
}

QString SystemController::onLockErrorOf(const QString &unitId) const {
    const QString drafted = m_draftUsb.value(unitId).value(QStringLiteral("onLockError")).toString();
    if (!drafted.isEmpty()) return drafted;
    const QString wanted = unitById(units(), unitId).value(QStringLiteral("usbOnLockErrorWanted")).toString();
    return wanted.isEmpty() ? QStringLiteral("mute") : wanted;
}

bool SystemController::keepUsbAligned() const {
    return m_draftKeep.isValid() ? m_draftKeep.toBool() : m_view.value(QLatin1String("keepUsbAligned")).toBool();
}

bool SystemController::setUsbMode(const QString &unitId, const QString &mode) {
    static const QStringList modes{ QString(), QStringLiteral("off"), QStringLiteral("rate_lock"), QStringLiteral("phase_lock") };
    if (!mayEdit() || !modes.contains(mode)) return false;
    if (!m_dirty) m_draftRevision = revision();
    QVariantMap d = m_draftUsb.value(unitId);
    d.insert(QStringLiteral("mode"), mode);
    m_draftUsb.insert(unitId, d);
    m_dirty = true;
    emit draftChanged();
    return true;
}

bool SystemController::setOnLockError(const QString &unitId, const QString &action) {
    if (!mayEdit() || (action != QLatin1String("mute") && action != QLatin1String("play_unaligned"))) return false;
    if (!m_dirty) m_draftRevision = revision();
    QVariantMap d = m_draftUsb.value(unitId);
    // The mode goes with it: the core's edit sets both (none: the unit's own)
    if (!d.contains(QStringLiteral("mode"))) d.insert(QStringLiteral("mode"), usbModeOf(unitId));
    d.insert(QStringLiteral("onLockError"), action);
    m_draftUsb.insert(unitId, d);
    m_dirty = true;
    emit draftChanged();
    return true;
}

bool SystemController::setKeepUsbAligned(bool on) {
    if (!mayEdit()) return false;
    if (!m_dirty) m_draftRevision = revision();
    m_draftKeep = on;
    m_dirty = true;
    emit draftChanged();
    return true;
}

QString SystemController::roomLipSyncText(const QString &roomId) const {
    if (m_draftRooms.contains(roomId)) {
        const QVariant v = m_draftRooms.value(roomId);
        return v.isValid() ? QString::number(v.toLongLong()) : QString();
    }
    for (const QVariant &r : rooms()) {
        const QVariantMap m = r.toMap();
        if (m.value(QStringLiteral("id")).toString() != roomId) continue;
        const QVariant us = m.value(QStringLiteral("lipSyncUs"));
        return us.isNull() ? QString() : QString::number(us.toLongLong());
    }
    return QString();
}

QString SystemController::unitLatencyText(const QString &unitId) const {
    if (m_draftUnits.contains(unitId)) {
        const QVariant v = m_draftUnits.value(unitId);
        return v.isValid() ? QString::number(v.toLongLong()) : QString();
    }
    for (const QVariant &u : units()) {
        const QVariantMap m = u.toMap();
        if (m.value(QStringLiteral("id")).toString() != unitId) continue;
        const QVariant us = m.value(QStringLiteral("measuredLatencyUs"));
        return us.isNull() ? QString() : QString::number(us.toLongLong());
    }
    return QString();
}

QByteArray SystemController::draftJson() const {
    QJsonObject edits;
    edits.insert(QStringLiteral("setSystemDelay"), m_draftDelaySet);
    edits.insert(QStringLiteral("systemDelayFrames"), m_draftAuto ? QJsonValue() : QJsonValue(m_draftFrames));
    QJsonArray rooms, units;
    for (auto it = m_draftRooms.cbegin(); it != m_draftRooms.cend(); ++it)
        rooms.append(QJsonObject{ { QStringLiteral("roomId"), it.key() },
                                  { QStringLiteral("lipSyncUs"), it.value().isValid() ? QJsonValue(it.value().toLongLong()) : QJsonValue() } });
    for (auto it = m_draftUnits.cbegin(); it != m_draftUnits.cend(); ++it)
        units.append(QJsonObject{ { QStringLiteral("unitId"), it.key() },
                                  { QStringLiteral("measuredLatencyUs"), it.value().isValid() ? QJsonValue(it.value().toLongLong()) : QJsonValue() } });
    edits.insert(QStringLiteral("rooms"), rooms);
    edits.insert(QStringLiteral("units"), units);
    // -- WP AUSB2 --
    QJsonArray usb;
    for (auto it = m_draftUsb.cbegin(); it != m_draftUsb.cend(); ++it) {
        const QString mode = it.value().value(QStringLiteral("mode")).toString();
        const QString action = it.value().value(QStringLiteral("onLockError")).toString();
        usb.append(QJsonObject{ { QStringLiteral("unitId"), it.key() },
                                { QStringLiteral("mode"), mode.isEmpty() ? QJsonValue() : QJsonValue(mode) },
                                { QStringLiteral("onLockError"), action.isEmpty() ? QJsonValue() : QJsonValue(action) } });
    }
    edits.insert(QStringLiteral("usbModes"), usb);
    if (m_draftKeep.isValid()) edits.insert(QStringLiteral("keepUsbAligned"), m_draftKeep.toBool());
    // -- WP ATQT -- the S/PDIF edits, in the same system.set
    QJsonArray spdif;
    for (auto it = m_draftSpdif.cbegin(); it != m_draftSpdif.cend(); ++it) {
        QJsonObject e = QJsonObject::fromVariantMap(it.value());
        e.insert(QStringLiteral("unitId"), it.key());
        spdif.append(e);
    }
    if (!spdif.isEmpty()) edits.insert(QStringLiteral("spdif"), spdif);
    return QJsonDocument(edits).toJson(QJsonDocument::Compact);
}

void SystemController::save() {
    if (!m_dirty || m_stale || !mayEdit() || busy()) return;
    const QByteArray edits = draftJson();
    const quint64 rev = quint64(m_draftRevision);
    run([edits, rev](FfiCore *c) {
            return act([&](char *b, uint32_t n) { return dspi_alink_system_save(c, rev, edits.constData(), b, n); });
        },
        [this](const QJsonObject &o) {
            if (isOk(o)) {
                clearDraft();
                // -- WP P1QT1 -- the hub's warnings about the model it kept
                // (HWLK1F's note: a unit whose firmware has no audio link)
                QStringList warnings;
                for (const QJsonValue &w : o.value(QLatin1String("warnings")).toArray()) warnings.append(w.toString());
                if (warnings.isEmpty()) setAction(tr("Saved on the hub."), false);
                else
                    setAction(tr("Saved on the hub, with warnings:") + QLatin1Char('\n') + warnings.join(QLatin1Char('\n')), false);
            } else if (str(o, "code") == QLatin1String("conflict")) {
                m_stale = true;
                emit draftChanged();
                setAction(str(o, "error"), true);
            } else {
                setAction(o.isEmpty() ? tr("The hub did not answer.") : str(o, "error"), true);
            }
            refresh();
        });
}

void SystemController::clearDraft() {
    m_dirty = false;
    m_stale = false;
    m_draftDelaySet = false;
    m_draftRooms.clear();
    m_draftUnits.clear();
    m_draftUsb.clear();   // -- WP AUSB2 --
    m_draftSpdif.clear();   // -- WP ATQT --
    m_draftKeep = QVariant();
    m_draftAuto = systemDelayAuto();
    m_draftFrames = systemDelayFrames();
    emit draftChanged();
}

void SystemController::reload() {
    clearDraft();
    setAction(QString(), false);
    refresh();
}

// ---- -- WP ATQT -- The S/PDIF transport ---------------------------------------------

QVariantMap SystemController::spdifRow(const QString &unitId) const { return unitById(m_spdifUnits, unitId); }

QVariantList SystemController::spdifSources() const {
    QVariantList out;
    for (const QVariant &v : m_spdifUnits) {
        const QVariantMap u = v.toMap();
        if (u.value(QStringLiteral("role")).toString() != QLatin1String("source")) continue;
        out.append(QVariantMap{ { QStringLiteral("id"), u.value(QStringLiteral("id")) }, { QStringLiteral("name"), u.value(QStringLiteral("name")) } });
    }
    return out;
}

QVariantList SystemController::spdifCandidates() const {
    QVariantList out;
    for (const QVariant &v : m_units) {
        const QVariantMap u = v.toMap();
        const QString id = u.value(QStringLiteral("id")).toString();
        if (u.value(QStringLiteral("kind")).toString() != QLatin1String("unit")) continue;
        if (u.value(QStringLiteral("transport")).toString() == QLatin1String("usb_direct")) continue;
        if (!spdifRow(id).isEmpty()) continue;
        out.append(QVariantMap{ { QStringLiteral("id"), id }, { QStringLiteral("name"), u.value(QStringLiteral("name")) } });
    }
    return out;
}

bool SystemController::spdifFed(const QString &unitId) const {
    return spdifRow(unitId).value(QStringLiteral("role")).toString() == QLatin1String("follower");
}

bool SystemController::spdifEdit(const QString &unitId, const QVariantMap &change) {
    if (!mayEdit() || unitId.isEmpty()) return false;
    if (!m_dirty) m_draftRevision = revision();
    QVariantMap e = m_draftSpdif.value(unitId);
    // A new role drops what the old one drafted
    if (change.contains(QStringLiteral("role"))) e.clear();
    for (auto it = change.cbegin(); it != change.cend(); ++it) e.insert(it.key(), it.value());
    m_draftSpdif.insert(unitId, e);
    m_dirty = true;
    emit draftChanged();
    return true;
}

bool SystemController::setSpdifSource(const QString &unitId, const QString &sourceId) {
    if (!spdifFed(unitId) || unitById(spdifSources(), sourceId).isEmpty()) return false;
    return spdifEdit(unitId, { { QStringLiteral("role"), QStringLiteral("follower") }, { QStringLiteral("source"), sourceId } });
}

bool SystemController::setSpdifSlot(const QString &unitId, int slot, bool on) {
    if (slot < 0 || slot > 3) return false;
    const bool source = spdifRow(unitId).value(QStringLiteral("role")).toString() == QLatin1String("source")
                        || m_draftSpdif.value(unitId).value(QStringLiteral("role")).toString() == QLatin1String("source");
    if (!source) return false;
    int mask = spdifMaskOf(unitId);
    mask = on ? (mask | (1 << slot)) : (mask & ~(1 << slot));
    // A source carries its time base on one slot at least (the hub says so too)
    if (mask == 0) return false;
    return spdifEdit(unitId, { { QStringLiteral("txMask"), mask } });
}

bool SystemController::setSpdifD(const QString &unitId, const QString &samples) {
    if (spdifRow(unitId).value(QStringLiteral("role")).toString() != QLatin1String("source")) return false;
    const QString t = samples.trimmed();
    if (t.isEmpty()) return spdifEdit(unitId, { { QStringLiteral("setD"), true }, { QStringLiteral("d"), QVariant() } });
    bool ok = false;
    const int d = t.toInt(&ok);
    if (!ok || d < 0 || d > 65535) return false;
    return spdifEdit(unitId, { { QStringLiteral("setD"), true }, { QStringLiteral("d"), d } });
}

bool SystemController::addSpdif(const QString &unitId, const QString &as) {
    if (unitById(spdifCandidates(), unitId).isEmpty()) return false;
    if (as == QLatin1String("source")) return spdifEdit(unitId, { { QStringLiteral("role"), QStringLiteral("source") } });
    if (unitById(spdifSources(), as).isEmpty()) return false;
    return spdifEdit(unitId, { { QStringLiteral("role"), QStringLiteral("follower") }, { QStringLiteral("source"), as } });
}

bool SystemController::removeSpdif(const QString &unitId) {
    if (spdifRow(unitId).isEmpty()) return false;
    return spdifEdit(unitId, { { QStringLiteral("role"), QStringLiteral("off") } });
}

int SystemController::spdifMaskOf(const QString &unitId) const {
    const QVariantMap e = m_draftSpdif.value(unitId);
    if (e.contains(QStringLiteral("txMask"))) return e.value(QStringLiteral("txMask")).toInt();
    if (e.value(QStringLiteral("role")).toString() == QLatin1String("source") && spdifRow(unitId).isEmpty()) return 0b1000;
    return spdifRow(unitId).value(QStringLiteral("txMask")).toInt();
}

QString SystemController::spdifDText(const QString &unitId) const {
    const QVariantMap e = m_draftSpdif.value(unitId);
    if (e.value(QStringLiteral("setD")).toBool()) {
        const QVariant d = e.value(QStringLiteral("d"));
        return d.isValid() && !d.isNull() ? QString::number(d.toInt()) : QString();
    }
    const QVariant d = spdifRow(unitId).value(QStringLiteral("d"));
    return d.isValid() && !d.isNull() ? QString::number(d.toInt()) : QString();
}

QString SystemController::spdifSourceOf(const QString &unitId) const {
    const QVariantMap e = m_draftSpdif.value(unitId);
    if (e.contains(QStringLiteral("source"))) return e.value(QStringLiteral("source")).toString();
    return spdifRow(unitId).value(QStringLiteral("source")).toString();
}

QStringList SystemController::spdifDrafted() const {
    QStringList out;
    const auto nameOf = [this](const QString &id) {
        const QString n = unitById(m_units, id).value(QStringLiteral("name")).toString();
        return n.isEmpty() ? id : n;
    };
    for (auto it = m_draftSpdif.cbegin(); it != m_draftSpdif.cend(); ++it) {
        const QVariantMap e = it.value();
        const QString role = e.value(QStringLiteral("role")).toString();
        const QString name = nameOf(it.key());
        if (role == QLatin1String("source")) out << tr("%1 becomes an S/PDIF time-base source.").arg(name);
        else if (role == QLatin1String("follower"))
            out << tr("%1 is fed by S/PDIF from %2.").arg(name, nameOf(e.value(QStringLiteral("source")).toString()));
        else if (role == QLatin1String("off")) out << tr("%1 goes back on the audio link.").arg(name);
        if (e.contains(QStringLiteral("txMask")))
            out << tr("%1 sends the time base on: %2.").arg(name, [](int mask) {
                QStringList on;
                for (int n = 0; n < 4; ++n)
                    if (mask & (1 << n)) on << QStringLiteral("%1-%2").arg(2 * n + 1).arg(2 * n + 2);
                return on.join(QStringLiteral(", "));
            }(e.value(QStringLiteral("txMask")).toInt()));
        if (e.value(QStringLiteral("setD")).toBool()) {
            const QVariant d = e.value(QStringLiteral("d"));
            out << (d.isValid() && !d.isNull() ? tr("%1's D becomes %2 samples.").arg(name).arg(d.toInt())
                                               : tr("%1's D becomes the hub's.").arg(name));
        }
    }
    return out;
}
