#ifndef NOTICECENTER_H
#define NOTICECENTER_H

#include <QObject>
#include <QString>
#include <QTimer>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// -- WP QP18 -- Notices that need no answer (PRO1 U5): a finished export, a
// saved volume, an undo that left someone else's change alone. They show in
// the banner stack (NoticeBanner), never as a modal alert, and an "info" one
// goes by itself after a while; "warning" and "error" stay until dismissed.
// A question that needs an answer is still a QuestionDialog.
//
//     NoticeCenter.post(qsTr("Filters exported"))                 // QML
//     Registrar::instance<NoticeCenter>()->post(text, "warning"); // C++
class NoticeCenter : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // The notice on show: { text, kind, serial }; empty with none
    Q_PROPERTY(QVariantMap current READ current NOTIFY changed)
    // Notices waiting behind it
    Q_PROPERTY(int waiting READ waiting NOTIFY changed)
    // Counts every notice posted (tests and the smoke driver)
    Q_PROPERTY(int serial READ serial NOTIFY changed)
public:
    // How long an "info" notice stays
    static constexpr int kInfoMs = 8000;
    // Older notices beyond this many are dropped
    static constexpr int kMaxWaiting = 5;

    NoticeCenter(const AppContext &ctx, QObject *parent = nullptr);
    static NoticeCenter *create(QQmlEngine *, QJSEngine *);

    QVariantMap current() const { return m_current; }
    int waiting() const { return int(m_queue.size()); }
    int serial() const { return m_serial; }
    // The last notice posted, shown or not (tests)
    QVariantMap last() const { return m_last; }

    // kind: "info", "warning" or "error"
    Q_INVOKABLE void post(const QString &text, const QString &kind = QStringLiteral("info"));
    Q_INVOKABLE void dismiss();
    Q_INVOKABLE void clear();

signals:
    void changed();

private:
    void showNext();

    QVariantMap m_current;
    QVariantMap m_last;
    QVariantList m_queue;
    QTimer m_timer;
    int m_serial = 0;
};

#endif // NOTICECENTER_H
