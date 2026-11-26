// -- WP P1QT1 -- Device > Snapshots…, the A/B compare and the safe apply's
// countdown bar (PLAN-CLIENTS 5.9: P1SN, P1AB, P1SA)
#include "SnapshotsController.h"

#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceSession.h"   // -- WP MDQT2b --
#include "DeviceRegistry.h"   // -- WP AIVIS --
#include "FilesController.h"
#include "HubFeed.h"   // -- WP P2QT --
#include "NoticeCenter.h"
#include "Shell.h"
#include "Snapshot.h"

#include "CoreApi.h"

#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QGuiApplication>
#include <QHash>
#include <QJSEngine>
#include <QJsonArray>
#include <QJsonDocument>
#include <QRegularExpression>
#include <QSaveFile>
#include <QSet>
#include <QSettings>
#include <QWindow>
#include <QStandardPaths>
#include <algorithm>
#include <memory>

static const Registrar::Controller<SnapshotsController> snapshotsController;
static const Registrar::Window snapshotsWindow("device.snapshots", "SnapshotsWindow");
static const Registrar::Banner safeApplyBanner("SafeApplyBanner", 5, Registrar::Blocking);

namespace {
// A reply of a call that changes the device: far more than it needs
constexpr uint32_t kReplyBytes = 256 << 10;
// The core refuses a document over 4 MB; one byte more is enough for it to say so
constexpr qint64 kReadLimit = (4 << 20) + 1;
// -- WP P1DCFIX -- the safe apply's timeout, remembered on this console
const QString kTimeoutKey = QStringLiteral("snapshots/guardTimeoutS");
// The notice the countdown says (P1NT's table; `{s}` the seconds left)
constexpr const char *kWaitingNotice = "guard.waiting";

QByteArray once(const std::function<uint32_t(char *, uint32_t)> &fill) {
    QByteArray buf(int(kReplyBytes), '\0');
    const uint32_t need = fill(buf.data(), kReplyBytes);
    if (need == 0 || need > kReplyBytes) return QByteArray();
    buf.truncate(int(need) - 1);
    return buf;
}

QJsonObject object(const QByteArray &reply) { return QJsonDocument::fromJson(reply).object(); }

bool isOk(const QJsonObject &o) { return o.value(QLatin1String("ok")).toBool(); }

// A C string argument, or null for ""
const char *orNull(const QByteArray &s) { return s.isEmpty() ? nullptr : s.constData(); }

// A text field has the keyboard: the A/B key is the field's, not the device's
bool typing() {
    QObject *focus = QGuiApplication::focusObject();
    return focus && (focus->inherits("QQuickTextInput") || focus->inherits("QQuickTextEdit"));
}

// -- WP P1DCFIX -- a dialog is open (a modal window, or a modal popup of the
// window with the keyboard): the bare A/B key is not the device's then either
bool dialogOpen() {
    if (QGuiApplication::modalWindow()) return true;
    QWindow *window = QGuiApplication::focusWindow();
    if (!window) return false;
    const QList<QObject *> all = window->findChildren<QObject *>();
    for (QObject *o : all)
        if (o->inherits("QQuickPopup") && o->property("modal").toBool() && o->property("opened").toBool()) return true;
    return false;
}

QVariantMap row(const QJsonObject &s) {
    const qint64 ms = qint64(s.value(QLatin1String("savedMs")).toDouble());
    return {
        { QStringLiteral("id"), s.value(QLatin1String("id")).toString() },
        { QStringLiteral("name"), s.value(QLatin1String("name")).toString() },
        { QStringLiteral("note"), s.value(QLatin1String("note")).toString() },
        { QStringLiteral("savedMs"), double(ms) },
        { QStringLiteral("saved"), ms > 0 ? QDateTime::fromMSecsSinceEpoch(ms).toLocalTime().toString(QStringLiteral("yyyy-MM-dd HH:mm")) : QString() },
        { QStringLiteral("platform"), s.value(QLatin1String("platform")).toString() },
        { QStringLiteral("inputs"), s.value(QLatin1String("inputChannels")).toInt() },
        { QStringLiteral("outputs"), s.value(QLatin1String("outputChannels")).toInt() },
    };
}
} // namespace

