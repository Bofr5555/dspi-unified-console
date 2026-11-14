// -- WP P1QT2 -- Settings > Installer: limits, the PIN and the lock
// (LimitsController.h; PLAN-CLIENTS 5.9, OD-P1c, OD-P1d, OD-P1e)
#include "LimitsController.h"

#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "NoticeCenter.h"
#include "SettingsController.h"
#include "Snapshot.h"
#include "ToolsController.h"

#include <QDir>
#include <QJSEngine>
#include <QJsonArray>
#include <QJsonDocument>
#include <QStandardPaths>
#include <cmath>
#include <memory>

namespace {

const Registrar::Controller<LimitsController> limitsController;
const Registrar::Banner lockBanner("LockBanner", 12);

// Settings > Installer, under Advanced and Networking (OD-P1e). The search
// finds its rows.
const SettingsRegistry::Page installerPage{
    "installer", SettingsRegistry::Application, 45, QT_TRANSLATE_NOOP("SettingsPages", "Installer"), "key-round",
    "InstallerPage",
    { QT_TRANSLATE_NOOP("SettingsPages", "Limits"), QT_TRANSLATE_NOOP("SettingsPages", "Installer PIN"),
      QT_TRANSLATE_NOOP("SettingsPages", "Lock"), QT_TRANSLATE_NOOP("SettingsPages", "Unlock"),
      QT_TRANSLATE_NOOP("SettingsPages", "Recovery code"), QT_TRANSLATE_NOOP("SettingsPages", "Maximum gain"),
      QT_TRANSLATE_NOOP("SettingsPages", "Limiter ceiling"), QT_TRANSLATE_NOOP("SettingsPages", "Master volume"),
      QT_TRANSLATE_NOOP("SettingsPages", "Installer's note") }
};

// The fields of a profile's output row and its own (limits-profiles.md)
const char *const kOutputFields[] = { "maxGainDb", "limiterCeilingDb", "highpassMinHz", "highpassMinSlopeDb", "delayMinMs",
                                      "delayMaxMs" };
const char *const kProfileFields[] = { "maxPeqBoostDb", "masterVolumeMaxDb" };
const char *const kTextFields[] = { "note", "contact" };

// One call, one buffer: these replies are small, and a PIN call must not run twice
QByteArray once(const std::function<uint32_t(char *, uint32_t)> &fill, uint32_t len = 64 * 1024) {
    QByteArray buf(int(len), '\0');
    const uint32_t need = fill(buf.data(), len);
    if (need == 0 || need > len) return QByteArray();
    buf.truncate(int(need) - 1);
    return buf;
}

QJsonObject object(const QByteArray &reply) { return QJsonDocument::fromJson(reply).object(); }
bool isOk(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }

template <size_t N>
bool oneOf(const QString &field, const char *const (&list)[N]) {
    for (const char *f : list)
        if (field == QLatin1String(f)) return true;
    return false;
}

NoticeCenter *notices() { return Registrar::instance<NoticeCenter>(); }

} // namespace

LimitsController::LimitsController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    // Named before any device opens (installer-lock.md: with a device open
    // whose PIN is set, naming it needs the console unlocked), and the
    // confirmation tokens on from the start (P1TK)
    const QString dir = QStandardPaths::writableLocation(QStandardPaths::AppDataLocation) + QStringLiteral("/limits");
    const QByteArray d = QDir::toNativeSeparators(dir).toUtf8();
    ctx.core->worker()->post([d](FfiCore *c) {
        once([&](char *b, uint32_t n) { return dspi_limits_set_dir(c, d.constData(), b, n); }, 1024);
        dspi_require_confirmation(c, true);
    });
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &LimitsController::onSnapshot);
    // -- WP P1EFIX -- the editor's ranges: the core's table, no copy here. The
    // call takes no core handle and does not block (stateless text).
    const QJsonArray rows = object(readSized([](char *b, uint32_t n) { return dspi_limits_field_ranges(b, n); }, 4096))
                                .value(QLatin1String("value"))
                                .toArray();
    for (const QJsonValue &row : rows) {
        const QJsonObject r = row.toObject();
        m_ranges.insert(r.value(QLatin1String("key")).toString(), r.toVariantMap());
    }
}

