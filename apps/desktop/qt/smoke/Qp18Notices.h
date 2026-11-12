#ifndef QP18NOTICES_H
#define QP18NOTICES_H

#include "NoticeCenter.h"
#include "Registrar.h"

#include <QVariantMap>

// -- WP QP18 -- A success is a notice in the banner stack now, not an alert
// (NoticeCenter). The sections that waited for the alert also take the notice:
// qp18NoticeMark() at a section's start, then qp18NoticeNew()/qp18TakeNotice().
inline int &qp18NoticeSeen() {
    static int seen = 0;
    return seen;
}
inline void qp18NoticeMark() {
    if (NoticeCenter *c = Registrar::instance<NoticeCenter>()) qp18NoticeSeen() = c->serial();
}
inline bool qp18NoticeNew() {
    NoticeCenter *c = Registrar::instance<NoticeCenter>();
    return c && c->serial() > qp18NoticeSeen();
}
// The newest notice's text, and the notices cleared
inline QString qp18TakeNotice() {
    NoticeCenter *c = Registrar::instance<NoticeCenter>();
    if (!c) return {};
    qp18NoticeSeen() = c->serial();
    const QString text = c->last().value(QStringLiteral("text")).toString();
    c->clear();
    return text;
}

#endif // QP18NOTICES_H