SnapshotsController::SnapshotsController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    // The store lives in the app's data folder (one subfolder per serial, the core's)
    const QString dir = QStandardPaths::writableLocation(QStandardPaths::AppDataLocation) + QStringLiteral("/snapshots");
    const QByteArray d = QDir::toNativeSeparators(dir).toUtf8();
    ctx.core->worker()->post([d](FfiCore *c) {
        char buf[512];
        dspi_snapshot_set_dir(c, d.constData(), buf, sizeof buf);
    });
    m_poll = ctx.poller->add(
        QStringLiteral("safe apply"), kGuardPollMs, this,
        [](FfiCore *c) { return readSized([c](char *b, uint32_t n) { return dspi_apply_poll(c, b, n); }); },
        [this](QByteArray reply) {
            const QJsonObject o = object(reply);
            if (isOk(o)) setGuard(o);
        });
    ctx.poller->setEnabled(m_poll, false);
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &SnapshotsController::onSnapshotApplied);
    // -- WP P1DCFIX -- the A/B key is a bare B: never while typing or asked something
    ctx.shell->addHandler(QStringLiteral("device.abToggle"), this, [this] {
        if (!typing() && !dialogOpen()) abToggle();
    });
    // -- WP P1DCFIX -- the timeout set before, and the countdown's words (the
    // notice table: no handle, no I/O)
    m_timeoutS = std::clamp(QSettings().value(kTimeoutKey, kGuardTimeoutDefaultS).toInt(), kGuardTimeoutMinS, kGuardTimeoutMaxS);
    const QJsonObject notice = object(once([](char *b, uint32_t n) { return dspi_notice_for(kWaitingNotice, nullptr, b, n); }));
    const QString title = notice.value(QLatin1String("title")).toString();
    const QString reason = notice.value(QLatin1String("reason")).toString();
    m_waitingText = isOk(notice) && notice.value(QLatin1String("key")).toString() == QLatin1String(kWaitingNotice)
                        ? title + QLatin1Char(' ') + reason
                        : tr("Keep these settings? They go back in {s} s unless you press Keep.");
    // -- WP P2QT -- the hub's store changed (any console's save, rename or delete)
    if (HubFeed *feed = HubFeed::of(ctx.core)) connect(feed, &HubFeed::hubEvent, this, &SnapshotsController::onHubEvent);
    // -- WP MDQT2b -- the guards left waiting on other devices, at the bar's rate
    m_session = ctx.activeSession();
    m_driveTimer.setInterval(kGuardPollMs);
    connect(&m_driveTimer, &QTimer::timeout, this, &SnapshotsController::driveTick);
}

void SnapshotsController::setGuardTimeoutS(int seconds) {
    const int s = std::clamp(seconds, kGuardTimeoutMinS, kGuardTimeoutMaxS);
    if (s == m_timeoutS) return;
    m_timeoutS = s;
    QSettings().setValue(kTimeoutKey, s);
    emit guardTimeoutSChanged();
}

SnapshotsController *SnapshotsController::create(QQmlEngine *, QJSEngine *) {
    SnapshotsController *self = Registrar::instance<SnapshotsController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

QStringList SnapshotsController::guardChanges() const {
    QStringList out;
    for (const QJsonValue &v : m_guard.value(QLatin1String("changes")).toArray()) out.append(v.toString());
    return out;
}

void SnapshotsController::setRollback(bool on) {
    if (on == m_rollback) return;
    m_rollback = on;
    emit rollbackChanged();
}

void SnapshotsController::track(int delta) {
    const bool was = busy();
    m_busy += delta;
    if (busy() != was) emit busyChanged();
}

void SnapshotsController::failed(const QJsonObject &reply, const QString &fallback) {
    QString error = reply.value(QLatin1String("error")).toString();
    // -- WP MERGEP1D -- the installer's lock refused it: the core's notice row
    // for it (`installer_lock.hub` when the hub holds the lock, not the "on
    // this console" words). The table only: no handle, no I/O. The one lock
    // prompt (LockState.heldBy) comes with P1-E.
    const QByteArray key = reply.value(QLatin1String("noticeKey")).toString().toUtf8();
    if (key.startsWith("installer_lock")) {
        const QJsonObject notice = object(once([&](char *b, uint32_t n) { return dspi_notice_for(key.constData(), nullptr, b, n); }));
        if (isOk(notice) && notice.value(QLatin1String("key")).toString().toUtf8() == key)
            error = notice.value(QLatin1String("title")).toString() + QLatin1String(": ") + notice.value(QLatin1String("reason")).toString();
    }
    // -- WP P2QT -- the hub's refusals of its store (`dlink.not_found`,
    // `dlink.conflict`, ...): the row's title before the hub's words
    if (key.startsWith("dlink.")) {
        auto *notices = Registrar::instance<NoticeCenter>();
        const QString title = notices ? notices->row(QString::fromUtf8(key)).value(QStringLiteral("title")).toString() : QString();
        if (!title.isEmpty()) error = error.isEmpty() ? notices->line(QString::fromUtf8(key)) : title + QLatin1String(": ") + error;
    }
    if (auto *notices = Registrar::instance<NoticeCenter>()) notices->post(error.isEmpty() ? fallback : error, QStringLiteral("error"));
}

// ---- The device changed ------------------------------------------------------------

void SnapshotsController::onSnapshotApplied() {
    const bool switched = m_ctx.activeSession() != m_session;   // -- WP MDQT2b --
    if (switched) rebind();
    const bool connected = m_ctx.device->connected();
    const QString serial = m_ctx.device->serial();
    const bool changed = switched || connected != m_connected || serial != m_serial;
    m_connected = connected;
    m_serial = serial;
    if (changed) {
        reload();
        reloadAb();
    }
    // Over DSPi Link the default may change with the session; no I/O
    const QPointer<DeviceSession> asked = m_ctx.activeSession();   // -- WP MDQT2b --
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return dspi_apply_guard_default(c); },
        [this, asked](bool on) {
            if (asked != m_ctx.activeSession() || on == m_guardDefault) return;
            m_guardDefault = on;
            emit guardChanged();
        });
    // A lost apply: ask again once a device is back (the hub's word, or the revert offered)
    if (connected && guardPhase() == QLatin1String("lost")) reloadGuard();
}