LimitsController *LimitsController::create(QQmlEngine *, QJSEngine *) {
    LimitsController *self = Registrar::instance<LimitsController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

bool LimitsController::connected() const { return m_ctx.device->connected(); }

void LimitsController::track(int delta) {
    const bool was = busy();
    m_busy += delta;
    if (busy() != was) emit busyChanged();
}

// ---- Reading ---------------------------------------------------------------------

// On the worker: the status, the lock (asking the hub first on a new
// connection: one lock, the hub's when it holds one), the profile's document
// and, once, the screens a lock covers
LimitsController::Read LimitsController::readAll(FfiCore *c, bool hub) {
    Read r;
    r.status = object(once([c](char *b, uint32_t n) { return dspi_limits_status(c, b, n); }));
    QJsonObject lock;
    if (hub) lock = object(once([c](char *b, uint32_t n) { return dspi_installer_lock_refresh(c, b, n); }));
    if (!isOk(lock)) lock = object(once([c](char *b, uint32_t n) { return dspi_installer_lock_state(c, b, n); }));
    r.lock = lock;
    const QByteArray doc = once([c](char *b, uint32_t n) { return dspi_limits_profile_document(c, b, n); }, 1 << 20);
    r.doc = object(doc);
    // The manifest: every screen the locked user does not see (no handle, no I/O)
    const QJsonObject manifest = object(once([](char *b, uint32_t n) { return dspi_mode_manifest(b, n); }));
    QStringList front;
    for (const QJsonValue &v : manifest.value(QLatin1String("lockedUser")).toObject().value(QLatin1String("screens")).toArray())
        front << v.toString();
    for (const QJsonValue &v : manifest.value(QLatin1String("screens")).toArray()) {
        const QString id = v.toObject().value(QLatin1String("id")).toString();
        if (!id.isEmpty() && !front.contains(id)) r.screens << id;
    }
    return r;
}

void LimitsController::apply(const Read &r) {
    m_status = r.status;
    if (isOk(r.lock)) m_lock = r.lock;
    m_limits = r.doc.value(QLatin1String("limits")).toObject();
    if (!r.screens.isEmpty()) m_screens = r.screens;
    // A hold the person has not been told of (a preset slot loaded under the
    // profile, OD-P1j): the core's report as a notice
    const QStringList held = lastHeld();
    if (!m_primeHeld && !held.isEmpty() && held != m_heldSeen)
        if (auto *n = notices()) n->post(held.join(QLatin1Char('\n')));
    m_primeHeld = false;   // a new connection's first read: what it held before is old news
    m_heldSeen = held;
    emit changed();
    emit lockChanged();
}

void LimitsController::onSnapshot() {
    const DeviceSnapshot &snap = m_ctx.device->snapshot();
    if (!snap.connected) {
        if (m_generation == 0) return;
        m_generation = 0;
        m_status = m_lock = m_limits = QJsonObject();
        m_recoveryCode.clear();
        m_pinError.clear();
        m_heldSeen.clear();
        emit changed();
        emit lockChanged();
        return;
    }
    const bool fresh = snap.generation != m_generation;
    m_generation = snap.generation;
    if (fresh) {
        m_primeHeld = true;
        m_recoveryCode.clear();
        m_pinError.clear();
        m_heldSeen.clear();
    }
    // One read at a time; a snapshot meanwhile asks for one more
    if (m_reading) {
        m_readAgain = true;
        return;
    }
    m_reading = true;
    const bool hub = fresh && m_ctx.device->remote();
    m_ctx.core->worker()->call(this, [hub](FfiCore *c) { return readAll(c, hub); }, [this](const Read &r) {
        m_reading = false;
        apply(r);
        if (m_readAgain) {
            m_readAgain = false;
            onSnapshot();
        }
    });
}

void LimitsController::refresh() {
    if (!connected()) return;
    track(1);
    m_ctx.core->worker()->call(this, [](FfiCore *c) { return readAll(c, false); }, [this](const Read &r) {
        apply(r);
        track(-1);
    });
}

// ---- The profile -----------------------------------------------------------------

QString LimitsController::profileState() const {
    const QString s = m_status.value(QLatin1String("state")).toString();
    return s.isEmpty() ? QStringLiteral("none") : s;
}

QString LimitsController::standingNotice() const {
    const QString key = m_status.value(QLatin1String("noticeKey")).toString();
    if (key.isEmpty()) return QString();
    const NoticeCenter *n = notices();
    const QString detail = m_status.value(QLatin1String("detail")).toString();
    return n ? n->line(key, detail) : detail;
}

QStringList LimitsController::lastHeld() const {
    QStringList out;
    for (const QJsonValue &v : m_status.value(QLatin1String("lastHeld")).toArray()) out << v.toString();
    return out;
}

QVariantList LimitsController::outputs() const {
    QVariantList list;
    const int n = m_ctx.device->numOutputs();
    const ToolsController *tools = Registrar::instance<ToolsController>();
    for (int o = 0; o < n; ++o) {
        QVariantMap row{ { QStringLiteral("output"), o },
                         { QStringLiteral("name"), tools ? tools->outputName(o) : tr("Out %1").arg(o + 1) } };
        for (const QJsonValue &v : m_limits.value(QLatin1String("outputs")).toArray()) {
            const QJsonObject r = v.toObject();
            if (r.value(QLatin1String("output")).toInt(-1) != o) continue;
            for (const char *f : kOutputFields)
                if (r.value(QLatin1String(f)).isDouble()) row.insert(QLatin1String(f), r.value(QLatin1String(f)).toDouble());
        }
        list << row;
    }
    return list;
}

QVariantMap LimitsController::limits() const {
    QVariantMap out;
    for (const char *f : kProfileFields)
        if (m_limits.value(QLatin1String(f)).isDouble()) out.insert(QLatin1String(f), m_limits.value(QLatin1String(f)).toDouble());
    for (const char *f : kTextFields) out.insert(QLatin1String(f), m_limits.value(QLatin1String(f)).toString());
    return out;
}

void LimitsController::setOutputLimit(int output, const QString &field, double value) {
    if (output < 0 || output >= m_ctx.device->numOutputs() || !oneOf(field, kOutputFields)) return;
    QJsonArray rows = m_limits.value(QLatin1String("outputs")).toArray();
    int at = -1;
    for (int i = 0; i < rows.size(); ++i)
        if (rows.at(i).toObject().value(QLatin1String("output")).toInt(-1) == output) at = i;
    QJsonObject row = at >= 0 ? rows.at(at).toObject() : QJsonObject{ { QStringLiteral("output"), output } };
    if (std::isfinite(value)) row.insert(field, value);
    else row.remove(field);
    // The slope only means something with the corner
    if (field == QLatin1String("highpassMinHz") && !std::isfinite(value)) row.remove(QStringLiteral("highpassMinSlopeDb"));
    const bool empty = row.size() <= 1;
    if (at >= 0) {
        if (empty) rows.removeAt(at);
        else rows.replace(at, row);
    } else if (!empty) {
        rows.append(row);
    }
    if (rows.isEmpty()) m_limits.remove(QStringLiteral("outputs"));
    else m_limits.insert(QStringLiteral("outputs"), rows);
    sendProfile(QStringLiteral("limits output %1 %2").arg(output).arg(field));
}

void LimitsController::setLimit(const QString &field, double value) {
    if (!oneOf(field, kProfileFields)) return;
    if (std::isfinite(value)) m_limits.insert(field, value);
    else m_limits.remove(field);
    sendProfile(QStringLiteral("limits %1").arg(field));
}

void LimitsController::setText(const QString &field, const QString &text) {
    if (!oneOf(field, kTextFields)) return;
    const QString t = text.trimmed();
    if (t == m_limits.value(field).toString()) return;
    if (t.isEmpty()) m_limits.remove(field);
    else m_limits.insert(field, t);
    sendProfile(QStringLiteral("limits %1").arg(field));
}

void LimitsController::clearLimits() {
    // What the lock covers lives in the profile: kept while a PIN is set
    const QJsonValue pages = m_limits.value(QLatin1String("lockedPages"));
    m_limits = QJsonObject();
    if (pinSet() && pages.isArray() && !pages.toArray().isEmpty()) {
        m_limits.insert(QStringLiteral("lockedPages"), pages);
        sendProfile(QStringLiteral("limits clear"));
        return;
    }
    if (!connected()) return;
    track(1);
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            const QJsonObject reply = object(once([c](char *b, uint32_t n) { return dspi_limits_clear(c, b, n); }));
            Read after = readAll(c, false);   // after the call, in this order
            return std::make_pair(reply, after);
        },
        [this](const std::pair<QJsonObject, Read> &r) {
            if (!isOk(r.first))
                if (auto *n = notices()) n->postKey(r.first.value(QLatin1String("noticeKey")).toString(), r.first.value(QLatin1String("error")).toString());
            apply(r.second);
            track(-1);
        });
}

