#include "NoticeCenter.h"

#include "DeviceModel.h"
#include "Snapshot.h"

#include "CoreApi.h"

#include <QJSEngine>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>

static const Registrar::Controller<NoticeCenter> noticeCenter;
static const Registrar::Banner noticeBanner("NoticeBanner", 45);

NoticeCenter::NoticeCenter(const AppContext &ctx, QObject *parent) : QObject(parent) {
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

bool NoticeCenter::gateKey(const QString &key) {
    for (const char *family : { "limit_exceeded", "installer_lock", "confirmation_required", "image_refused", "limits_unknown" }) {
        const QLatin1String f(family);
        if (key == f || (key.startsWith(f) && key.mid(f.size()).startsWith(QLatin1Char('.')))) return true;
    }
    return false;
}