// ---- -- WP MDQT2b -- Several devices ---------------------------------------------------

// Another device is the active one: a guard that waits on the one left is
// driven there, the new one's list, compare and guard are read
void SnapshotsController::rebind() {
    DeviceSession *left = m_session;
    m_session = m_ctx.activeSession();
    const QString phase = guardPhase();
    if (left && (phase == QLatin1String("waiting") || phase == QLatin1String("lost"))) drive(left);
    m_driven.removeAll(m_session);
    m_guard = QJsonObject();
    m_ctx.poller->setEnabled(m_poll, false);
    m_abActive = false;
    m_abSide.clear();
    m_abAName.clear();
    m_abBName.clear();
    m_abRows.clear();
    m_toggling = 0;
    emit abChanged();
    emit guardChanged();
    reloadGuard();
}

void SnapshotsController::drive(DeviceSession *session) {
    if (!session || session == m_ctx.activeSession() || m_driven.contains(session)) return;
    m_driven.append(session);
    if (!m_driveTimer.isActive()) m_driveTimer.start();
    emit guardChanged();
    driveTick();
}

// One poll of each guard driven here, on its own device's worker
void SnapshotsController::driveTick() {
    m_driven.removeAll(nullptr);
    for (const QPointer<DeviceSession> &s : std::as_const(m_driven)) {
        if (s == m_ctx.activeSession() || m_driving.contains(s.data())) continue;
        m_driving.insert(s.data());
        const DeviceSession *key = s.data();
        s->worker()->call(
            this, [](FfiCore *c) { return object(readSized([c](char *b, uint32_t n) { return dspi_apply_poll(c, b, n); })); },
            [this, key, s](const QJsonObject &o) {
                m_driving.remove(key);
                if (!s || !m_driven.contains(s) || !isOk(o)) return;
                const QString phase = o.value(QLatin1String("phase")).toString();
                if (phase == QLatin1String("waiting") || phase == QLatin1String("lost")) return;
                // It ended there: its words, with the device's name
                m_driven.removeAll(s);
                if (m_driven.isEmpty()) m_driveTimer.stop();
                emit guardChanged();
                auto *notices = Registrar::instance<NoticeCenter>();
                const QString message = o.value(QLatin1String("message")).toString();
                if (!notices || message.isEmpty() || phase == QLatin1String("idle")) return;
                const QString kind = phase == QLatin1String("kept") ? QStringLiteral("info")
                                   : phase == QLatin1String("reverted") ? QStringLiteral("warning")
                                                                       : QStringLiteral("error");
                //: %1 is a device's name, %2 how its safe apply ended
                notices->post(tr("%1: %2").arg(s->title(), message), kind);
            });
    }
    if (m_driven.isEmpty()) m_driveTimer.stop();
}

QStringList SnapshotsController::guardsElsewhere() const {
    QStringList out;
    for (const QPointer<DeviceSession> &s : m_driven)
        if (s && s != m_ctx.activeSession()) out << s->title();
    return out;
}

bool SnapshotsController::guardWaitingOn(const DeviceSession *session) const {
    if (!session) return false;
    if (session == m_ctx.activeSession()) return guardWaiting() || guardPhase() == QLatin1String("lost");
    for (const QPointer<DeviceSession> &s : m_driven)
        if (s == session) return true;
    return false;
}

// ---- The store -----------------------------------------------------------------------

void SnapshotsController::storeCall(const QString &op, std::function<QByteArray(FfiCore *)> job,
                                    std::function<void(const QJsonObject &)> done) {
    track(1);
    const QPointer<DeviceSession> asked = m_ctx.activeSession();   // -- WP MDQT2b --
    m_ctx.core->worker()->call(this, std::move(job), [this, op, done, asked](QByteArray reply) {
        track(-1);
        if (asked != m_ctx.activeSession()) return;   // -- WP MDQT2b -- the store of the device left
        const QJsonObject o = object(reply);
        if (!isOk(o)) failed(o, tr("The snapshot could not be stored."));
        else if (done) done(o);
        if (op != QLatin1String("list")) {
            emit stored(op, isOk(o));
            reload();
        }
    });
}

// -- WP P2QT -- both sides on a hub: this console's list and the merged one
namespace {
struct Lists {
    QByteArray local;
    QByteArray merged;
};
} // namespace

