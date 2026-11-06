#ifndef QTCOMPAT_H
#define QTCOMPAT_H

// The few spellings that differ between Qt 5.15 and Qt 6 (the app builds
// against either)

#include <QtGlobal>
#include <QMouseEvent>
#include <QHoverEvent>
#include <QEnterEvent>

// An event's position in the window (scene)
template <class E> inline QPointF eventScenePos(const E *e) {
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
    return e->scenePosition();
#else
    return e->windowPos();
#endif
}

// An event's position in the receiving item
inline QPointF eventLocalPos(const QMouseEvent *e) {
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
    return e->position();
#else
    return e->localPos();
#endif
}
inline QPointF eventLocalPos(const QHoverEvent *e) {
#if QT_VERSION >= QT_VERSION_CHECK(6, 0, 0)
    return e->position();
#else
    return e->posF();
#endif
}

#endif // QTCOMPAT_H
