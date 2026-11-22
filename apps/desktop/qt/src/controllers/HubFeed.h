#ifndef HUBFEED_H
#define HUBFEED_H

#include <QJsonObject>
#include <QObject>
#include <QString>

class Core;
struct FfiCore;

// -- WP P2QT -- The hub's own events past DSPi Link (P2CC1, P2CC2:
// `event.added`, `inventory.changed`, `snapshot.changed`, `alarm.changed`,
// `schedule.ran`) for every controller that wants them. The core has one
// listener for these (dspi_hub_server_set_listener); this object holds it and
// hands each event out as a signal on the GUI thread, so the Snapshots window,
// the Events window and NoticeCenter each hear every event.
//
// One per Core, a child of it, made on first use (HubFeed::of): not a
// Registrar controller, so a controller's constructor may connect to it.
class HubFeed : public QObject {
    Q_OBJECT
public:
    // The feed of `core`, made the first time; null for a null core
    static HubFeed *of(Core *core);
    ~HubFeed() override;

signals:
    // `kind` the hub's message (`event.added`, `alarm.changed`, ...), `event`
    // its JSON as dspi_hub_server_events gives it ({kind, serial, record,
    // device, snapshot, change, alarm, run})
    void hubEvent(const QString &kind, const QJsonObject &event);

private:
    explicit HubFeed(Core *core);
    static void thunk(const char *kind, const char *eventJson, void *self);
    void listen();   // -- WP MDQT1 --

    Core *m_core;
    FfiCore *m_handle = nullptr;   // -- WP MDQT1 -- the core it listens to
};

#endif // HUBFEED_H
