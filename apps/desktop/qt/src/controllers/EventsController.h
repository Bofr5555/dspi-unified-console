#ifndef EVENTSCONTROLLER_H
#define EVENTSCONTROLLER_H

#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>

#include "Poller.h"
#include "Registrar.h"

class DeviceSession;
class QJSEngine;
class QQmlEngine;

// -- WP P2QT -- Device > Events…: what happened to the open DSPi, from two
// places (PLAN-P2 M1):
// - "On this console": the core's own event log (dspi_event_log_*, one file
//   per serial under the app's data folder, read newest first);
// - "On the hub": the hub's shared history over DSPi Link (P2CC1:
//   dspi_hub_events_read, then dspi_hub_events_watch and `event.added`
//   through HubFeed), with who caused each record by name (`by`). Offered
//   only on a hub with the `events` cap; an upstream hub without it shows
//   the local tab alone.
// The window says when it shows (setActive): the local log is read again
// every few seconds then, and the hub's watch is on only then.
class EventsController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // "local" or "hub"
    Q_PROPERTY(QString source READ source WRITE setSource NOTIFY sourceChanged)
    // The open device is on a hub that keeps the shared history
    Q_PROPERTY(bool hubAvailable READ hubAvailable NOTIFY hubAvailableChanged)
    // Newest first: [{ id, time, day, text, by, detail, count }]
    Q_PROPERTY(QVariantList entries READ entries NOTIFY entriesChanged)
    // The hub's list could not be read: why ("" when it was, or locally)
    Q_PROPERTY(QString problem READ problem NOTIFY entriesChanged)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)
    // The window shows (it sets this)
    Q_PROPERTY(bool active READ active WRITE setActive NOTIFY activeChanged)
public:
    // How many records the window lists
    static constexpr int kShown = 200;
    // How often the local log is read while the window shows
    static constexpr int kLocalPollMs = 2000;

    EventsController(const AppContext &ctx, QObject *parent = nullptr);
    static EventsController *create(QQmlEngine *, QJSEngine *);

    QString source() const { return m_source; }
    void setSource(const QString &source);
    bool hubAvailable() const { return m_hubAvailable; }
    QVariantList entries() const { return m_entries; }
    QString problem() const { return m_problem; }
    bool busy() const { return m_busy > 0; }
    bool active() const { return m_active; }
    void setActive(bool on);

    Q_INVOKABLE void reload();

    // A record (the local log's or the hub's) as a row (tests)
    static QVariantMap row(const QJsonObject &record);

signals:
    void sourceChanged();
    void hubAvailableChanged();
    void entriesChanged();
    void busyChanged();
    void activeChanged();
    // A read finished (the smoke driver)
    void loaded(const QString &source, bool ok);

private:
    void onSnapshot();
    void onHubEvent(const QString &kind, const QJsonObject &event);
    void readLocal();
    void applyLocal(const QJsonArray &records);
    void readHub();
    // The hub's watch: on while the hub's tab shows
    void updateWatch();
    void track(int delta);

    AppContext m_ctx;
    QString m_source = QStringLiteral("local");
    bool m_hubAvailable = false;
    QVariantList m_entries;
    QString m_problem;
    int m_busy = 0;
    bool m_active = false;
    bool m_watching = false;
    bool m_connected = false;
    bool m_remote = false;
    QString m_serial;
    quint64 m_newestHubId = 0;
    int m_generation = 0;
    Poller::Id m_poll = -1;
    QPointer<DeviceSession> m_session;   // -- WP MDQT2b -- the device the list and the hub watch are of
};

#endif // EVENTSCONTROLLER_H