void LimitsController::sendProfile(const QString &what) {
    Q_UNUSED(what)
    if (!connected()) return;
    QJsonObject limits = m_limits;
    limits.remove(QStringLiteral("serial"));   // the open device's, the core's to fill
    const QJsonObject doc{ { QStringLiteral("schemaVersion"), 1 },
                           { QStringLiteral("meta"), QJsonObject{ { QStringLiteral("kind"), QStringLiteral("limits") },
                                                                  { QStringLiteral("name"), tr("Limits profile") } } },
                           { QStringLiteral("limits"), limits } };
    const QByteArray text = QJsonDocument(doc).toJson(QJsonDocument::Compact);
    track(1);
    emit changed();
    m_ctx.core->worker()->call(
        this,
        [text](FfiCore *c) {
            const QJsonObject reply = object(once([&](char *b, uint32_t n) { return dspi_limits_apply_profile(c, text.constData(), b, n); }));
            Read after = readAll(c, false);   // after the call, in this order
            return std::make_pair(reply, after);
        },
        [this](const std::pair<QJsonObject, Read> &r) {
            if (isOk(r.first)) {
                postHeld(r.first);
                // Shown just now: not again as a hold from elsewhere
                m_heldSeen.clear();
                for (const QJsonValue &v : r.second.status.value(QLatin1String("lastHeld")).toArray()) m_heldSeen << v.toString();
            } else if (auto *n = notices()) {
                n->postKey(r.first.value(QLatin1String("noticeKey")).toString(), r.first.value(QLatin1String("error")).toString());
            }
            apply(r.second);
            track(-1);
        });
}