void SnapshotsController::reload() {
    if (!m_ctx.device->connected()) {
        if (!m_snapshots.isEmpty()) {
            m_snapshots.clear();
            emit snapshotsChanged();
        }
        if (m_hubAvailable || !m_hubSnapshots.isEmpty() || !m_hubProblem.isEmpty()) {
            m_hubAvailable = false;
            m_hubSnapshots.clear();
            m_hubProblem.clear();
            emit hubChanged();
        }
        return;
    }
    track(1);
    const int n = ++m_reloads;
    const bool remote = m_ctx.device->remote();
    m_ctx.core->worker()->call(
        this,
        [remote](FfiCore *c) {
            Lists l;
            l.local = readSized([c](char *b, uint32_t n) { return dspi_snapshot_list(c, nullptr, b, n); });
            // Off a hub the merged list is this console's alone: not asked
            if (remote) l.merged = readSized([c](char *b, uint32_t n) { return dspi_snapshot_list_merged(c, nullptr, b, n); }, 16384);
            return l;
        },
        [this, n](const Lists &l) {
            track(-1);
            if (n != m_reloads) return;   // a newer read follows (-- WP MDQT2b -- a switch reads again too)
            // The merged list's word on each id: where it is, and the local copies
            const QJsonObject merged = object(l.merged);
            QHash<QString, QString> where;
            QSet<QString> localCopies;
            QVariantList hubRows;
            const QJsonArray entries = merged.value(QLatin1String("entries")).toArray();
            for (qsizetype i = entries.size() - 1; i >= 0; --i) {
                const QJsonObject e = entries.at(i).toObject();
                const QJsonObject snap = e.value(QLatin1String("snapshot")).toObject();
                const QString id = snap.value(QLatin1String("id")).toString();
                const QString w = e.value(QLatin1String("where")).toString();
                if (e.value(QLatin1String("localCopy")).toBool()) {
                    localCopies.insert(id);
                    continue;
                }
                where.insert(id, w);
                if (w == QLatin1String("hub") || w == QLatin1String("both")) {
                    QVariantMap r = row(snap);
                    r.insert(QStringLiteral("where"), w);
                    hubRows.append(r);
                }
            }
            QVariantList rows;
            const QJsonArray list = object(l.local).value(QLatin1String("value")).toArray();
            // Newest first, as the History window lists its changes
            for (qsizetype i = list.size() - 1; i >= 0; --i) {
                QVariantMap r = row(list.at(i).toObject());
                const QString id = r.value(QStringLiteral("id")).toString();
                const bool copy = localCopies.contains(id);
                r.insert(QStringLiteral("localCopy"), copy);
                r.insert(QStringLiteral("where"), !copy && where.value(id) == QLatin1String("both") ? QStringLiteral("both")
                                                                                                    : QStringLiteral("local"));
                rows.append(r);
            }
            m_snapshots = rows;
            emit snapshotsChanged();
            const bool listed = merged.value(QLatin1String("hubListed")).toBool();
            const QString key = merged.value(QLatin1String("hubNoticeKey")).toString();
            auto *notices = Registrar::instance<NoticeCenter>();
            m_hubAvailable = listed;
            m_hubSnapshots = listed ? hubRows : QVariantList();
            m_hubProblem = !listed && !key.isEmpty() && notices ? notices->line(key, key) : QString();
            emit hubChanged();
        });
}

void SnapshotsController::capture(const QString &name, const QString &note) {
    if (!m_ctx.device->connected()) {
        failed({}, tr("No device connected."));
        return;
    }
    const QString trimmed = name.trimmed();
    const QByteArray n = (trimmed.isEmpty()
                              ? tr("Snapshot %1").arg(QDateTime::currentDateTime().toString(QStringLiteral("yyyy-MM-dd HH:mm")))
                              : trimmed)
                             .toUtf8();
    const QByteArray t = note.toUtf8();
    storeCall(QStringLiteral("capture"), [n, t](FfiCore *c) {
        return readSized([&](char *b, uint32_t len) { return dspi_snapshot_capture(c, n.constData(), t.constData(), b, len); });
    });
}

void SnapshotsController::rename(const QString &id, const QString &name) {
    if (name.trimmed().isEmpty()) return;
    const QByteArray i = id.toUtf8(), n = name.trimmed().toUtf8();
    storeCall(QStringLiteral("rename"), [i, n](FfiCore *c) {
        return readSized([&](char *b, uint32_t len) { return dspi_snapshot_rename(c, i.constData(), n.constData(), b, len); });
    });
}

void SnapshotsController::setNote(const QString &id, const QString &note) {
    const QByteArray i = id.toUtf8(), t = note.toUtf8();
    storeCall(QStringLiteral("note"), [i, t](FfiCore *c) {
        return readSized([&](char *b, uint32_t len) { return dspi_snapshot_set_note(c, i.constData(), t.constData(), b, len); });
    });
}

void SnapshotsController::remove(const QString &id) {
    const QByteArray i = id.toUtf8();
    storeCall(QStringLiteral("delete"), [i](FfiCore *c) {
        return readSized([&](char *b, uint32_t len) { return dspi_snapshot_delete(c, i.constData(), b, len); });
    });
}

