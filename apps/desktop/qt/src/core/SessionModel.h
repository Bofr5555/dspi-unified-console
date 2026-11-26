#ifndef SESSIONMODEL_H
#define SESSIONMODEL_H

#include <QObject>
#include <QString>
#include <QHash>
#include <atomic>
#include <functional>
#include <memory>

#include "Snapshot.h"

class DeviceSession;

// -- WP MDQT1 -- (DeviceModel until then) One open device as the GUI knows it:
// the last snapshot the session's worker read. GUI thread only. QML and the
// controllers see the active one's through DeviceModel.
//
// Reads: every change of the device (connect, a notification, a write) ends in
// a new snapshot, applied here, then applied(flags).
//
// Writes: one path. DeviceModel::write() lands here, in the session that was
// active when it was called: the setter runs on that session's worker, followed
// by a fresh snapshot; a refusal emits writeFailed().
class SessionModel : public QObject {
    Q_OBJECT
public:
    explicit SessionModel(DeviceSession *session);

    // Opens the demo device, or the first DSPi found.
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
    bool busy() const { return m_busy; }

    // The last snapshot's state record; null when not connected. For
    // controllers reading fields no property exposes. GUI thread.
    const DspState *state() const { return m_snap.state.get(); }
    const DeviceSnapshot &snapshot() const { return m_snap; }
    DeviceSession *session() const { return m_session; }

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
    void beginDrag();
    void endDrag();
    bool dragging() const { return m_drag != 0; }

    // Re-reads the device on the worker (dspi_fetch_all), then a snapshot.
    void refresh();
    // -- WP SHOWFIX -- the snapshot again, without reading the device: the
    // core's channel words follow a speaker layout that changed (no state did)
    void reread();
    void openDemo();
    // -- WP UB12 -- "Reset Demo": the demo's own settings again (dspi_reset_demo)
    void resetDemo();
    void selectDevice(const QString &serial);
    // Closes and reopens the open device (the device button's right-click)
    void reconnect();
    // Wire channel `channel`'s name; "" restores the device's default. At most
    // 31 bytes of UTF-8; false, with nothing sent, for a longer name or no device.
    bool setChannelName(int channel, const QString &name);

    // -- WP QP13 -- A device bound through a DSPi Link session (LinkSession).
    // While one is, a scan, a hot-plug arrival and reconnect() leave the
    // connection alone: no local device opens under the session.
    bool remote() const { return m_remote; }
    // After dspi_link_select_device: reads the device and shows it as `serial`.
    void adoptRemote(const QString &serial);
    // The session is gone: forget the hub's device and read what is left.
    void leaveRemote();

    // -- WP P1QT1 -- After dspi_open_document: an offline document is the open
    // "device" (`serial` the core's OFFLINE-DOCUMENT). A reconnect re-reads it;
    // opening the demo, a DSPi or a hub's device leaves it.
    void adoptDocument(const QString &serial);
    bool document() const { return m_document; }

    // Setters the worker ran so far (the smoke driver's coalescing check)
    int settersRun() const { return m_settersRun.load(); }

    // -- WP QP16 -- write() for the few requests a read-only device still
    // takes: clearing the clip marks and entering the bootloader (behaviour.md
    // §14.2). Anything else goes through write().
    void writeEvenReadOnly(const QString &what, std::function<bool(FfiCore *)> setter,
                           std::function<void(bool ok)> done = {});

    // -- WP MDQT1 -- The set opened a device of `kind` ("demo", "usb", "hub",
    // "document") on this session's core and read it
    void adoptOpened(const QString &kind, const QString &serial, const DeviceSnapshot &snap);
    // `close` runs on the worker and closes the device: nothing is open or
    // remembered afterwards
    void closed(std::function<void(FfiCore *)> close);

signals:
    // -- WP MDQT2c -- The open device is about to be replaced by another one on
    // this session's core (Classic); what is posted to the worker now comes first
    void replacing();
    // After every applied snapshot; `flags` are the NOTIFY_* bits that caused
    // it (0 for a read or write of this app's own).
    void applied(uint flags);
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
    struct PendingWrite;
    void apply(const DeviceSnapshot &snap, uint flags);
    void onNotifications();
    void onDeviceEvent(int event, const QString &serial);
    void track(int delta);

    DeviceSession *m_session;
    DeviceSnapshot m_snap;
    QString m_serial;
    bool m_demo = false;
    int m_inFlight = 0;
    bool m_busy = false;
    QHash<QString, std::shared_ptr<PendingWrite>> m_queuedWrites;
    std::atomic<int> m_settersRun{ 0 };
    bool m_remote = false;   // -- WP QP13 --
    bool m_document = false;   // -- WP P1QT1 --
    // -- WP QTGAP1 -- the drag under way (0: none), and the last one's number
    quint64 m_drag = 0;
    quint64 m_drags = 0;
    // -- WP QP16 --
    void post(const QString &what, std::function<bool(FfiCore *)> setter, std::function<void(bool)> done,
              bool evenReadOnly);
};

#endif // SESSIONMODEL_H
