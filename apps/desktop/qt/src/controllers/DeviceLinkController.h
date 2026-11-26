#ifndef DEVICELINKCONTROLLER_H
#define DEVICELINKCONTROLLER_H

#include <QHash>
#include <QList>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QStringList>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class DeviceSession;
class QJSEngine;
class QQmlEngine;

// -- WP UB34 -- (UXP B4, owner D6) Several DSPis as one system in daily use:
// the dock's "All devices" link. With two or more devices open on this
// computer (Modern), the person may link them, off by default and for this
// run only:
// - a User volume move on the shown device moves every linked device by as
//   much, keeping each one's offset from the shown device (taken when the
//   link goes on, the shown device changes or a device joins; a device held
//   at -60 or 0 dB gets its offset back when the volume comes back);
// - Mute mutes (and unmutes) them all;
// - a preset recall, from wherever it is asked (SidebarController's
//   requestPresetSwitch), recalls that slot on every linked device after
//   one question that names each device, its preset and the unsaved
//   changes the recall drops.
// -- FIX016 -- (REVG F2) Every User volume move follows the link (Home, the
// tray and Measure's refusal row go through SidebarController::setUserVolume,
// which hands a linked move here), and a device whose volume another way
// changed (another client, an agent, a remote) has its offset taken again
// from where it is, so the next linked move never jumps it.
// Each write is the device's own (the core's limits and the installer's
// lock apply as ever); the Master volume, the source and everything else
// stay per device. Hub zones stay the way for a whole installation.
class DeviceLinkController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // Two or more open devices can be written, in Modern: the chip shows
    Q_PROPERTY(bool available READ available NOTIFY changed)
    // The link is on (and available)
    Q_PROPERTY(bool linked READ linked WRITE setLinked NOTIFY changed)
    // The devices it acts on, the shown one first
    Q_PROPERTY(QStringList names READ names NOTIFY changed)
    // The chip's tooltip
    Q_PROPERTY(QString help READ help NOTIFY changed)
public:
    DeviceLinkController(const AppContext &ctx, QObject *parent = nullptr);
    static DeviceLinkController *create(QQmlEngine *, QJSEngine *);

    bool available() const;
    bool linked() const { return m_linked && available(); }
    void setLinked(bool on);
    QStringList names() const;
    QString help() const;

    // The shown device to `db` (User volume), the others by as much
    Q_INVOKABLE void setUserVolume(double db);
    // Every linked device muted, or unmuted
    Q_INVOKABLE void setMute(bool on);
    // From SidebarController::requestPresetSwitch: true when the link takes
    // the switch (one question, then every device)
    bool takePresetSwitch(int slot);
    // The question's answer: "accept" recalls
    Q_INVOKABLE void answerRecall(const QString &answer);

    // The offsets kept, by device id (dB from the shown device), for tests
    QHash<QString, double> offsets() const { return m_offsets; }

signals:
    void changed();
    // The one question of a recall on every device
    void recallQuestion(const QString &title, const QString &text);
    // Every recall asked has answered: how many went through, and which failed
    void recalled(int slot, int done, const QStringList &failed);

private:
    QList<DeviceSession *> targets() const;
    void forget();
    void recallNext();

    AppContext m_ctx;
    bool m_linked = false;
    QHash<QString, double> m_offsets;
    // -- FIX016 -- (REVG F2) by device id: the User volume the link last
    // asked for, and its writes not answered yet
    QHash<QString, float> m_asked;
    QHash<QString, int> m_inFlight;
    bool movedElsewhere(DeviceSession *s) const;
    void track(DeviceSession *s, float to);
    void untrack(const QString &id);
    int m_askedSlot = -1;
    QList<QPointer<DeviceSession>> m_recall;
    int m_recalled = 0;
    QStringList m_recallFailed;
};

#endif // DEVICELINKCONTROLLER_H
