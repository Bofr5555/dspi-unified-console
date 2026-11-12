#include "NoticeCenter.h"

#include <QJSEngine>

static const Registrar::Controller<NoticeCenter> noticeCenter;
static const Registrar::Banner noticeBanner("NoticeBanner", 45);

NoticeCenter::NoticeCenter(const AppContext &, QObject *parent) : QObject(parent) {
    m_timer.setSingleShot(true);
    connect(&m_timer, &QTimer::timeout, this, &NoticeCenter::dismiss);
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
