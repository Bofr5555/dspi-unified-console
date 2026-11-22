#ifndef DEVICEMODEL_H
#define DEVICEMODEL_H

#include <QObject>
#include <QString>
#include <QtQml/qqmlregistration.h>
#include <functional>

#include "ChannelListModel.h"
#include "PresetModel.h"
#include "Snapshot.h"

class Core;
class DeviceRegistry;
class SessionModel;
class QQmlEngine;
class QJSEngine;

// The active device as the GUI sees it: the last snapshot of it, as properties
// and list models QML binds to. GUI thread only. -- WP MDQT1 -- A facade over the
// active session's SessionModel (DeviceRegistry), which holds the state: when
// another session becomes active, everything here is that one's, said as after a
// device switch (snapshotApplied, stateChanged(0)).
//
// Reads: every change of the device (connect, a notification, a write) ends in
// a new snapshot, then stateChanged(flags).
//
// Writes: one path. A controller calls write() with the dspi_* setter; the
// setter runs on the worker of the device that was active then, followed by a
// fresh snapshot; a refusal emits writeFailed(). QML never calls the core: it
// calls a controller's invokable, which validates and calls write(). `done` runs
// whether that device is still the active one or not.
class DeviceModel : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool connected READ connected NOTIFY snapshotApplied)
    Q_PROPERTY(bool demo READ demo NOTIFY snapshotApplied)
    Q_PROPERTY(bool readOnly READ readOnly NOTIFY snapshotApplied)
    Q_PROPERTY(QString serial READ serial NOTIFY snapshotApplied)
    Q_PROPERTY(QString deviceName READ deviceName NOTIFY snapshotApplied)
    Q_PROPERTY(QString deviceTitle READ deviceTitle NOTIFY snapshotApplied)
    Q_PROPERTY(QString platformName READ platformName NOTIFY snapshotApplied)
    Q_PROPERTY(QString firmwareVersion READ firmwareVersion NOTIFY snapshotApplied)
    Q_PROPERTY(int wireFormat READ wireFormat NOTIFY snapshotApplied)
    Q_PROPERTY(int compat READ compat NOTIFY snapshotApplied)
    Q_PROPERTY(int numInputs READ numInputs NOTIFY snapshotApplied)
    Q_PROPERTY(int numOutputs READ numOutputs NOTIFY snapshotApplied)
    Q_PROPERTY(int revision READ revision NOTIFY snapshotApplied)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)
    Q_PROPERTY(ChannelListModel *channels READ channels CONSTANT)
    Q_PROPERTY(PresetModel *presets READ presets CONSTANT)
    // -- WP QP13 -- The open device is a hub's, bound through DSPi Link
    Q_PROPERTY(bool remote READ remote NOTIFY snapshotApplied)
public:
    DeviceModel(Core *core, DeviceRegistry *registry, QObject *parent = nullptr);
    ~DeviceModel() override;

    // The one instance, made in main() before QML loads.
    static DeviceModel *instance();
    static DeviceModel *create(QQmlEngine *, QJSEngine *);

    // Opens the demo device, or the first DSPi found, then starts the hot-plug polls.
    void start(bool demo);

    bool connected() const;
    bool demo() const;
    bool readOnly() const;
    QString serial() const;
    QString deviceName() const;
    QString deviceTitle() const;
    QString platformName() const;
    QString firmwareVersion() const;
    int wireFormat() const;
    int compat() const;
    int numInputs() const;
    int numOutputs() const;
    int revision() const { return m_revision; }
    bool busy() const;
    ChannelListModel *channels() const { return m_channels; }
    PresetModel *presets() const { return m_presets; }

    // The last snapshot's state record; null when not connected. For
    // controllers reading fields no property exposes. GUI thread.
    const DspState *state() const;
    const DeviceSnapshot &snapshot() const;
    Core *core() const { return m_core; }
    // -- WP MDQT1 -- The active session's own
    SessionModel *active() const { return m_active; }

    // The write path (see the class comment). `what` names the write for
    // writeFailed and the log, and is its coalescing key: a write whose `what`
    // matches one still queued (not started) replaces that one's setter, latest
    // wins, so name it by its target ("output gain 3", not "output gain").
    // `done` (optional) runs on the GUI thread after the snapshot is applied,
    // for a replaced write too, with the outcome of the setter that ran.
    void write(const QString &what, std::function<bool(FfiCore *)> setter,
               std::function<void(bool ok)> done = {});

    // -- WP QTGAP1 -- A continuous drag (a slider or a graph handle held and
    // moved): the writes between beginDrag() and endDrag() join one undo step
    // per value (dspi_set_dragging, posted in order with them); every other
    // write is a step of its own. A write never replaces a queued one from
    // outside its own drag. endDrag() with no drag does nothing.
    Q_INVOKABLE void beginDrag();
    Q_INVOKABLE void endDrag();
    bool dragging() const;

    // Re-reads the device on the worker (dspi_fetch_all), then a snapshot.
    Q_INVOKABLE void refresh();
    Q_INVOKABLE void openDemo();
    Q_INVOKABLE void selectDevice(const QString &serial);
    // Closes and reopens the open device (the device button's right-click)
    Q_INVOKABLE void reconnect();
    // Wire channel `channel`'s name; "" restores the device's default. At most
    // 31 bytes of UTF-8; false, with nothing sent, for a longer name or no device.
    Q_INVOKABLE bool setChannelName(int channel, const QString &name);

    // -- WP QP13 -- A device bound through a DSPi Link session (LinkSession).
    // While one is, a scan, a hot-plug arrival and reconnect() leave the
    // connection alone: no local device opens under the session.
    bool remote() const;
    // -- WP P1QT1 -- After dspi_open_document: an offline document is the open
    // "device" (`serial` the core's OFFLINE-DOCUMENT). A reconnect re-reads it;
    // opening the demo, a DSPi or a hub's device leaves it.
    void adoptDocument(const QString &serial);
    bool document() const;

    // Setters the worker ran so far (the smoke driver's coalescing check)
    int settersRun() const;

    // -- WP QP16 -- write() for the few requests a read-only device still
    // takes: clearing the clip marks and entering the bootloader (behaviour.md
    // §14.2). Anything else goes through write().
    void writeEvenReadOnly(const QString &what, std::function<bool(FfiCore *)> setter,
                           std::function<void(bool ok)> done = {});

signals:
    // After every applied snapshot; `flags` are the NOTIFY_* bits that caused
    // it (0 for a read or write of this app's own).
    void stateChanged(uint flags);
    void snapshotApplied();
    void writeFailed(const QString &what);
    void busyChanged();
    // -- WP QP16 -- After each write the worker took up: whether it went
    // through and, if not, the core's reason (`code` and `message` as
    // dspi_take_failure gives them; "dropped" when another device had opened
    // since the write was asked for, so it was not sent)
    void writeFinished(const QString &what, bool ok, const QString &code, const QString &message);
    // -- WP P1QT2 -- A write the core refused, with the notice key of its
    // reason (P1NT's table: `limit_exceeded`, `installer_lock.locked`, ...),
    // so NoticeCenter can show it in the core's words
    void writeRefused(const QString &what, const QString &noticeKey, const QString &message);

private:
    void letGo();   // -- WP MDQT2a --
    void rebind();
    void onApplied(uint flags);

    Core *m_core;
    DeviceRegistry *m_registry;
    SessionModel *m_active = nullptr;
    ChannelListModel *m_channels;
    PresetModel *m_presets;
    int m_revision = 0;
};

#endif // DEVICEMODEL_H
