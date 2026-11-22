#include "NoticeCenter.h"

#include "DeviceModel.h"
#include "DeviceSession.h"   // -- WP MDQT2b --
#include "Snapshot.h"
// -- WP P2QT --
#include "Core.h"
#include "CoreWorker.h"
#include "HubFeed.h"

#include "CoreApi.h"

#include <QJSEngine>
#include <QPointer>
#include <QLocale>   // -- WP P2QT --
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QDateTime>   // -- WP P2QT --

static const Registrar::Controller<NoticeCenter> noticeCenter;
static const Registrar::Banner noticeBanner("NoticeBanner", 45);

NoticeCenter::NoticeCenter(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    m_timer.setSingleShot(true);
    connect(&m_timer, &QTimer::timeout, this, &NoticeCenter::dismiss);
    // -- WP P1QT2 -- the core's notice table (P1NT): a pure function of the
    // build, no handle and no I/O, so it is read here once
    const QJsonObject table = QJsonDocument::fromJson(readSized([](char *b, uint32_t n) { return dspi_notice_texts(b, n); }, 16384)).object();
    for (const QJsonValue &v : table.value(QLatin1String("value")).toArray()) {
        const QJsonObject r = v.toObject();
        const QString key = r.value(QLatin1String("key")).toString();
        if (!key.isEmpty()) m_rows.insert(key, r.toVariantMap());
    }
    // A write refused by a limit, the lock or a missing confirmation shows
    // the core's row for it (PLAN-CLIENTS 5.9: P1QT2)
    if (ctx.device)
        connect(ctx.device, &DeviceModel::writeRefused, this, [this](const QString &, const QString &key, const QString &message) {
            if (gateKey(key)) postKey(key, message);
        });
    // -- WP P2QT -- the hub's alarms
    if (ctx.device) connect(ctx.device, &DeviceModel::snapshotApplied, this, &NoticeCenter::onSnapshot);
    if (HubFeed *feed = HubFeed::of(ctx.core)) connect(feed, &HubFeed::hubEvent, this, &NoticeCenter::onHubEvent);
}

