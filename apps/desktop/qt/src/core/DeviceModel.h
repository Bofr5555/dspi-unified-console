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
    // writeFailed and the log. `done` (optional) runs on the GUI thread after
    // the snapshot is applied.
    void write(const QString &what, std::function<bool(FfiCore *)> setter,
               std::function<void(bool ok)> done = {});

    // Re-reads the device on the worker (dspi_fetch_all), then a snapshot.
    Q_INVOKABLE void refresh();
    Q_INVOKABLE void openDemo();
    Q_INVOKABLE void selectDevice(const QString &serial);
    // Closes and reopens the open device (the device button's right-click)
    Q_INVOKABLE void reconnect();
    // Wire channel `channel`'s name; "" restores the device's default. At most
    // 31 bytes of UTF-8; false, with nothing sent, for a longer name or no device.
    Q_INVOKABLE bool setChannelName(int channel, const QString &name);

signals:
    // After every applied snapshot; `flags` are the NOTIFY_* bits that caused
    // it (0 for a read or write of this app's own).
    void stateChanged(uint flags);
    void snapshotApplied();
    void writeFailed(const QString &what);
    void busyChanged();

private:
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
};

#endif // DEVICEMODEL_H
