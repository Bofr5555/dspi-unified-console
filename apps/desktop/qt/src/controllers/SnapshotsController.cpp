// -- WP P1QT1 -- Device > Snapshots…, the A/B compare and the safe apply's
// countdown bar (PLAN-CLIENTS 5.9: P1SN, P1AB, P1SA)
#include "SnapshotsController.h"

#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "FilesController.h"
#include "NoticeCenter.h"
#include "Shell.h"
#include "Snapshot.h"

#include "CoreApi.h"

#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QGuiApplication>
#include <QJSEngine>
#include <QJsonArray>
#include <QJsonDocument>
#include <QRegularExpression>
#include <QSaveFile>
#include <QSettings>
#include <QWindow>
#include <QStandardPaths>
#include <algorithm>
#include <memory>

static const Registrar::Controller<SnapshotsController> snapshotsController;
static const Registrar::Window snapshotsWindow("device.snapshots", "SnapshotsWindow");
static const Registrar::Banner safeApplyBanner("SafeApplyBanner", 5);

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
    const QString error = reply.value(QLatin1String("error")).toString();
    if (auto *notices = Registrar::instance<NoticeCenter>()) notices->post(error.isEmpty() ? fallback : error, QStringLiteral("error"));
}

// ---- The device changed ------------------------------------------------------------

void SnapshotsController::onSnapshotApplied() {
    const bool connected = m_ctx.device->connected();
    const QString serial = m_ctx.device->serial();
    const bool changed = connected != m_connected || serial != m_serial;
    m_connected = connected;
    m_serial = serial;
    if (changed) {
        reload();
        reloadAb();
    }
    // Over DSPi Link the default may change with the session; no I/O
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return dspi_apply_guard_default(c); },
        [this](bool on) {
            if (on == m_guardDefault) return;
            m_guardDefault = on;
            emit guardChanged();
        });
    // A lost apply: ask again once a device is back (the hub's word, or the revert offered)
    if (connected && guardPhase() == QLatin1String("lost")) reloadGuard();
}

// ---- The store -----------------------------------------------------------------------

void SnapshotsController::storeCall(const QString &op, std::function<QByteArray(FfiCore *)> job,
                                    std::function<void(const QJsonObject &)> done) {
    track(1);
    m_ctx.core->worker()->call(this, std::move(job), [this, op, done](QByteArray reply) {
        track(-1);
        const QJsonObject o = object(reply);
        if (!isOk(o)) failed(o, tr("The snapshot could not be stored."));
        else if (done) done(o);
        if (op != QLatin1String("list")) {
            emit stored(op, isOk(o));
            reload();
        }
    });
}

void SnapshotsController::reload() {
    if (!m_ctx.device->connected()) {
        if (!m_snapshots.isEmpty()) {
            m_snapshots.clear();
            emit snapshotsChanged();
        }
        return;
    }
    track(1);
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return readSized([c](char *b, uint32_t n) { return dspi_snapshot_list(c, nullptr, b, n); }); },
        [this](QByteArray reply) {
            track(-1);
            QVariantList rows;
            const QJsonArray list = object(reply).value(QLatin1String("value")).toArray();
            // Newest first, as the History window lists its changes
            for (qsizetype i = list.size() - 1; i >= 0; --i) rows.append(row(list.at(i).toObject()));
            m_snapshots = rows;
            emit snapshotsChanged();
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
    // Each its own key: two toggles in a row are two, not the newest one
    m_ctx.device->write(
        QStringLiteral("%1 %2").arg(what).arg(++m_writes),
        [job, reply](FfiCore *c) {
            *reply = object(job(c));
            return isOk(*reply);
        },
        [this, reply, done](bool ok) {
            track(-1);
            done(ok, *reply);
        });
}

void SnapshotsController::recall(const QString &id) {
    if (guardWaiting()) {
        failed({}, tr("Finish the safe apply first: Keep or Revert."));
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
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return readSized([c](char *b, uint32_t n) { return dspi_ab_diff(c, b, n); }); },
        [this](QByteArray reply) {
            const QJsonObject d = object(reply);
            QVariantList rows;
            if (isOk(d))
                for (const QJsonValue &v : d.value(QLatin1String("rows")).toArray()) rows.append(v.toObject().toVariantMap());
            m_abRows = rows;
            emit abChanged();
        });
}

void SnapshotsController::reloadAb() {
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return readSized([c](char *b, uint32_t n) { return dspi_ab_state(c, b, n); }); },
        [this](QByteArray reply) { setAb(object(reply)); });
}

void SnapshotsController::abStart(const QString &aId, const QString &bId) {
    if (bId.isEmpty() || aId == bId) return;
    if (guardWaiting()) {
        failed({}, tr("Finish the safe apply first: Keep or Revert."));
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

void SnapshotsController::guardStarted(const QJsonObject &state) { setGuard(state); }

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
    const bool open = now == QLatin1String("waiting") || now == QLatin1String("lost") || now == QLatin1String("idle");
    const bool wasOpen = before == QLatin1String("waiting") || before == QLatin1String("lost");
    const bool ended = !open && (id != beforeId || (wasOpen && now != before));
    if (!ended) return;
    // The device may hold other settings now: read it, and the compare the
    // core ends with a revert
    m_ctx.device->refresh();
    reloadAb();
    const QString message = guardMessage();
    auto *notices = Registrar::instance<NoticeCenter>();
    if (!notices || message.isEmpty()) return;
    const QString kind = now == QLatin1String("kept") ? QStringLiteral("info")
                       : now == QLatin1String("reverted") ? QStringLiteral("warning")
                                                          : QStringLiteral("error");
    notices->post(message, kind);
}

void SnapshotsController::reloadGuard() {
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return readSized([c](char *b, uint32_t n) { return dspi_apply_poll(c, b, n); }); },
        [this](QByteArray reply) {
            const QJsonObject o = object(reply);
            if (isOk(o)) setGuard(o);
        });
}

void SnapshotsController::keep() {
    if (guardPhase() != QLatin1String("waiting") && guardPhase() != QLatin1String("lost")) return;
    track(1);
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return object(once([&](char *b, uint32_t n) { return dspi_apply_confirm(c, b, n); })); },
        [this](QJsonObject o) {
            track(-1);
            if (isOk(o)) setGuard(o);
            else failed(o, tr("Nothing waits for Keep."));
        });
}

void SnapshotsController::revert() {
    track(1);
    m_ctx.core->worker()->call(
        this, [](FfiCore *c) { return object(once([&](char *b, uint32_t n) { return dspi_apply_revert(c, b, n); })); },
        [this](QJsonObject o) {
            track(-1);
            if (isOk(o)) setGuard(o);
            else failed(o, tr("There is nothing to revert."));
        });
}
