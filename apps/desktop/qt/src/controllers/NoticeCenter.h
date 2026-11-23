#ifndef NOTICECENTER_H
#define NOTICECENTER_H

#include <QHash>
#include <QJsonObject>
#include <QStringList>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QVariantMap>
#include <QTimer>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <functional>

#include "Registrar.h"

class DeviceSession;
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
    // The notice on show: { text, kind, serial }; empty with none. -- WP P2QT --
    // A hub alarm's adds { alarmId, canAck }
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

    // -- WP AIVIS -- An "info" notice with a button (`action`, "" for none; `run` is
    // called once, then the notice goes). One with the same tag on show is replaced
    // in place and its time starts again.
    void postTagged(const QString &tag, const QString &text, const QString &action, std::function<void()> run);
    // The tag of the notice on show, "" for none
    QString tag() const { return m_current.value(QStringLiteral("tag")).toString(); }
    Q_INVOKABLE void act();

    // -- WP P1QT2 -- The core's notice table (P1NT, `dspi_notice_texts`, read
    // once: no handle, no I/O). A row by key: { key, kind, title, reason,
    // action }, {} for a key the table lacks.
    Q_INVOKABLE QVariantMap row(const QString &key) const;
    // A row in one line, "Title: reason", or `fallback` when the table lacks it
    Q_INVOKABLE QString line(const QString &key, const QString &fallback = QString()) const;
    // A row's reason sentence alone, or `fallback`
    Q_INVOKABLE QString sentence(const QString &key, const QString &fallback = QString()) const;
    // Posts the row for `key` in the row's kind ("Title: reason"); `detail`
    // when the table lacks it. Keys of the same family fall back to the
    // family's row (`installer_lock.x` to `installer_lock`).
    Q_INVOKABLE void postKey(const QString &key, const QString &detail = QString());
    // -- WP REVERTW -- a revert that was not fully successful (the core's
    // `revert.*` rows): "Title. detail (or the reason). Action", with the
    // English words for a table without the row; "" for an empty key
    Q_INVOKABLE QString revertText(const QString &key, const QString &detail = QString()) const;
    // A write refusal worth a notice of its own: a limit, the installer's
    // lock, a missing confirmation, a hub that refused an image or could not
    // judge the write. Other refusals keep their own words where they are.
    static bool gateKey(const QString &key);
    Q_INVOKABLE void dismiss();
    Q_INVOKABLE void clear();

    // -- WP P2QT -- The hub's alarms (P2CC2, PLAN-P2 M3) over DSPi Link: each
    // one raised shows as a notice in its row's words (dspi_hub_alarm_notice,
    // the time in this console's clock), the active ones read when a hub
    // session starts and every `alarm.changed` after; one acknowledged or
    // cleared leaves the stack. Acknowledge is the hub's `alarm.ack` (Control,
    // Admin; a Viewer's notice offers none).
    Q_INVOKABLE void acknowledge(const QString &alarmId);
    // The alarm notices waiting or on show, by id (tests)
    QStringList alarmIds() const;

signals:
    void changed();
    // -- WP P2QT -- an acknowledge finished (the smoke driver)
    void acknowledged(const QString &alarmId, bool ok);

private:
    void showNext();

    QVariantMap m_current;
    QVariantMap m_last;
    QVariantList m_queue;
    QTimer m_timer;
    QHash<int, std::function<void()>> m_actions;   // -- WP AIVIS -- by serial
    int m_serial = 0;
    QHash<QString, QVariantMap> m_rows;   // -- WP P1QT2 -- the core's notice table
    // -- WP P2QT --
    void onHubEvent(const QString &kind, const QJsonObject &event);
    void onSnapshot();
    void showAlarm(const QJsonObject &alarm);
    void dropAlarm(const QString &id);
    AppContext m_ctx;
    bool m_hubSession = false;
    bool m_canAck = false;
    QPointer<DeviceSession> m_session;   // -- WP MDQT2b -- the device whose hub alarms show
};

#endif // NOTICECENTER_H