void SnapshotsController::exportSnapshot(const QString &id) {
    QString name;
    for (const QVariant &v : m_snapshots)
        if (v.toMap().value(QStringLiteral("id")).toString() == id) name = v.toMap().value(QStringLiteral("name")).toString();
    // A name that is no file name keeps only its safe characters
    name.replace(QRegularExpression(QStringLiteral(R"([\\/:*?"<>|])")), QStringLiteral("-"));
    FilesController::pickFile(true, tr("Export Snapshot"), tr("DSPi configurations (*.dspipreset)"),
                              (name.isEmpty() ? tr("Snapshot") : name) + QStringLiteral(".dspipreset"), this,
                              [this, id](const QString &path) { exportTo(id, path); });
}

void SnapshotsController::exportTo(const QString &id, const QString &path) {
    const QByteArray i = id.toUtf8();
    storeCall(QStringLiteral("export"), [i, path](FfiCore *c) {
        const QByteArray doc = readSized([&](char *b, uint32_t len) { return dspi_snapshot_export(c, i.constData(), b, len); });
        // The document, or a JSON failure in its place (a document has no `ok`)
        const QJsonObject o = object(doc);
        if (doc.isEmpty() || o.contains(QLatin1String("ok"))) return doc.isEmpty() ? QByteArray(R"({"ok":false})") : doc;
        QSaveFile f(path);
        if (!f.open(QIODevice::WriteOnly) || f.write(doc) != doc.size() || !f.commit()) {
            const QString error = SnapshotsController::tr("Failed to write file: %1").arg(f.errorString());
            return QJsonDocument(QJsonObject{ { QStringLiteral("ok"), false }, { QStringLiteral("error"), error } }).toJson();
        }
        return QByteArray(R"({"ok":true})");
    },
              [path](const QJsonObject &) {
                  if (auto *notices = Registrar::instance<NoticeCenter>())
                      notices->post(SnapshotsController::tr("Snapshot exported to %1.").arg(QFileInfo(path).fileName()));
              });
}

void SnapshotsController::importSnapshot() {
    if (!m_ctx.device->connected()) {
        failed({}, tr("No device connected."));
        return;
    }
    FilesController::pickFile(false, tr("Import Snapshot"), tr("DSPi configurations (*.dspipreset *.json);;All files (*)"),
                              QString(), this, [this](const QString &path) { importFrom(path); });
}

void SnapshotsController::importFrom(const QString &path) {
    storeCall(QStringLiteral("import"), [path](FfiCore *c) {
        QFile f(path);
        if (!f.open(QIODevice::ReadOnly)) {
            const QString error = SnapshotsController::tr("Failed to read file: %1").arg(f.errorString());
            return QJsonDocument(QJsonObject{ { QStringLiteral("ok"), false }, { QStringLiteral("error"), error } }).toJson();
        }
        const QByteArray doc = f.read(kReadLimit);
        return readSized([&](char *b, uint32_t len) { return dspi_snapshot_import(c, doc.constData(), nullptr, nullptr, b, len); });
    });
}

// ---- Writes ------------------------------------------------------------------------------

void SnapshotsController::writeCall(const QString &what, std::function<QByteArray(FfiCore *)> job,
                                    std::function<void(bool, const QJsonObject &)> done) {
    auto reply = std::make_shared<QJsonObject>();
    track(1);
    const QPointer<DeviceSession> asked = m_ctx.activeSession();   // -- WP MDQT2b --
    // Each its own key: two toggles in a row are two, not the newest one
    m_ctx.device->write(
        QStringLiteral("%1 %2").arg(what).arg(++m_writes),
        [job, reply](FfiCore *c) {
            *reply = object(job(c));
            return isOk(*reply);
        },
        [this, reply, done, asked](bool ok) {
            track(-1);
            // -- WP MDQT2b -- a write of the device left: its compare and
            // guard are not this one's (a guard it began is driven there)
            if (asked != m_ctx.activeSession()) {
                const QJsonObject guard = reply->contains(QLatin1String("guardState"))
                                              ? reply->value(QLatin1String("guardState")).toObject()
                                              : *reply;
                if (asked && guard.value(QLatin1String("phase")).toString() == QLatin1String("waiting")) drive(asked);
                return;
            }
            done(ok, *reply);
        });
}

void SnapshotsController::recall(const QString &id) {
    if (guardWaiting()) {
        failed({}, tr("Finish the trial first: Keep or Revert.") /* -- WP UFW -- (UXP D10) */);
        return;
    }
    const QByteArray i = id.toUtf8();
    if (m_guardDefault || m_rollback) {
        writeCall(QStringLiteral("snapshot recall"),
                  [i, t = uint32_t(m_timeoutS)](FfiCore *c) {
                      return once([&](char *b, uint32_t n) { return dspi_apply_guarded_snapshot(c, i.constData(), t, b, n); });
                  },
                  [this](bool ok, const QJsonObject &o) {
                      if (ok) setGuard(o);
                      else failed(o, tr("The snapshot was not recalled."));
                      reloadAb();
                  });
        return;
    }
    writeCall(QStringLiteral("snapshot recall"),
              [i](FfiCore *c) { return once([&](char *b, uint32_t n) { return dspi_snapshot_recall(c, i.constData(), b, n); }); },
              [this](bool ok, const QJsonObject &o) {
                  reloadAb();
                  if (!ok) {
                      failed(o, tr("The snapshot was not recalled."));
                      return;
                  }
                  auto *notices = Registrar::instance<NoticeCenter>();
                  if (!notices) return;
                  QStringList notes;
                  for (const QJsonValue &v : o.value(QLatin1String("notes")).toArray()) notes.append(v.toString());
                  //: %1 is the snapshot's name
                  const QString text = tr("Recalled “%1”.").arg(o.value(QLatin1String("name")).toString());
                  if (notes.isEmpty()) notices->post(text);
                  else notices->post(text + QLatin1Char(' ') + notes.join(QLatin1Char(' ')), QStringLiteral("warning"));
              });
}

// ---- A/B -------------------------------------------------------------------------------------

void SnapshotsController::setAb(const QJsonObject &o) {
    const bool active = o.value(QLatin1String("active")).toBool();
    const QString side = o.value(QLatin1String("side")).toString();
    const QString a = o.value(QLatin1String("aName")).toString();
    const QString b = o.value(QLatin1String("bName")).toString();
    const bool pairChanged = a != m_abAName || b != m_abBName || active != m_abActive;
    m_abActive = active;
    m_abSide = active ? side : QString();
    m_abAName = active ? a : QString();
    m_abBName = active ? b : QString();
    if (!active) m_abRows.clear();
    emit abChanged();
    if (!active || !pairChanged) return;
    const QPointer<DeviceSession> asked = m_ctx.activeSession();   // -- WP MDQT2b --
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return readSized([c](char *b, uint32_t n) { return dspi_ab_diff(c, b, n); }); },
        [this, asked](QByteArray reply) {
            if (asked != m_ctx.activeSession()) return;
            const QJsonObject d = object(reply);
            QVariantList rows;
            if (isOk(d))
                for (const QJsonValue &v : d.value(QLatin1String("rows")).toArray()) rows.append(v.toObject().toVariantMap());
            m_abRows = rows;
            emit abChanged();
        });
}

