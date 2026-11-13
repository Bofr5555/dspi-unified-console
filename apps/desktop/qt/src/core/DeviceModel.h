#ifndef DEVICEMODEL_H
#define DEVICEMODEL_H

#include <QObject>
#include <QString>
#include <QtQml/qqmlregistration.h>
#include <QHash>
#include <atomic>
#include <functional>
#include <memory>

#include "ChannelListModel.h"
#include "PresetModel.h"
#include "Snapshot.h"

class Core;
class QQmlEngine;
class QJSEngine;

// The device as the GUI sees it: the last snapshot the worker read, as
// properties and list models QML binds to. GUI thread only.
//
// Reads: every change of the device (connect, a notification, a write) ends in
// a new snapshot, applied here, then stateChanged(flags).
//
// Writes: one path. A controller calls write() with the dspi_* setter; the
// setter runs on the worker, followed by a fresh snapshot; a refusal emits
// writeFailed(). QML never calls the core: it calls a controller's invokable,
// which validates and calls write(). Undo, if it comes, hooks in here.
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
    explicit DeviceModel(Core *core, QObject *parent = nullptr);
    ~DeviceModel() override;

    // The one instance, made in main() before QML loads.
    static DeviceModel *instance();
    static DeviceModel *create(QQmlEngine *, QJSEngine *);

    // Opens the demo device, or the first DSPi found, then starts the hot-plug polls.
    void start(bool demo);

    bool connected() const { return m_snap.connected; }
    bool demo() const { return m_demo; }
    bool readOnly() const;
    QString serial() const { return m_serial; }
    QString deviceName() const { return m_snap.deviceName; }
    QString deviceTitle() const;
    QString platformName() const;
    QString firmwareVersion() const;
    int wireFormat() const;
    int compat() const;
    int numInputs() const;
    int numOutputs() const;
    int revision() const { return m_revision; }
    bool busy() const { return m_busy; }
    ChannelListModel *channels() const { return m_channels; }
    PresetModel *presets() const { return m_presets; }

    // The last snapshot's state record; null when not connected. For
    // controllers reading fields no property exposes. GUI thread.
    const DspState *state() const { return m_snap.state.get(); }
    const DeviceSnapshot &snapshot() const { return m_snap; }
    Core *core() const { return m_core; }

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
    bool dragging() const { return m_drag != 0; }

    // Re-reads the device on the worker (dspi_fetch_all), then a snapshot.
    Q_INVOKABLE void refresh();
    Q_INVOKABLE void openDemo();
    Q_INVOKABLE void selectDevice(const QString &serial);
    // Closes and reopens the open device (the device button's right-click)
    Q_INVOKABLE void reconnect();
    // Wire channel `channel`'s name; "" restores the device's default. At most
    // 31 bytes of UTF-8; false, with nothing sent, for a longer name or no device.
    Q_INVOKABLE bool setChannelName(int channel, const QString &name);

    // -- WP QP13 -- A device bound through a DSPi Link session (LinkController).
    // While one is, a scan, a hot-plug arrival and reconnect() leave the
    // connection alone: no local device opens under the session.
    bool remote() const { return m_remote; }
    // After dspi_link_select_device: reads the device and shows it as `serial`.
    void adoptRemote(const QString &serial);
    // The session is gone: forget the hub's device and read what is left.
    void leaveRemote();

    // Setters the worker ran so far (the smoke driver's coalescing check)
    int settersRun() const { return m_settersRun.load(); }

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

private:
    struct PendingWrite;
    void apply(const DeviceSnapshot &snap, uint flags);
    void onNotifications();
    void onDeviceEvent(int event, const QString &serial);
    void track(int delta);

    Core *m_core;
    ChannelListModel *m_channels;
    PresetModel *m_presets;
    DeviceSnapshot m_snap;
    QString m_serial;
    bool m_demo = false;
    int m_revision = 0;
    int m_inFlight = 0;
    bool m_busy = false;
    QHash<QString, std::shared_ptr<PendingWrite>> m_queuedWrites;
    std::atomic<int> m_settersRun{ 0 };
    bool m_remote = false;   // -- WP QP13 --
    // -- WP QTGAP1 -- the drag under way (0: none), and the last one's number
    quint64 m_drag = 0;
    quint64 m_drags = 0;
    // -- WP QP16 --
    void post(const QString &what, std::function<bool(FfiCore *)> setter, std::function<void(bool)> done,
              bool evenReadOnly);
};

#endif // DEVICEMODEL_H