// What applying the profile held, in the import's words (the core's lines)
void LimitsController::postHeld(const QJsonObject &reply) {
    QStringList lines;
    for (const QJsonValue &v : reply.value(QLatin1String("lines")).toArray()) lines << v.toString();
    if (lines.isEmpty())
        for (const QJsonValue &v : reply.value(QLatin1String("held")).toArray())
            if (v.isString()) lines << v.toString();
    if (!lines.isEmpty())
        if (auto *n = notices()) n->post(lines.join(QLatin1Char('\n')));
}

// ---- The PIN and the lock --------------------------------------------------------

bool LimitsController::installerOpen() const {
    // No device, or no PIN: nothing to ask
    if (!connected() || !m_lock.contains(QLatin1String("installerOpen"))) return true;
    return m_lock.value(QLatin1String("installerOpen")).toBool();
}

bool LimitsController::covers() const { return !m_lock.value(QLatin1String("covers")).toArray().isEmpty(); }

QString LimitsController::scopeText() const {
    QString key = m_lock.value(QLatin1String("scopeNoticeKey")).toString();
    if (key.isEmpty()) key = heldByHub() ? QStringLiteral("installer_lock.hub") : QStringLiteral("installer_lock.scope");
    const NoticeCenter *n = notices();
    return n ? n->sentence(key) : QString();
}