void SnapshotsController::reloadAb() {
    const QPointer<DeviceSession> asked = m_ctx.activeSession();   // -- WP MDQT2b --
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return readSized([c](char *b, uint32_t n) { return dspi_ab_state(c, b, n); }); },
        [this, asked](QByteArray reply) {
            if (asked == m_ctx.activeSession()) setAb(object(reply));
        });
}

void SnapshotsController::abStart(const QString &aId, const QString &bId) {
    if (bId.isEmpty() || aId == bId) return;
    if (guardWaiting()) {
        failed({}, tr("Finish the trial first: Keep or Revert.") /* -- WP UFW -- (UXP D10) */);
        return;
    }
    const QByteArray a = aId.toUtf8(), b = bId.toUtf8();
    writeCall(QStringLiteral("ab start"),
              [a, b](FfiCore *c) { return once([&](char *buf, uint32_t n) { return dspi_ab_start(c, orNull(a), b.constData(), buf, n); }); },
              [this](bool ok, const QJsonObject &o) {
                  if (ok) setAb(o);
                  else failed(o, tr("The compare did not start."));
                  reload();   // A captured from the device is a snapshot of the store
              });
}

void SnapshotsController::abToggle() {
    if (!canToggle()) return;
    ++m_toggling;
    emit abChanged();
    writeCall(QStringLiteral("ab toggle"),
              [](FfiCore *c) { return once([&](char *b, uint32_t n) { return dspi_ab_toggle(c, b, n); }); },
              [this](bool ok, const QJsonObject &o) {
                  --m_toggling;
                  if (ok) setAb(o);
                  else {
                      failed(o, tr("The other side was not recalled."));
                      emit abChanged();
                  }
              });
}

void SnapshotsController::abKeep(const QString &side) {
    if (!m_abActive) return;
    const QByteArray s = side.toUtf8();
    writeCall(QStringLiteral("ab keep"),
              [s](FfiCore *c) { return once([&](char *b, uint32_t n) { return dspi_ab_keep(c, s.constData(), b, n); }); },
              [this](bool ok, const QJsonObject &o) {
                  if (ok) setAb(o);
                  else failed(o, tr("The compare did not end."));
              });
}

// ---- The safe apply ---------------------------------------------------------------------------

void SnapshotsController::guardStarted(const QJsonObject &state, DeviceSession *on) {
    // -- WP MDQT2b -- begun on a device that is not the active one any more
    if (on && on != m_ctx.activeSession()) {
        drive(on);
        return;
    }
    setGuard(state);
}

