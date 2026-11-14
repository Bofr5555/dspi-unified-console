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
    // -- WP HUBADMIN -- the hub's admin events (a core thread; no I/O to set)
    if (FfiCore *h = ctx.core->handle()) dspi_hub_admin_set_listener(h, &LimitsController::hubEventThunk, this);
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

// -- WP HUBADMIN -- Waits for an event in progress, so none reaches this
// object afterwards (a freed core has no handle and sends none)
LimitsController::~LimitsController() {
    if (FfiCore *h = m_ctx.core ? m_ctx.core->handle() : nullptr) dspi_hub_admin_set_listener(h, nullptr, nullptr);
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
LimitsController::Read LimitsController::readAll(FfiCore *c, bool hub, bool hubState) {
    Read r;
    r.status = object(once([c](char *b, uint32_t n) { return dspi_limits_status(c, b, n); }));
    // -- WP HUBADMIN -- which copy; the hub's state when it is the hub's (then
    // the target again: it takes the role the hub just answered with)
    r.target = object(once([c](char *b, uint32_t n) { return dspi_installer_target(c, b, n); }, 4096));
    if (hubState && r.target.value(QLatin1String("copy")).toString() == QLatin1String("hub")) {
        r.hubRead = true;
        r.hub = object(once([c](char *b, uint32_t n) { return dspi_hub_limits_get(c, b, n); }, 1 << 20));
        const QJsonObject live = object(once([c](char *b, uint32_t n) { return dspi_installer_target(c, b, n); }, 4096));
        if (isOk(live)) r.target = live;
    }
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
    // -- WP HUBADMIN -- the hub's copy: its profile is the page's
    m_target = isOk(r.target) ? r.target : QJsonObject();
    if (r.hubRead || !hubCopy()) m_hub = r.hub;
    if (hubCopy()) {
        m_limits = object(m_hub.value(QLatin1String("profileDocument")).toString().toUtf8()).value(QLatin1String("limits")).toObject();
        // Its holds come with each change (postHeld) and the hub's own banner
        m_heldSeen = lastHeld();
        m_primeHeld = false;
        emit changed();
        emit lockChanged();
        return;
    }
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
        m_target = m_hub = QJsonObject();   // -- WP HUBADMIN --
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
        m_hub = QJsonObject();   // -- WP HUBADMIN -- another device, another copy
        m_hubDue = true;
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
    // -- WP HUBADMIN -- the hub's copy once per connection (a round trip)
    const bool hubState = m_hubDue;
    m_hubDue = false;
    m_ctx.core->worker()->call(this, [hub, hubState](FfiCore *c) { return readAll(c, hub, hubState); }, [this](const Read &r) {
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
    // -- WP HUBADMIN -- the hub keeps a profile or none
    if (hubCopy()) return active() ? QStringLiteral("ok") : QStringLiteral("none");
    const QString s = m_status.value(QLatin1String("state")).toString();
    return s.isEmpty() ? QStringLiteral("none") : s;
}

QString LimitsController::standingNotice() const {
    if (hubCopy()) return QString();   // -- WP HUBADMIN -- this console's file is not in force
    const QString key = m_status.value(QLatin1String("noticeKey")).toString();
    if (key.isEmpty()) return QString();
    const NoticeCenter *n = notices();
    const QString detail = m_status.value(QLatin1String("detail")).toString();
    return n ? n->line(key, detail) : detail;
}

QStringList LimitsController::lastHeld() const {
    QStringList out;
    // -- WP HUBADMIN -- what the hub last held the device to
    if (hubCopy()) {
        for (const QJsonValue &v : m_hub.value(QLatin1String("held")).toArray()) out << v.toString();
        return out;
    }
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
    // -- WP HUBADMIN -- the hub's copy: its profile goes
    if (hubCopy()) {
        adminCall([](FfiCore *c) { return once([&](char *b, uint32_t n) { return dspi_hub_limits_clear(c, b, n); }); }, {}, false);
        return;
    }
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
    // -- WP HUBADMIN -- the hub's copy: set on the hub; what it held at once
    // is shown as the console's clamp report is
    if (hubCopy()) {
        emit changed();
        adminCall([text](FfiCore *c) { return once([&](char *b, uint32_t n) { return dspi_hub_limits_set(c, text.constData(), b, n); }); },
                  [this](const QJsonObject &reply) {
                      if (isOk(reply)) postHeld(reply);
                  },
                  false);
        return;
    }
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

bool LimitsController::covers() const {
    // -- WP HUBADMIN -- the hub's profile names what its lock covers
    if (hubCopy()) return !m_limits.value(QLatin1String("lockedPages")).toArray().isEmpty();
    return !m_lock.value(QLatin1String("covers")).toArray().isEmpty();
}

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
    // -- WP HUBADMIN -- the hub's PIN (an Admin needs no current one); its
    // recovery code is shown this once, with the hub's row
    if (hubCopy()) {
        adminCall([p](FfiCore *c) { return once([&](char *b, uint32_t n) { return dspi_hub_pin_set(c, p.constData(), b, n); }); },
                  [this](const QJsonObject &reply) {
                      if (!isOk(reply)) return;
                      m_recoveryCode = reply.value(QLatin1String("recoveryCode")).toString();
                      m_hubRecovery = true;
                      emit lockChanged();
                  },
                  true);
        return;
    }
    m_hubRecovery = false;
    pinCall([p](FfiCore *c) { return once([&](char *b, uint32_t n) { return dspi_installer_pin_set(c, p.constData(), b, n); }); },
            [this](const QJsonObject &reply) {
                if (!isOk(reply)) return;
                m_recoveryCode = reply.value(QLatin1String("recoveryCode")).toString();
                emit lockChanged();
            });
}

void LimitsController::clearPin() {
    // -- WP HUBADMIN --
    if (hubCopy()) {
        adminCall([](FfiCore *c) { return once([&](char *b, uint32_t n) { return dspi_hub_pin_clear(c, b, n); }); },
                  [this](const QJsonObject &reply) {
                      if (!isOk(reply)) return;
                      m_recoveryCode.clear();
                      emit lockChanged();
                  },
                  true);
        return;
    }
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
    // -- WP HUBADMIN -- what the hub's lock covers is its profile's lockedPages
    if (hubCopy()) {
        if (pages.isEmpty()) m_limits.remove(QStringLiteral("lockedPages"));
        else m_limits.insert(QStringLiteral("lockedPages"), pages);
        sendProfile(QStringLiteral("limits lockedPages"));
        return;
    }
    const QByteArray json = QJsonDocument(pages).toJson(QJsonDocument::Compact);
    pinCall([json](FfiCore *c) { return once([&](char *b, uint32_t n) { return dspi_installer_lock_cover(c, json.constData(), b, n); }); });
}

void LimitsController::dismissRecovery() {
    if (m_recoveryCode.isEmpty()) return;
    m_recoveryCode.clear();
    emit lockChanged();
}

// ---- -- WP HUBADMIN -- The hub's copy -------------------------------------------

bool LimitsController::active() const {
    if (hubCopy()) return !m_hub.value(QLatin1String("profileDocument")).toString().isEmpty();
    return m_status.value(QLatin1String("active")).toBool();
}

bool LimitsController::pinSet() const {
    if (hubCopy()) return m_hub.value(QLatin1String("pinSet")).toBool();
    return m_lock.value(QLatin1String("pinSet")).toBool();
}

QString LimitsController::copy() const {
    if (!connected()) return QStringLiteral("none");
    const QString c = m_target.value(QLatin1String("copy")).toString();
    // Not read yet: this console's, as before
    return c.isEmpty() ? QStringLiteral("console") : c;
}

bool LimitsController::editable() const {
    if (!connected()) return false;
    if (m_target.isEmpty()) return true;
    // The hub's copy that could not be read is not edited blind
    if (hubCopy() && !isOk(m_hub)) return false;
    return m_target.value(QLatin1String("editable")).toBool();
}

QString LimitsController::copyText() const {
    const QString key = m_target.value(QLatin1String("noticeKey")).toString();
    const NoticeCenter *n = notices();
    return key.isEmpty() || !n ? QString() : n->line(key);
}

QString LimitsController::readOnlyText() const {
    if (editable()) return QString();
    QString key = m_target.value(QLatin1String("readOnlyKey")).toString();
    // The hub's state not read: the hub's reason (a hub without limits, the session down)
    if (key.isEmpty() && hubCopy()) key = m_hub.value(QLatin1String("noticeKey")).toString();
    const NoticeCenter *n = notices();
    if (key.isEmpty() || !n) return QString();
    return n->line(key, m_hub.value(QLatin1String("error")).toString());
}

QVariantList LimitsController::modePins() const {
    QVariantList out;
    if (!hubCopy()) return out;
    for (const QJsonValue &v : m_hub.value(QLatin1String("pinnedModes")).toArray()) {
        const QJsonObject p = v.toObject();
        out << QVariantMap{ { QStringLiteral("role"), p.value(QLatin1String("role")).toString() },
                            { QStringLiteral("mode"), p.value(QLatin1String("mode")).toString() } };
    }
    return out;
}

QString LimitsController::recoveryKey() const {
    return m_hubRecovery ? QStringLiteral("installer_lock.hub_recovery") : QStringLiteral("installer_lock.recovery");
}

void LimitsController::setModePin(const QString &role, const QString &mode) {
    if (!hubCopy()) return;
    const QByteArray r = role.toUtf8(), m = mode.toUtf8();
    adminCall([r, m](FfiCore *c) {
        return once([&](char *b, uint32_t n) { return dspi_hub_mode_pin(c, r.constData(), m.constData(), b, n); });
    },
              {}, false);
}

void LimitsController::adminCall(std::function<QByteArray(FfiCore *)> call, std::function<void(const QJsonObject &)> then,
                                 bool asPinError) {
    if (!connected()) return;
    track(1);
    m_ctx.core->worker()->call(
        this,
        [call](FfiCore *c) {
            const QJsonObject reply = object(call(c));
            Read after = readAll(c, false);   // after the call, in this order
            return std::make_pair(reply, after);
        },
        [this, then, asPinError](const std::pair<QJsonObject, Read> &r) {
            if (asPinError) setPinError(r.first);
            else if (!isOk(r.first))
                if (auto *n = notices()) n->postKey(r.first.value(QLatin1String("noticeKey")).toString(), r.first.value(QLatin1String("error")).toString());
            apply(r.second);
            if (then) then(r.first);
            track(-1);
        });
}

// A core thread, no core lock held: copy the event and return
void LimitsController::hubEventThunk(const char *, const char *eventJson, void *self) {
    auto *lc = static_cast<LimitsController *>(self);
    const QByteArray json(eventJson ? eventJson : "");
    QMetaObject::invokeMethod(lc, [lc, json] { lc->onHubEvent(object(json)); }, Qt::QueuedConnection);
}

// The hub changed its copy (another Admin, or this one): the lock the core
// read again at once, then the page from the hub
void LimitsController::onHubEvent(const QJsonObject &event) {
    if (!connected()) return;
    const QString serial = event.value(QLatin1String("serial")).toString();
    const QString open = m_target.value(QLatin1String("serial")).toString();
    if (!serial.isEmpty() && !open.isEmpty() && serial != open) return;
    const QJsonObject lock = event.value(QLatin1String("lock")).toObject();
    if (!lock.isEmpty()) {
        QJsonObject withOk = lock;
        withOk.insert(QStringLiteral("ok"), true);
        m_lock = withOk;
        emit lockChanged();
    }
    refresh();
}