NoticeCenter *NoticeCenter::create(QQmlEngine *, QJSEngine *) {
    NoticeCenter *self = Registrar::instance<NoticeCenter>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

void NoticeCenter::post(const QString &text, const QString &kind) {
    if (text.trimmed().isEmpty()) return;
    // The same notice again while it shows only restarts its time
    if (!m_current.isEmpty() && m_current.value(QStringLiteral("text")).toString() == text) {
        if (m_timer.isActive()) m_timer.start();
        return;
    }
    const QString k = kind == QLatin1String("warning") || kind == QLatin1String("error") ? kind : QStringLiteral("info");
    m_last = { { QStringLiteral("text"), text }, { QStringLiteral("kind"), k }, { QStringLiteral("serial"), ++m_serial } };
    m_queue.append(m_last);
    while (m_queue.size() > kMaxWaiting) m_queue.removeFirst();
    if (m_current.isEmpty()) showNext();
    else emit changed();
}

void NoticeCenter::dismiss() {
    m_timer.stop();
    m_current.clear();
    showNext();
}

void NoticeCenter::clear() {
    m_timer.stop();
    m_queue.clear();
    m_current.clear();
    emit changed();
}

void NoticeCenter::showNext() {
    if (!m_queue.isEmpty()) {
        m_current = m_queue.takeFirst().toMap();
        if (m_current.value(QStringLiteral("kind")).toString() == QLatin1String("info")) m_timer.start(kInfoMs);
    }
    emit changed();
}

// -- WP P1QT2 --

QVariantMap NoticeCenter::row(const QString &key) const {
    if (m_rows.contains(key)) return m_rows.value(key);
    // `installer_lock.locked` without a row of its own: its family's
    const qsizetype dot = key.indexOf(QLatin1Char('.'));
    return dot > 0 ? m_rows.value(key.left(dot)) : QVariantMap();
}

QString NoticeCenter::line(const QString &key, const QString &fallback) const {
    const QVariantMap r = row(key);
    const QString title = r.value(QStringLiteral("title")).toString();
    const QString reason = r.value(QStringLiteral("reason")).toString();
    if (title.isEmpty()) return fallback;
    //: A notice from the core's table: %1 its title, %2 its reason
    return reason.isEmpty() ? title : tr("%1: %2").arg(title, reason);
}

QString NoticeCenter::sentence(const QString &key, const QString &fallback) const {
    const QString reason = row(key).value(QStringLiteral("reason")).toString();
    return reason.isEmpty() ? fallback : reason;
}

void NoticeCenter::postKey(const QString &key, const QString &detail) {
    const QVariantMap r = row(key);
    const QString kind = r.value(QStringLiteral("kind")).toString();
    post(line(key, detail), kind.isEmpty() ? QStringLiteral("error") : kind);
}

// -- WP REVERTW --
QString NoticeCenter::revertText(const QString &key, const QString &detail) const {
    if (key.isEmpty()) return {};
    const QVariantMap r = row(key);
    QString title = r.value(QStringLiteral("title")).toString();
    if (title.isEmpty()) title = tr("The revert was not fully successful");
    QString action = r.value(QStringLiteral("action")).toString();
    if (action.isEmpty()) action = tr("Check the settings, or restore a backup (a snapshot).");
    QString what = detail.trimmed();
    if (what.isEmpty()) what = r.value(QStringLiteral("reason")).toString();
    if (!what.isEmpty()) what[0] = what[0].toUpper();
    if (!what.isEmpty() && !what.endsWith(QLatin1Char('.'))) what += QLatin1Char('.');
    //: A revert that was not fully successful: %1 the title, %2 what was not put back, %3 what to do
    if (what.isEmpty()) return tr("%1. %2").arg(title, action);
    return tr("%1. %2 %3").arg(title, what, action);
}

bool NoticeCenter::gateKey(const QString &key) {
    for (const char *family : { "limit_exceeded", "installer_lock", "confirmation_required", "image_refused", "limits_unknown" }) {
        const QLatin1String f(family);
        if (key == f || (key.startsWith(f) && key.mid(f.size()).startsWith(QLatin1Char('.')))) return true;
    }
    return false;
}

// -- WP P2QT -- The hub's alarms ------------------------------------------------------

namespace {
// A notice's alarm id, "" for another notice
QString alarmOf(const QVariant &notice) { return notice.toMap().value(QStringLiteral("alarmId")).toString(); }
} // namespace

QStringList NoticeCenter::alarmIds() const {
    QStringList ids;
    if (!alarmOf(m_current).isEmpty()) ids << alarmOf(m_current);
    for (const QVariant &n : m_queue)
        if (!alarmOf(n).isEmpty()) ids << alarmOf(n);
    return ids;
}

// A hub session began or ended: its active alarms, or none
void NoticeCenter::onSnapshot() {
    if (!m_ctx.core) return;
    const bool hub = m_ctx.device->connected() && m_ctx.device->remote();
    m_canAck = hub && !m_ctx.device->readOnly();
    // -- WP MDQT2b -- another device is the active one: the alarms shown are
    // its hub's, read anew (another hub's are never acknowledged on this one)
    if (m_ctx.activeSession() != m_session) {
        m_session = m_ctx.activeSession();
        for (const QString &id : alarmIds()) dropAlarm(id);
        m_hubSession = false;
    }
    if (hub == m_hubSession) return;
    m_hubSession = hub;
    if (!hub) {
        for (const QString &id : alarmIds()) dropAlarm(id);
        return;
    }
    const QPointer<DeviceSession> asked = m_ctx.activeSession();   // -- WP MDQT2b --
    m_ctx.core->worker()->call(
        this,
        [](FfiCore *c) {
            // Unsupported (no `alarms` cap, an upstream hub) sends nothing: no notice
            return readSized([c](char *b, uint32_t n) { return dspi_hub_alarms(c, nullptr, "active", b, n); }, 16384);
        },
        [this, asked](const QByteArray &reply) {
            if (!m_hubSession || asked != m_ctx.activeSession()) return;
            const QJsonObject o = QJsonDocument::fromJson(reply).object();
            for (const QJsonValue &v : o.value(QLatin1String("value")).toArray()) showAlarm(v.toObject());
        });
}

void NoticeCenter::onHubEvent(const QString &kind, const QJsonObject &event) {
    if (kind != QLatin1String("alarm.changed") || !m_hubSession) return;
    showAlarm(event.value(QLatin1String("alarm")).toObject());
}

// Raised (again): shown once while it is on; acknowledged or cleared: gone
void NoticeCenter::showAlarm(const QJsonObject &alarm) {
    const QString id = alarm.value(QLatin1String("id")).toString();
    if (id.isEmpty()) return;
    if (alarm.value(QLatin1String("state")).toString() != QLatin1String("raised")) {
        dropAlarm(id);
        return;
    }
    if (alarmIds().contains(id)) return;
    // The time in this console's clock; the row's words from the core (no I/O)
    const QDateTime at = QDateTime::fromString(alarm.value(QLatin1String("raisedAt")).toString(), Qt::ISODate);
    const QByteArray time = at.isValid() ? QLocale().toString(at.toLocalTime().time(), QLocale::ShortFormat).toUtf8() : QByteArray();
    const QByteArray json = QJsonDocument(alarm).toJson(QJsonDocument::Compact);
    const QJsonObject n = QJsonDocument::fromJson(readSized([&](char *b, uint32_t len) {
                                                     return dspi_hub_alarm_notice(json.constData(), time.isEmpty() ? nullptr : time.constData(), b, len);
                                                 })).object();
    const QString title = n.value(QLatin1String("title")).toString();
    const QString reason = n.value(QLatin1String("reason")).toString();
    QString text = title.isEmpty() ? alarm.value(QLatin1String("message")).toString()
                 : reason.isEmpty() ? title
                                    : tr("%1: %2").arg(title, reason);
    if (text.trimmed().isEmpty()) return;
    const QString k = n.value(QLatin1String("kind")).toString();
    m_last = { { QStringLiteral("text"), text },
               { QStringLiteral("kind"), k == QLatin1String("error") || k == QLatin1String("info") ? k : QStringLiteral("warning") },
               { QStringLiteral("serial"), ++m_serial },
               { QStringLiteral("alarmId"), id },
               { QStringLiteral("canAck"), m_canAck } };
    m_queue.append(m_last);
    while (m_queue.size() > kMaxWaiting) m_queue.removeFirst();
    if (m_current.isEmpty()) showNext();
    else emit changed();
}

void NoticeCenter::dropAlarm(const QString &id) {
    bool dropped = false;
    for (qsizetype i = m_queue.size() - 1; i >= 0; --i)
        if (alarmOf(m_queue.at(i)) == id) {
            m_queue.removeAt(i);
            dropped = true;
        }
    if (alarmOf(m_current) == id) {
        dismiss();
        return;
    }
    if (dropped) emit changed();
}

void NoticeCenter::acknowledge(const QString &alarmId) {
    if (alarmId.isEmpty() || !m_ctx.core) return;
    const QByteArray id = alarmId.toUtf8();
    m_ctx.core->worker()->call(
        this, [id](FfiCore *c) { return readSized([&](char *b, uint32_t n) { return dspi_hub_alarm_ack(c, id.constData(), b, n); }); },
        [this, alarmId](const QByteArray &reply) {
            const QJsonObject o = QJsonDocument::fromJson(reply).object();
            const bool ok = o.value(QLatin1String("ok")).toBool();
            if (ok) {
                dropAlarm(alarmId);
            } else {
                const QString key = o.value(QLatin1String("noticeKey")).toString();
                const QString error = o.value(QLatin1String("error")).toString();
                post(!key.isEmpty() ? line(key, error) : error, QStringLiteral("error"));
            }
            emit acknowledged(alarmId, ok);
        });
}