void SnapshotsController::setGuard(const QJsonObject &state) {
    const QString before = guardPhase();
    const quint64 beforeId = quint64(m_guard.value(QLatin1String("id")).toDouble());
    m_guard = state;
    const QString now = guardPhase();
    const quint64 id = quint64(state.value(QLatin1String("id")).toDouble());
    m_ctx.poller->setEnabled(m_poll, now == QLatin1String("waiting"));
    emit guardChanged();
    emit abChanged();   // canToggle follows the guard
    // It ended now: the one that waited, or a new one that ended at once (an
    // import that failed and was put back)
    // -- WP REVERTW -- a lost guard whose end the hub no longer knows has ended
    const bool open = now == QLatin1String("waiting") || (now == QLatin1String("lost") && guardRevertNotice().isEmpty())
                      || now == QLatin1String("idle");
    const bool wasOpen = before == QLatin1String("waiting") || before == QLatin1String("lost");
    const bool ended = !open && (id != beforeId || (wasOpen && now != before));
    if (!ended) return;
    // The device may hold other settings now: read it, and the compare the
    // core ends with a revert
    m_ctx.device->refresh();
    reloadAb();
    const QString message = guardMessage();
    auto *notices = Registrar::instance<NoticeCenter>();
    // -- WP REVERTW -- not fully put back: the notice's words, an error
    if (notices && !guardRevertNotice().isEmpty()) {
        notices->post(guardRevertText(), QStringLiteral("error"));
        return;
    }
    if (!notices || message.isEmpty()) return;
    const QString kind = now == QLatin1String("kept") ? QStringLiteral("info")
                       : now == QLatin1String("reverted") ? QStringLiteral("warning")
                                                          : QStringLiteral("error");
    notices->post(message, kind);
}

// -- WP REVERTW --
QString SnapshotsController::guardRevertText() const {
    const QString key = guardRevertNotice();
    if (key.isEmpty()) return {};
    QStringList what{ guardMessage() };
    for (const QJsonValue &n : m_guard.value(QLatin1String("notes")).toArray()) what << n.toString();
    what.removeAll(QString());
    auto *notices = Registrar::instance<NoticeCenter>();
    const QString detail = what.join(QLatin1Char(' '));
    if (notices) return notices->revertText(key, detail);
    return tr("The revert was not fully successful. %1 Check the settings, or restore a backup (a snapshot).").arg(detail);
}

void SnapshotsController::reloadGuard() {
    const QPointer<DeviceSession> asked = m_ctx.activeSession();   // -- WP MDQT2b --
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return readSized([c](char *b, uint32_t n) { return dspi_apply_poll(c, b, n); }); },
        [this, asked](QByteArray reply) {
            if (asked != m_ctx.activeSession()) return;
            const QJsonObject o = object(reply);
            if (isOk(o)) setGuard(o);
        });
}

void SnapshotsController::keep() {
    if (guardPhase() != QLatin1String("waiting") && guardPhase() != QLatin1String("lost")) return;
    track(1);
    const QPointer<DeviceSession> asked = m_ctx.activeSession();   // -- WP MDQT2b --
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return object(once([&](char *b, uint32_t n) { return dspi_apply_confirm(c, b, n); })); },
        [this, asked](QJsonObject o) {
            track(-1);
            if (asked != m_ctx.activeSession()) return;
            if (isOk(o)) setGuard(o);
            else failed(o, tr("Nothing waits for Keep."));
        });
}

void SnapshotsController::revert() {
    track(1);
    const QPointer<DeviceSession> asked = m_ctx.activeSession();   // -- WP MDQT2b --
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return object(once([&](char *b, uint32_t n) { return dspi_apply_revert(c, b, n); })); },
        [this, asked](QJsonObject o) {
            track(-1);
            if (asked != m_ctx.activeSession()) return;
            if (isOk(o)) setGuard(o);
            else failed(o, tr("There is nothing to revert."));
        });
}

// ---- -- WP P2QT -- The hub's store ---------------------------------------------------------

void SnapshotsController::onHubEvent(const QString &kind, const QJsonObject &event) {
    if (kind != QLatin1String("snapshot.changed") || !m_ctx.device->connected()) return;
    const QString serial = event.value(QLatin1String("serial")).toString();
    if (serial.isEmpty() || serial == m_ctx.device->serial()) reload();
}

void SnapshotsController::saveToHub(const QString &id) {
    const QByteArray i = id.toUtf8();
    storeCall(QStringLiteral("hub-save"), [i](FfiCore *c) {
        return readSized([&](char *b, uint32_t len) { return dspi_hub_snapshots_save(c, i.constData(), b, len); });
    },
              [](const QJsonObject &o) {
                  auto *notices = Registrar::instance<NoticeCenter>();
                  const QString name = o.value(QLatin1String("snapshot")).toObject().value(QLatin1String("name")).toString();
                  //: %1 is the snapshot's name
                  if (notices) notices->post(SnapshotsController::tr("Saved “%1” to the hub.").arg(name));
              });
}

void SnapshotsController::copyHere(const QString &id) {
    const QByteArray i = id.toUtf8();
    storeCall(QStringLiteral("hub-copy"), [i](FfiCore *c) {
        return readSized([&](char *b, uint32_t len) { return dspi_hub_snapshots_copy_here(c, i.constData(), b, len); });
    },
              [](const QJsonObject &o) {
                  auto *notices = Registrar::instance<NoticeCenter>();
                  //: %1 is the snapshot's name
                  if (notices) notices->post(SnapshotsController::tr("Copied “%1” to this console.").arg(o.value(QLatin1String("name")).toString()));
              });
}