void LimitsController::setPinError(const QJsonObject &reply) {
    QString text;
    if (!isOk(reply)) {
        const QString key = reply.value(QLatin1String("noticeKey")).toString();
        const NoticeCenter *n = notices();
        text = n ? n->line(key, reply.value(QLatin1String("error")).toString()) : reply.value(QLatin1String("error")).toString();
        if (text.isEmpty()) text = tr("That did not work.");
    }
    if (text == m_pinError) return;
    m_pinError = text;
    emit lockChanged();
}

void LimitsController::pinCall(std::function<QByteArray(FfiCore *)> call, std::function<void(const QJsonObject &)> then) {
    if (!connected()) return;
    track(1);
    m_ctx.core->worker()->call(
        this,
        [call](FfiCore *c) {
            const QJsonObject reply = object(call(c));
            Read after = readAll(c, false);   // after the call: the order of make_pair's arguments is not
            return std::make_pair(reply, after);
        },
        [this, then](const std::pair<QJsonObject, Read> &r) {
            setPinError(r.first);
            apply(r.second);
            if (then) then(r.first);
            track(-1);
        });
}

void LimitsController::setPin(const QString &pin, const QString &repeat) {
    if (pin != repeat) {
        m_pinError = tr("The two PINs are not the same.");
        emit lockChanged();
        return;
    }
    const QByteArray p = pin.toUtf8();
    pinCall([p](FfiCore *c) { return once([&](char *b, uint32_t n) { return dspi_installer_pin_set(c, p.constData(), b, n); }); },
            [this](const QJsonObject &reply) {
                if (!isOk(reply)) return;
                m_recoveryCode = reply.value(QLatin1String("recoveryCode")).toString();
                emit lockChanged();
            });
}

void LimitsController::clearPin() {
    pinCall([](FfiCore *c) { return once([&](char *b, uint32_t n) { return dspi_installer_pin_clear(c, b, n); }); },
            [this](const QJsonObject &reply) {
                if (!isOk(reply)) return;
                m_recoveryCode.clear();
                emit lockChanged();
            });
}

void LimitsController::unlock(const QString &pin) {
    const QByteArray p = pin.toUtf8();
    pinCall([p](FfiCore *c) { return once([&](char *b, uint32_t n) { return dspi_installer_unlock(c, p.constData(), b, n); }); },
            [this](const QJsonObject &reply) { emit unlockFinished(isOk(reply)); });
}

void LimitsController::lockNow() {
    m_recoveryCode.clear();
    pinCall([](FfiCore *c) { return once([&](char *b, uint32_t n) { return dspi_installer_lock(c, b, n); }); });
}

void LimitsController::setCovers(bool on) {
    QJsonArray pages;
    if (on)
        for (const QString &s : m_screens) pages.append(s);
    const QByteArray json = QJsonDocument(pages).toJson(QJsonDocument::Compact);
    pinCall([json](FfiCore *c) { return once([&](char *b, uint32_t n) { return dspi_installer_lock_cover(c, json.constData(), b, n); }); });
}

void LimitsController::dismissRecovery() {
    if (m_recoveryCode.isEmpty()) return;
    m_recoveryCode.clear();
    emit lockChanged();
}
