#include "PointerTracker.h"

#include <QQuickWindow>
#include <QMouseEvent>
#include "QtCompat.h"
#include <QHoverEvent>
#include <QGuiApplication>
#include <QTimer>

PointerTracker::PointerTracker(QQuickItem *parent) : QQuickItem(parent) {
    setFlag(ItemHasContents, false);
}

void PointerTracker::itemChange(ItemChange change, const ItemChangeData &value) {
    QQuickItem::itemChange(change, value);
    if (change != ItemSceneChange) return;
    if (m_window) m_window->removeEventFilter(this);
    m_window = value.window;
    if (m_window) m_window->installEventFilter(this);
    setInside(false);
}

bool PointerTracker::eventFilter(QObject *, QEvent *event) {
    switch (event->type()) {
    case QEvent::MouseMove:
    case QEvent::MouseButtonPress:
    case QEvent::MouseButtonRelease: {
        auto *me = static_cast<QMouseEvent *>(event);
        setInside(isVisible() && contains(mapFromScene(eventScenePos(me))));
        m_seen = m_inWindow = true;
        m_lastScene = eventScenePos(me);
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
        m_lastGlobal = me->globalPosition();
#else
        m_lastGlobal = me->screenPos();
#endif
        break;
    }
    case QEvent::Enter: {
        auto *ee = static_cast<QEnterEvent *>(event);
        setInside(isVisible() && contains(mapFromScene(eventScenePos(ee))));
        m_seen = m_inWindow = true;
        m_lastScene = eventScenePos(ee);
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
        m_lastGlobal = ee->globalPosition();
#else
        m_lastGlobal = ee->screenPos();
#endif
        break;
    }
    case QEvent::Leave:
        setInside(false);
        m_inWindow = false;
        if (!m_resendFor.hasExpired()) QTimer::singleShot(0, this, &PointerTracker::sendLastMove);
        break;
    default:
        break;
    }
    return false;              // only watching
}

void PointerTracker::resendPointer() {
    if (!m_window || !m_seen) return;
    m_resendFor.setRemainingTime(250);
    sendLastMove();
}

void PointerTracker::sendLastMove() {
    if (!m_window) return;
    QMouseEvent move(QEvent::MouseMove, m_lastScene, m_lastScene, m_lastGlobal, Qt::NoButton, Qt::NoButton,
                     QGuiApplication::keyboardModifiers());
    QCoreApplication::sendEvent(m_window, &move);
}

void PointerTracker::setInside(bool inside) {
    if (inside == m_inside) return;
    m_inside = inside;
    emit containsPointerChanged();
}