void SnapshotsController::removeFromHub(const QString &id) {
    const QByteArray i = id.toUtf8();
    storeCall(QStringLiteral("hub-delete"), [i](FfiCore *c) {
        return readSized([&](char *b, uint32_t len) { return dspi_hub_snapshots_delete(c, i.constData(), b, len); });
    });
}

void SnapshotsController::recallFromHub(const QString &id) {
    if (guardWaiting()) {
        failed({}, tr("Finish the trial first: Keep or Revert.") /* -- WP UFW -- (UXP D10) */);
        return;
    }
    QString name;
    for (const QVariant &v : m_hubSnapshots)
        if (v.toMap().value(QStringLiteral("id")).toString() == id) name = v.toMap().value(QStringLiteral("name")).toString();
    const QByteArray i = id.toUtf8();
    const QByteArray label = (name.isEmpty() ? id : name).toUtf8();
    const bool guarded = m_guardDefault || m_rollback;
    writeCall(QStringLiteral("hub snapshot recall"),
              [i, label, guarded, t = uint32_t(m_timeoutS)](FfiCore *c) {
                  // The guard first (the hub's own on a hub with it), so Keep
                  // and Revert cover the hub's recall like any other
                  QJsonObject begun;
                  if (guarded) {
                      begun = object(once([&](char *b, uint32_t n) { return dspi_apply_guard_begin(c, label.constData(), t, b, n); }));
                      if (!isOk(begun)) return QJsonDocument(begun).toJson(QJsonDocument::Compact);
                  }
                  QJsonObject r = object(once([&](char *b, uint32_t n) {
                      return dspi_hub_snapshots_recall(c, nullptr, i.constData(), false, 0, b, n);
                  }));
                  if (guarded && !isOk(r)) {
                      // Nothing to keep: the settings from before, now
                      r.insert(QStringLiteral("guardState"),
                               object(once([&](char *b, uint32_t n) { return dspi_apply_revert(c, b, n); })));
                  } else if (guarded) {
                      r.insert(QStringLiteral("guardState"), begun);
                  }
                  return QJsonDocument(r).toJson(QJsonDocument::Compact);
              },
              [this](bool ok, const QJsonObject &o) {
                  const QJsonObject state = o.value(QLatin1String("guardState")).toObject();
                  if (!ok) failed(o, tr("The snapshot was not recalled."));
                  if (isOk(state)) setGuard(state);
                  reloadAb();
                  emit stored(QStringLiteral("hub-recall"), ok);
                  if (!ok || !state.isEmpty()) return;
                  auto *notices = Registrar::instance<NoticeCenter>();
                  if (!notices) return;
                  QStringList notes;
                  for (const QJsonValue &v : o.value(QLatin1String("notes")).toArray()) notes.append(v.toString());
                  //: %1 is the snapshot's name
                  const QString text = tr("Recalled “%1” from the hub.").arg(o.value(QLatin1String("name")).toString());
                  if (notes.isEmpty()) notices->post(text);
                  else notices->post(text + QLatin1Char(' ') + notes.join(QLatin1Char(' ')), QStringLiteral("warning"));
              });
}

// ---- -- WP AIVIS -- Beside an AI agent ----------------------------------------------------------

void SnapshotsController::agentCalled(const QString &device, const QString &tool, bool write) {
    if (!write) return;
    DeviceSession *on = m_ctx.registry->session(device);
    if (!on) return;
    if (on == m_ctx.activeSession()) {
        static const QStringList filing{ QStringLiteral("snapshot_"), QStringLiteral("preset_"), QStringLiteral("_import"), QStringLiteral("autoeq_apply"),
                                         QStringLiteral("tune_"), QStringLiteral("revert_to_saved"), QStringLiteral("ab_") };
        const bool files = std::any_of(filing.cbegin(), filing.cend(), [&](const QString &f) { return tool.contains(f); });
        if (files) {
            reload();
            reloadAb();
        }
        return;   // the guard is read with every write (AiController)
    }
    // A preview it began there is marked and its end is said, as the user's own is
    static const QStringList guards{ QStringLiteral("dspi_apply_changes"), QStringLiteral("dspi_tune_apply"), QStringLiteral("dspi_autoeq_apply"),
                                     QStringLiteral("dspi_config_import"), QStringLiteral("dspi_snapshot_recall"), QStringLiteral("dspi_filters_import") };
    if (!guards.contains(tool)) return;
    on->worker()->call(
        this, [](FfiCore *c) { return object(readSized([c](char *b, uint32_t n) { return dspi_apply_status(c, b, n); })); },
        [this, session = QPointer<DeviceSession>(on)](const QJsonObject &o) {
            const QString phase = o.value(QLatin1String("phase")).toString();
            if (session && isOk(o) && phase == QLatin1String("waiting")) drive(session);
        });
}
